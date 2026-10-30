//! Transaction analysis: work out what a signing request does, from the bytes alone, and refuse
//! anything the device cannot account for.
//!
//! The rules, in the order they run:
//!
//! 1. The transaction decodes, is in canonical form, and stays within the size limits.
//! 2. Every input is the device's own: a mint v2 note whose nonce re-derives from its tweak, or an
//!    LNv2 refund whose refund key re-derives from the contract's ephemeral point. Peg-in claims,
//!    incoming-contract claims and preimage claims are never the device's and are refused.
//! 3. Every output is accounted for: change notes and staged notes re-derive to the device's
//!    trees; a Lightning contract, a peg-out; anything else is refused, including ecash issued to
//!    a third party.
//! 4. The outputs match the stated intent, and a Lightning contract pays exactly the invoice's
//!    payment hash, on the federation's network, with its refund coming back to the device.
//! 5. Inputs cover outputs plus the module fees the federation will charge, and the transaction
//!    does not overpay fees beyond rounding.

use std::collections::HashSet;

use bitcoin::secp256k1::PublicKey;
use fps_protocol::{Branch, InputHint, Intent, RejectCode, SignRequest};

use crate::encoding::Decode;
use crate::error::{SignerError, SignerResult};
use crate::federation::{short_hex, Federation, KnownModule};
use crate::fedimint::lnv2::{
    LightningInput, LightningOutput, OutgoingContract, OutgoingWitness, PaymentImage,
};
use crate::fedimint::mintv2::{denomination_amount_msat, MintInput, MintOutput};
use crate::fedimint::walletv2::WalletOutput;
use crate::fedimint::{OutPoint, UnsignedTransaction};
use crate::invoice::{parse_invoice, InvoiceDetails};
use crate::keys::FederationKeys;
use crate::review::{format_msat, Review};
use crate::tbs;

/// The device's memory of earlier decisions.
pub trait Ledger {
    /// Whether the outgoing-branch note at this position was released in a token.
    fn is_released(&self, federation: &[u8; 32], denomination: u8, tweak: &[u8; 16]) -> bool;

    /// The consensus-encoded contract this device approved at `outpoint`, if it remembers one.
    fn approved_contract(&self, federation: &[u8; 32], outpoint: &OutPoint) -> Option<Vec<u8>>;
}

/// A ledger that remembers nothing, for tests and first use.
pub struct EmptyLedger;

impl Ledger for EmptyLedger {
    fn is_released(&self, _: &[u8; 32], _: u8, _: &[u8; 16]) -> bool {
        false
    }

    fn approved_contract(&self, _: &[u8; 32], _: &OutPoint) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisOptions {
    /// Check each input note's mint signature. Defence in depth only: the federation rejects a
    /// forged note and the transaction is atomic. Costs one pairing per note on the device.
    pub verify_note_signatures: bool,
    pub max_inputs: usize,
    pub max_outputs: usize,
    /// Fee overpayment tolerated for rounding. Mint v2 clients issue notes from 2^9 msat, so a
    /// well-formed transaction overpays by less than 512 msat.
    pub max_overpay_msat: u64,
    /// The device clock, if it has a trustworthy one; used only for warnings.
    pub now_unix: Option<u64>,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            verify_note_signatures: true,
            max_inputs: 64,
            max_outputs: 64,
            max_overpay_msat: 1_000,
            now_unix: None,
        }
    }
}

/// Which key signs an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningKey {
    Note {
        branch: Branch,
        denomination: u8,
        tweak: [u8; 16],
    },
    Refund {
        ephemeral: PublicKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    Note {
        branch: Branch,
        denomination: u8,
        tweak: [u8; 16],
        /// An outgoing note whose key was already given away in a token.
        released: bool,
    },
    Refund {
        contract_id: [u8; 32],
        /// Cancelled early by the gateway rather than refunded after expiry.
        cancel: bool,
        /// The contract matches one this device approved at that outpoint. When `false` the
        /// amount comes from the phone and cannot be checked.
        verified: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSummary {
    pub kind: InputKind,
    pub amount_msat: u64,
    pub key: SigningKey,
    /// The key the federation will check the signature against.
    pub public_key: PublicKey,
}

// One per output of one request, so the size of the largest variant does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSummary {
    /// A new spend-branch note.
    Change {
        denomination: u8,
        tweak: [u8; 16],
        amount_msat: u64,
    },
    /// A new outgoing-branch note, staged for release as ecash.
    Staged {
        denomination: u8,
        tweak: [u8; 16],
        amount_msat: u64,
    },
    /// An LNv2 outgoing contract.
    Lightning {
        contract: OutgoingContract,
        amount_msat: u64,
    },
    /// A wallet v2 peg-out.
    Withdrawal {
        address: String,
        value_sat: u64,
        fee_sat: u64,
        amount_msat: u64,
    },
}

impl OutputSummary {
    pub fn amount_msat(&self) -> u64 {
        match self {
            Self::Change { amount_msat, .. }
            | Self::Staged { amount_msat, .. }
            | Self::Lightning { amount_msat, .. }
            | Self::Withdrawal { amount_msat, .. } => *amount_msat,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub inputs_msat: u64,
    pub outputs_msat: u64,
    /// The fees the federation's modules charge, computed from their fee rules.
    pub module_fees_msat: u64,
    /// Anything inputs cover beyond outputs and module fees. Also goes to the federation.
    pub overpay_msat: u64,
    pub change_msat: u64,
    pub staged_msat: u64,
    /// Value handed to someone else: Lightning contracts and peg-outs.
    pub external_msat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningPayment {
    pub invoice: InvoiceDetails,
    pub contract: OutgoingContract,
    /// Index of the contract output; with the txid, the outpoint a refund will spend.
    pub output_index: u64,
    /// The gateway's fee: contract amount minus invoice amount.
    pub routing_fee_msat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub federation: [u8; 32],
    pub txid: [u8; 32],
    pub intent: Intent,
    pub inputs: Vec<InputSummary>,
    pub outputs: Vec<OutputSummary>,
    pub totals: Totals,
    pub lightning: Option<LightningPayment>,
    pub review: Review,
}

impl Analysis {
    /// Value that leaves the wallet if the transaction is accepted: external outputs plus every
    /// fee. Staged notes stay the device's until released.
    pub fn outflow_msat(&self) -> u64 {
        self.totals
            .external_msat
            .saturating_add(self.totals.module_fees_msat)
            .saturating_add(self.totals.overpay_msat)
    }

    /// Positions of the notes staged on the outgoing branch.
    pub fn staged_notes(&self) -> Vec<(u8, [u8; 16])> {
        self.outputs
            .iter()
            .filter_map(|output| match output {
                OutputSummary::Staged {
                    denomination,
                    tweak,
                    ..
                } => Some((*denomination, *tweak)),
                _ => None,
            })
            .collect()
    }

    /// The outpoint of the Lightning contract this transaction funds.
    pub fn lightning_outpoint(&self) -> Option<OutPoint> {
        self.lightning.as_ref().map(|payment| OutPoint {
            txid: self.txid,
            out_idx: payment.output_index,
        })
    }
}

fn reject(code: RejectCode, message: impl Into<String>) -> SignerError {
    SignerError::new(code, message)
}

/// Analyses a signing request. Every refusal names the input or output it is about.
pub fn analyze(
    federation: &Federation,
    keys: &FederationKeys,
    request: &SignRequest,
    ledger: &dyn Ledger,
    options: &AnalysisOptions,
) -> SignerResult<Analysis> {
    if request.federation.0 != federation.id || keys.federation() != federation.id {
        return Err(reject(
            RejectCode::UnknownFederation,
            "the request is for a different federation",
        ));
    }
    let tx = UnsignedTransaction::decode_canonical(request.transaction.as_slice())
        .map_err(SignerError::invalid_transaction)?;
    if tx.inputs.is_empty() {
        return Err(reject(
            RejectCode::InvalidTransaction,
            "the transaction has no inputs",
        ));
    }
    if tx.inputs.len() > options.max_inputs || tx.outputs.len() > options.max_outputs {
        return Err(reject(
            RejectCode::TooLarge,
            format!(
                "{} inputs and {} outputs; the limits are {} and {}",
                tx.inputs.len(),
                tx.outputs.len(),
                options.max_inputs,
                options.max_outputs
            ),
        ));
    }
    if request.inputs.len() != tx.inputs.len() {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!(
                "{} input hints for {} inputs",
                request.inputs.len(),
                tx.inputs.len()
            ),
        ));
    }

    let mut inputs = Vec::with_capacity(tx.inputs.len());
    let mut seen_keys = HashSet::new();
    for (index, (item, hint)) in tx.inputs.iter().zip(&request.inputs).enumerate() {
        let input = match federation.known_module(item.instance) {
            Some(KnownModule::Mint) => note_input(
                federation,
                keys,
                ledger,
                options,
                index,
                &item.payload,
                hint,
            )?,
            Some(KnownModule::Lightning) => {
                refund_input(federation, keys, ledger, index, &item.payload, hint)?
            }
            Some(KnownModule::Wallet) => {
                return Err(reject(
                    RejectCode::ForeignInput,
                    format!("input {index} claims a peg-in; those are signed by the phone"),
                ))
            }
            None => {
                return Err(unsupported_module(
                    federation,
                    "input",
                    index,
                    item.instance,
                ))
            }
        };
        if !seen_keys.insert(input.public_key.serialize()) {
            return Err(reject(
                RejectCode::InvalidTransaction,
                format!("input {index} repeats a key already spent by another input"),
            ));
        }
        inputs.push(input);
    }

    let mut outputs = Vec::with_capacity(tx.outputs.len());
    let mut seen_positions = HashSet::new();
    for (index, item) in tx.outputs.iter().enumerate() {
        let output = match federation.known_module(item.instance) {
            Some(KnownModule::Mint) => note_output(federation, keys, index, &item.payload)?,
            Some(KnownModule::Lightning) => lightning_output(index, &item.payload)?,
            Some(KnownModule::Wallet) => wallet_output(federation, index, &item.payload)?,
            None => {
                return Err(unsupported_module(
                    federation,
                    "output",
                    index,
                    item.instance,
                ))
            }
        };
        if let OutputSummary::Change {
            denomination,
            tweak,
            ..
        }
        | OutputSummary::Staged {
            denomination,
            tweak,
            ..
        } = &output
        {
            if !seen_positions.insert((*denomination, *tweak)) {
                return Err(reject(
                    RejectCode::InvalidTransaction,
                    format!("output {index} repeats a note position used by another output"),
                ));
            }
        }
        outputs.push(output);
    }

    check_intent(&request.intent, &inputs, &outputs)?;
    let lightning = match &request.intent {
        Intent::PayInvoice { invoice } => {
            Some(check_lightning(federation, keys, invoice, &outputs)?)
        }
        _ => None,
    };
    let totals = compute_totals(federation, &inputs, &outputs, options)?;
    let review = build_review(
        federation,
        &request.intent,
        &inputs,
        &outputs,
        &totals,
        lightning.as_ref(),
        options,
    );

    Ok(Analysis {
        federation: federation.id,
        txid: tx.txid(),
        intent: request.intent.clone(),
        inputs,
        outputs,
        totals,
        lightning,
        review,
    })
}

fn unsupported_module(
    federation: &Federation,
    side: &str,
    index: usize,
    instance: u16,
) -> SignerError {
    let kind = federation
        .module_kind(instance)
        .unwrap_or("an unknown module");
    reject(
        RejectCode::Unsupported,
        format!("{side} {index} belongs to module instance {instance} ({kind}), which this version does not handle"),
    )
}

fn note_input(
    federation: &Federation,
    keys: &FederationKeys,
    ledger: &dyn Ledger,
    options: &AnalysisOptions,
    index: usize,
    payload: &[u8],
    hint: &InputHint,
) -> SignerResult<InputSummary> {
    let input =
        MintInput::decode_exact(payload, "mint input").map_err(SignerError::invalid_transaction)?;
    let InputHint::Note { tweak, branch } = hint else {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!("input {index} is a note, but its hint is not a note hint"),
        ));
    };
    let note = input.note;
    let denomination = note.denomination;
    let (Some(amount_msat), Some(mint_key)) = (
        denomination_amount_msat(denomination),
        federation.mint.keys.get(&denomination),
    ) else {
        return Err(reject(
            RejectCode::InvalidNote,
            format!("input {index}: this mint does not issue denomination 2^{denomination}"),
        ));
    };
    let ours = keys
        .note_nonce(*branch, denomination, &tweak.0)
        .is_ok_and(|expected| expected == note.nonce);
    if !ours {
        return Err(reject(
            RejectCode::ForeignInput,
            format!("input {index} is not a note of this device"),
        ));
    }
    if options.verify_note_signatures
        && !tbs::verify(&tbs::nonce_message(&note.nonce), &note.signature, mint_key)
    {
        return Err(reject(
            RejectCode::InvalidNote,
            format!("input {index}: the mint signature does not verify"),
        ));
    }
    let released =
        *branch == Branch::Outgoing && ledger.is_released(&federation.id, denomination, &tweak.0);
    Ok(InputSummary {
        kind: InputKind::Note {
            branch: *branch,
            denomination,
            tweak: tweak.0,
            released,
        },
        amount_msat,
        key: SigningKey::Note {
            branch: *branch,
            denomination,
            tweak: tweak.0,
        },
        public_key: note.nonce,
    })
}

fn refund_input(
    federation: &Federation,
    keys: &FederationKeys,
    ledger: &dyn Ledger,
    index: usize,
    payload: &[u8],
    hint: &InputHint,
) -> SignerResult<InputSummary> {
    let input = LightningInput::decode_exact(payload, "lightning input")
        .map_err(SignerError::invalid_transaction)?;
    let (outpoint, witness) = match input {
        LightningInput::Outgoing { outpoint, witness } => (outpoint, witness),
        LightningInput::Incoming(_) => {
            return Err(reject(
                RejectCode::ForeignInput,
                format!(
                    "input {index} claims an incoming payment; receives are signed by the phone"
                ),
            ))
        }
    };
    let cancel = match witness {
        OutgoingWitness::Refund => false,
        OutgoingWitness::Cancel(_) => true,
        OutgoingWitness::Claim(_) => {
            return Err(reject(
                RejectCode::ForeignInput,
                format!(
                    "input {index} claims a contract with its preimage, which is the gateway's key"
                ),
            ))
        }
    };
    let InputHint::Refund { contract: encoded } = hint else {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!("input {index} refunds a contract, but its hint is not a contract"),
        ));
    };
    let contract = OutgoingContract::decode_exact(encoded.as_slice(), "outgoing contract")
        .map_err(SignerError::invalid_transaction)?;
    let ours = keys
        .view()
        .refund_key(&contract.ephemeral_pk)
        .is_ok_and(|expected| expected == contract.refund_pk);
    if !ours {
        return Err(reject(
            RejectCode::ForeignInput,
            format!("input {index}: the contract's refund key is not this device's"),
        ));
    }
    // The outpoint cannot be checked offline, but the device remembers the contracts it funded.
    // A phone that pairs this input with a different, cheaper contract of ours would make the
    // federation take the difference as fees, so a remembered contract must match exactly.
    let verified = match ledger.approved_contract(&federation.id, &outpoint) {
        Some(recorded) if recorded == encoded.as_slice() => true,
        Some(_) => {
            return Err(reject(
                RejectCode::ForeignInput,
                format!("input {index}: the contract differs from the one this device funded at that outpoint"),
            ))
        }
        None => false,
    };
    Ok(InputSummary {
        kind: InputKind::Refund {
            contract_id: contract.contract_id(),
            cancel,
            verified,
        },
        amount_msat: contract.amount_msat,
        key: SigningKey::Refund {
            ephemeral: contract.ephemeral_pk,
        },
        public_key: contract.refund_pk,
    })
}

fn note_output(
    federation: &Federation,
    keys: &FederationKeys,
    index: usize,
    payload: &[u8],
) -> SignerResult<OutputSummary> {
    let output = MintOutput::decode_exact(payload, "mint output")
        .map_err(SignerError::invalid_transaction)?;
    let denomination = output.denomination;
    let amount_msat = denomination_amount_msat(denomination)
        .filter(|_| federation.mint.keys.contains_key(&denomination))
        .ok_or_else(|| {
            reject(
                RejectCode::UnexpectedOutput,
                format!("output {index}: this mint does not issue denomination 2^{denomination}"),
            )
        })?;
    let view = keys.view();
    for branch in [Branch::Spend, Branch::Outgoing] {
        let Ok(nonce) = keys.note_nonce(branch, denomination, &output.tweak) else {
            continue;
        };
        if view.blinded_nonce(branch, &nonce, denomination, &output.tweak) != output.nonce {
            continue;
        }
        if !view.tweak_passes_filter(&output.tweak) {
            return Err(reject(
                RejectCode::UnexpectedOutput,
                format!("output {index}: its tweak is not ground for this wallet, so recovery would miss the note"),
            ));
        }
        let (denomination, tweak) = (denomination, output.tweak);
        return Ok(match branch {
            Branch::Spend => OutputSummary::Change {
                denomination,
                tweak,
                amount_msat,
            },
            Branch::Outgoing => OutputSummary::Staged {
                denomination,
                tweak,
                amount_msat,
            },
        });
    }
    Err(reject(
        RejectCode::UnexpectedOutput,
        format!("output {index} issues ecash to someone other than this device"),
    ))
}

fn lightning_output(index: usize, payload: &[u8]) -> SignerResult<OutputSummary> {
    match LightningOutput::decode_exact(payload, "lightning output")
        .map_err(SignerError::invalid_transaction)?
    {
        LightningOutput::Outgoing(contract) => Ok(OutputSummary::Lightning {
            amount_msat: contract.amount_msat,
            contract,
        }),
        LightningOutput::Incoming(_) => Err(reject(
            RejectCode::UnexpectedOutput,
            format!("output {index} funds an incoming contract, which only gateways do"),
        )),
    }
}

fn wallet_output(
    federation: &Federation,
    index: usize,
    payload: &[u8],
) -> SignerResult<OutputSummary> {
    let wallet = federation
        .wallet
        .as_ref()
        .expect("known_module returned Wallet, so the wallet module exists");
    let output = WalletOutput::decode_exact(payload, "wallet output")
        .map_err(SignerError::invalid_transaction)?;
    let address = output.destination.address(wallet.network).ok_or_else(|| {
        reject(
            RejectCode::UnexpectedOutput,
            format!("output {index}: the peg-out destination has no address on this network"),
        )
    })?;
    let amount_msat = output.amount_msat().ok_or_else(|| {
        reject(
            RejectCode::InvalidTransaction,
            format!("output {index}: the peg-out amount overflows"),
        )
    })?;
    Ok(OutputSummary::Withdrawal {
        address,
        value_sat: output.value_sat,
        fee_sat: output.fee_sat,
        amount_msat,
    })
}

fn check_intent(
    intent: &Intent,
    inputs: &[InputSummary],
    outputs: &[OutputSummary],
) -> SignerResult<()> {
    let count =
        |predicate: fn(&OutputSummary) -> bool| outputs.iter().filter(|o| predicate(o)).count();
    let lightning = count(|o| matches!(o, OutputSummary::Lightning { .. }));
    let withdrawals = count(|o| matches!(o, OutputSummary::Withdrawal { .. }));
    let staged = count(|o| matches!(o, OutputSummary::Staged { .. }));
    let refunds = inputs
        .iter()
        .filter(|i| matches!(i.kind, InputKind::Refund { .. }))
        .count();
    let released = inputs
        .iter()
        .filter(|i| matches!(i.kind, InputKind::Note { released: true, .. }))
        .count();

    let mismatch = |message: String| Err(reject(RejectCode::IntentMismatch, message));
    if released > 0 && *intent != Intent::Reclaim {
        return Err(reject(
            RejectCode::AlreadyReleased,
            "an input spends a note already released in a token; only a reclaim may do that",
        ));
    }
    match intent {
        Intent::PayInvoice { .. } => {
            if lightning != 1 || withdrawals + staged + refunds > 0 {
                return mismatch(format!(
                    "paying an invoice needs exactly one Lightning contract and only change besides; found {lightning} contracts, {withdrawals} peg-outs, {staged} staged notes, {refunds} refunds"
                ));
            }
        }
        Intent::Withdraw => {
            if withdrawals != 1 || lightning + staged + refunds > 0 {
                return mismatch(format!(
                    "a withdrawal needs exactly one peg-out and only change besides; found {withdrawals} peg-outs, {lightning} contracts, {staged} staged notes, {refunds} refunds"
                ));
            }
        }
        Intent::PrepareSend { amount_msat } => {
            let staged_total: u64 = outputs
                .iter()
                .filter(|o| matches!(o, OutputSummary::Staged { .. }))
                .map(OutputSummary::amount_msat)
                .sum();
            if staged == 0 || lightning + withdrawals + refunds > 0 {
                return mismatch(
                    "preparing a send needs staged notes and only change besides".to_owned(),
                );
            }
            if staged_total != *amount_msat {
                return mismatch(format!(
                    "staged notes total {} but the send is for {}",
                    format_msat(staged_total),
                    format_msat(*amount_msat)
                ));
            }
        }
        Intent::Reissue => {
            if lightning + withdrawals + staged + refunds > 0 {
                return mismatch("a reissue may only produce change".to_owned());
            }
        }
        Intent::Reclaim => {
            if released == 0 || lightning + withdrawals + staged + refunds > 0 {
                return mismatch(
                    "a reclaim spends released notes and may only produce change".to_owned(),
                );
            }
        }
        Intent::Refund => {
            if refunds == 0 || lightning + withdrawals + staged > 0 {
                return mismatch(
                    "a refund spends a Lightning contract and may only produce change".to_owned(),
                );
            }
        }
    }
    Ok(())
}

fn check_lightning(
    federation: &Federation,
    keys: &FederationKeys,
    invoice: &str,
    outputs: &[OutputSummary],
) -> SignerResult<LightningPayment> {
    let module = federation
        .lightning
        .as_ref()
        .ok_or_else(|| reject(RejectCode::Unsupported, "the federation has no LNv2 module"))?;
    let (output_index, contract) = outputs
        .iter()
        .enumerate()
        .find_map(|(index, output)| match output {
            OutputSummary::Lightning { contract, .. } => Some((index as u64, *contract)),
            _ => None,
        })
        .expect("check_intent found exactly one Lightning output");
    let invoice = parse_invoice(invoice)?;
    let mismatch = |message: &str| Err(reject(RejectCode::InvoiceMismatch, message));
    if !invoice.matches_network(module.network) {
        return mismatch("the invoice is for a different network than the federation");
    }
    let Some(invoice_amount) = invoice.amount_msat else {
        return mismatch("invoices without an amount are not supported");
    };
    if contract.payment_image != PaymentImage::Hash(invoice.payment_hash) {
        return mismatch("the contract does not pay this invoice's payment hash");
    }
    if contract.amount_msat < invoice_amount {
        return mismatch("the contract pays less than the invoice amount");
    }
    let refunds_here = keys
        .view()
        .refund_key(&contract.ephemeral_pk)
        .is_ok_and(|expected| expected == contract.refund_pk);
    if !refunds_here {
        return Err(reject(
            RejectCode::UnexpectedOutput,
            "the contract's refund key is not this device's, so a failed payment could not come back",
        ));
    }
    Ok(LightningPayment {
        routing_fee_msat: contract.amount_msat - invoice_amount,
        invoice,
        contract,
        output_index,
    })
}

fn compute_totals(
    federation: &Federation,
    inputs: &[InputSummary],
    outputs: &[OutputSummary],
    options: &AnalysisOptions,
) -> SignerResult<Totals> {
    let overflow = || reject(RejectCode::InvalidTransaction, "amounts overflow");
    let lightning_fee = federation.lightning.as_ref().map(|m| m.fee);
    let wallet_fee = federation.wallet.as_ref().map(|m| m.fee);
    let mut totals = Totals::default();
    let mut fees = 0u64;

    for input in inputs {
        totals.inputs_msat = totals
            .inputs_msat
            .checked_add(input.amount_msat)
            .ok_or_else(overflow)?;
        let fee = match input.kind {
            InputKind::Note { .. } => federation.mint.fee.fee_msat(input.amount_msat),
            InputKind::Refund { .. } => lightning_fee
                .expect("refund inputs only decode with an LNv2 module")
                .fee_msat(input.amount_msat),
        };
        fees = fees.checked_add(fee).ok_or_else(overflow)?;
    }
    for output in outputs {
        let amount = output.amount_msat();
        totals.outputs_msat = totals
            .outputs_msat
            .checked_add(amount)
            .ok_or_else(overflow)?;
        let fee = match output {
            OutputSummary::Change { .. } => {
                totals.change_msat += amount;
                federation.mint.fee.fee_msat(amount)
            }
            OutputSummary::Staged { .. } => {
                totals.staged_msat += amount;
                federation.mint.fee.fee_msat(amount)
            }
            OutputSummary::Lightning { .. } => {
                totals.external_msat += amount;
                lightning_fee
                    .expect("lightning outputs only decode with an LNv2 module")
                    .fee_msat(amount)
            }
            OutputSummary::Withdrawal { .. } => {
                totals.external_msat += amount;
                wallet_fee
                    .expect("peg-outs only decode with a wallet module")
                    .fee_msat(amount)
            }
        };
        fees = fees.checked_add(fee).ok_or_else(overflow)?;
    }
    totals.module_fees_msat = fees;
    let required = totals.outputs_msat.checked_add(fees).ok_or_else(overflow)?;
    if totals.inputs_msat < required {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!(
                "inputs of {} do not cover outputs of {} plus fees of {}",
                format_msat(totals.inputs_msat),
                format_msat(totals.outputs_msat),
                format_msat(fees)
            ),
        ));
    }
    totals.overpay_msat = totals.inputs_msat - required;
    if totals.overpay_msat > options.max_overpay_msat {
        return Err(reject(
            RejectCode::PolicyLimit,
            format!(
                "the transaction overpays fees by {}; at most {} is accepted for rounding",
                format_msat(totals.overpay_msat),
                format_msat(options.max_overpay_msat)
            ),
        ));
    }
    Ok(totals)
}

fn build_review(
    federation: &Federation,
    intent: &Intent,
    inputs: &[InputSummary],
    outputs: &[OutputSummary],
    totals: &Totals,
    lightning: Option<&LightningPayment>,
    options: &AnalysisOptions,
) -> Review {
    let federation_fee = totals.module_fees_msat + totals.overpay_msat;
    let mut review = match intent {
        Intent::PayInvoice { .. } => {
            let payment = lightning.expect("pay-invoice analysis always has a payment");
            let invoice = &payment.invoice;
            let amount = invoice.amount_msat.unwrap_or(payment.contract.amount_msat);
            let mut review = Review::new("Pay Lightning invoice", amount)
                .line("Amount", format_msat(amount))
                .line("To node", short_hex(&invoice.payee.serialize()));
            if let Some(description) = invoice.description.as_ref().filter(|d| !d.is_empty()) {
                review = review.line("Description", description.clone());
            }
            review = review.line("Gateway fee", format_msat(payment.routing_fee_msat));
            if let Some(now) = options.now_unix {
                if now > invoice.expires_at_unix() {
                    review = review
                        .warn("This invoice has expired; the payment will fail and be refunded.");
                }
            }
            review
        }
        Intent::Withdraw => {
            let (address, value_sat, fee_sat) = outputs
                .iter()
                .find_map(|output| match output {
                    OutputSummary::Withdrawal {
                        address,
                        value_sat,
                        fee_sat,
                        ..
                    } => Some((address.clone(), *value_sat, *fee_sat)),
                    _ => None,
                })
                .expect("withdraw analysis always has a peg-out");
            Review::new("Withdraw on chain", value_sat * 1000)
                .line("Amount", format_msat(value_sat * 1000))
                .line("To address", address)
                .line("On-chain fee", format_msat(fee_sat * 1000))
        }
        Intent::PrepareSend { amount_msat } => Review::new("Prepare ecash to send", *amount_msat)
            .line("Amount", format_msat(*amount_msat))
            .line("Notes staged", totals_count(outputs, true).to_string())
            .warn("Nothing leaves your wallet yet. Releasing the ecash needs this device."),
        Intent::Reissue => Review::new("Reorganise notes", totals.change_msat)
            .line("Notes in", inputs.len().to_string())
            .line("Notes out", totals_count(outputs, false).to_string()),
        Intent::Reclaim => {
            let reclaimed: u64 = inputs
                .iter()
                .filter(|i| matches!(i.kind, InputKind::Note { released: true, .. }))
                .map(|i| i.amount_msat)
                .sum();
            Review::new("Reclaim unredeemed ecash", reclaimed)
                .line("Amount", format_msat(reclaimed))
                .warn("If the receiver already redeemed these notes, the federation will refuse this.")
        }
        Intent::Refund => {
            let refunded: u64 = inputs
                .iter()
                .filter(|i| matches!(i.kind, InputKind::Refund { .. }))
                .map(|i| i.amount_msat)
                .sum();
            let mut review = Review::new("Refund failed payment", refunded)
                .line("Amount", format_msat(refunded));
            if inputs.iter().any(|i| {
                matches!(
                    i.kind,
                    InputKind::Refund {
                        verified: false,
                        ..
                    }
                )
            }) {
                review = review.warn(
                    "This device has no record of funding this payment, so the amount comes from your phone.",
                );
            }
            review
        }
    };
    review = review
        .line("Federation fee", format_msat(federation_fee))
        .line("Change back", format_msat(totals.change_msat))
        .line("Federation", federation.display_name());
    review
}

fn totals_count(outputs: &[OutputSummary], staged: bool) -> usize {
    outputs
        .iter()
        .filter(|output| match output {
            OutputSummary::Staged { .. } => staged,
            OutputSummary::Change { .. } => !staged,
            _ => false,
        })
        .count()
}
