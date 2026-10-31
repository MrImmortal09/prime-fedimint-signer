//! Spend analysis: work out what a Cashu swap or melt does, from the request alone, and refuse
//! anything Prime cannot account for (`docs/plan/12_CASHU.md`, "What Prime checks before it
//! signs").
//!
//! The rules, in the order they run:
//!
//! 1. The request is for this mint, with 1 to 64 inputs and 0 to 64 outputs.
//! 2. Every input is this wallet's: its secret is the template locked to `P_e` for the request's
//!    epoch `e`, which Prime derives itself. Its keyset is pinned and has a key for its amount, a
//!    power of two, and `C` is a point. A DLEQ proof, when present, verifies against that key; a
//!    melt needs one on every input, because its signature does not commit to input amounts.
//! 3. Every output opens: Prime recomputes `B_ = hash_to_curve(secret) + r·G`. Its keyset is pinned
//!    and active and has a key for its amount, a power of two; a melt's outputs are blanks of
//!    amount 0. An output whose opening carries a hint is change: it must be the `fps-cashu-v1`
//!    output at that position, at the input epoch or up to three above it, so a restore finds it.
//!    Every other output is leaving. No secret appears twice among inputs and outputs, since a
//!    proof whose secret was spent can never be spent again.
//! 4. The input fee is `ceil(Σ input_fee_ppk / 1000)` over the inputs' keysets. A swap's inputs
//!    minus the fee equal its outputs. A melt's invoice parses and carries an amount, the quote's
//!    amount is that amount in sats rounded down or up, the inputs cover the amount, the fee
//!    reserve and the fee, and every output is a blank of this wallet's, enough of them to return
//!    everything the mint does not spend.
//! 5. Prime builds the NUT-11 message itself and refuses it unless its outputs read only one way.
//!
//! What leaves: a swap's leaving outputs plus the fee, which is the fee alone when every output is
//! change; for a melt, every input, because Prime cannot see what the quote pays.

use std::collections::BTreeSet;

use bitcoin::secp256k1::{PublicKey, SecretKey};
use fps_protocol::{CashuOutput, CashuOutputHint, CashuSignRequest, CashuSpendKind, RejectCode};
use fps_signer_core::error::{SignerError, SignerResult};
use fps_signer_core::invoice::{parse_invoice, InvoiceDetails};
use fps_signer_core::review::Review;

use crate::keys::MintKeys;
use crate::keyset::{short_id, PinnedKeyset, PinnedMint};
use crate::message::{self, SigAllMessage};
use crate::{dhke, dleq, hex, secret};

/// Most inputs one request may spend.
pub const MAX_INPUTS: usize = 64;
/// Most outputs one request may create.
pub const MAX_OUTPUTS: usize = 64;
/// How many epochs above the input epoch change may be locked to. A restore therefore scans epochs
/// until `MAX_EPOCHS_AHEAD + 1` in a row are empty: then no later epoch can hold change.
pub const MAX_EPOCHS_AHEAD: u32 = 3;
/// Longest output secret accepted, in bytes: the longest a CDK mint accepts in a proof it spends.
pub const MAX_SECRET_LEN: usize = 1024;
/// Longest melt quote id accepted, in bytes. Mints issue UUIDs or about 40 random characters.
pub const MAX_QUOTE_LEN: usize = 128;

/// What a spend is, as the review names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendKind {
    /// A swap with outputs leaving the wallet.
    Send,
    /// A swap into change at the input epoch only.
    Combine,
    /// A swap into change, some of it at a later epoch.
    Rotate,
    /// A melt.
    Melt,
}

impl SpendKind {
    /// The review's title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Send => "Cashu: send ecash",
            Self::Combine => "Cashu: combine ecash",
            Self::Rotate => "Cashu: move to a new key",
            Self::Melt => "Cashu: pay Lightning invoice",
        }
    }
}

/// Sums of a spend, in sats.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub inputs_sat: u64,
    /// The input fee the mint charges.
    pub fee_sat: u64,
    pub outputs_sat: u64,
    /// Outputs that are this wallet's change. A melt's blanks count 0 here.
    pub change_sat: u64,
    /// Outputs leaving the wallet.
    pub leaving_sat: u64,
}

/// A checked spend, ready for approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashuAnalysis {
    pub kind: SpendKind,
    /// The input epoch, whose lock key signs.
    pub epoch: u32,
    /// The NUT-11 message Prime built.
    pub message: String,
    /// Its SHA-256, which the signature covers.
    pub message_hash: [u8; 32],
    pub totals: Totals,
    /// What leaves the wallet if the mint accepts the request, in msat, for the policy.
    pub outflow_msat: u64,
    pub review: Review,
}

/// One output, as the analysis classified it.
struct OutputSummary {
    amount: u64,
    /// The lock epoch of change; `None` for an output leaving the wallet.
    change_epoch: Option<u32>,
}

fn reject(code: RejectCode, message: impl Into<String>) -> SignerError {
    SignerError::new(code, message)
}

fn overflow() -> SignerError {
    reject(RejectCode::InvalidTransaction, "amounts overflow")
}

fn add(total: &mut u64, amount: u64) -> SignerResult<()> {
    *total = total.checked_add(amount).ok_or_else(overflow)?;
    Ok(())
}

fn msat(sats: u64) -> SignerResult<u64> {
    sats.checked_mul(1000).ok_or_else(overflow)
}

/// Analyses a swap or melt. Every refusal names the input or output it is about.
pub fn analyze(
    request: &CashuSignRequest,
    mint: &PinnedMint,
    keys: &MintKeys,
) -> SignerResult<CashuAnalysis> {
    if request.mint.0 != mint.id || keys.mint_id() != mint.id {
        return Err(reject(
            RejectCode::UnknownFederation,
            "the request is for a different mint",
        ));
    }
    if request.inputs.is_empty() {
        return Err(reject(
            RejectCode::InvalidTransaction,
            "the request has no inputs",
        ));
    }
    if request.inputs.len() > MAX_INPUTS || request.outputs.len() > MAX_OUTPUTS {
        return Err(reject(
            RejectCode::TooLarge,
            format!(
                "{} inputs and {} outputs; the limits are {MAX_INPUTS} and {MAX_OUTPUTS}",
                request.inputs.len(),
                request.outputs.len()
            ),
        ));
    }
    let melt = matches!(request.kind, CashuSpendKind::Melt { .. });
    let epoch = request.epoch;
    let lock_key = keys.lock_pubkey(epoch).map_err(|error| {
        reject(
            RejectCode::Internal,
            format!("epoch {epoch} has no lock key: {error}"),
        )
    })?;

    let mut totals = Totals::default();
    let mut fee_ppk = 0u64;
    let mut secrets = BTreeSet::new();
    for (index, input) in request.inputs.iter().enumerate() {
        let ours =
            secret::parse(&input.secret).is_some_and(|template| template.lock_key == lock_key);
        if !ours {
            return Err(reject(
                RejectCode::ForeignInput,
                format!("input {index} is not locked to this wallet's key for epoch {epoch}"),
            ));
        }
        if !secrets.insert(input.secret.as_str()) {
            return Err(reject(
                RejectCode::InvalidTransaction,
                format!("input {index} repeats a proof another input spends"),
            ));
        }
        let keyset = mint.keyset(input.keyset.as_slice()).ok_or_else(|| {
            reject(
                RejectCode::InvalidNote,
                format!(
                    "input {index}: keyset {} is not pinned for this mint; update the mint keys first",
                    short_id(input.keyset.as_slice())
                ),
            )
        })?;
        let mint_key = keyset.key(input.amount).ok_or_else(|| {
            reject(
                RejectCode::InvalidNote,
                format!(
                    "input {index}: keyset {} has no key for {}",
                    short_id(&keyset.id),
                    input.amount
                ),
            )
        })?;
        let c = PublicKey::from_slice(&input.c.0).map_err(|_| {
            reject(
                RejectCode::InvalidNote,
                format!("input {index}: its signature C is not a point"),
            )
        })?;
        match &input.dleq {
            Some(proof) => {
                if !dleq::verify_proof(input.secret.as_bytes(), &c, proof, mint_key) {
                    return Err(reject(
                        RejectCode::InvalidNote,
                        format!(
                            "input {index}: its DLEQ proof does not verify against the pinned key for {}",
                            input.amount
                        ),
                    ));
                }
            }
            None if melt => {
                return Err(reject(
                    RejectCode::InvalidNote,
                    format!("input {index}: a melt needs a DLEQ proof on every input"),
                ))
            }
            None => {}
        }
        add(&mut totals.inputs_sat, input.amount)?;
        add(&mut fee_ppk, keyset.input_fee_ppk)?;
    }
    // Every other sum is at most this one, so it converts too.
    msat(totals.inputs_sat)?;
    totals.fee_sat = fee_ppk.div_ceil(1000);

    let mut lock_keys = LockKeys::new(keys, epoch);
    let mut blinded = BTreeSet::new();
    let mut outputs = Vec::with_capacity(request.outputs.len());
    for (index, output) in request.outputs.iter().enumerate() {
        let summary = check_output(mint, &mut lock_keys, melt, index, output)?;
        if !secrets.insert(output.opening.secret.as_str()) {
            return Err(reject(
                RejectCode::UnexpectedOutput,
                format!("output {index} reuses the secret of another input or output, so its proof could never be spent"),
            ));
        }
        if !blinded.insert(output.b.0) {
            return Err(reject(
                RejectCode::InvalidTransaction,
                format!("output {index} repeats a blinded message"),
            ));
        }
        add(&mut totals.outputs_sat, summary.amount)?;
        match summary.change_epoch {
            Some(_) => add(&mut totals.change_sat, summary.amount)?,
            None => add(&mut totals.leaving_sat, summary.amount)?,
        }
        outputs.push(summary);
    }

    let (kind, outflow_sat, review, quote) = match &request.kind {
        CashuSpendKind::Swap => {
            let (kind, outflow) = check_swap(&totals, &outputs, epoch)?;
            let review = swap_review(mint, kind, &totals, request, &outputs)?;
            (kind, outflow, review, None)
        }
        CashuSpendKind::Melt {
            quote,
            invoice,
            amount,
            fee_reserve,
        } => {
            let invoice = check_melt(
                &totals,
                outputs.len(),
                quote,
                invoice,
                *amount,
                *fee_reserve,
            )?;
            let review = melt_review(mint, &totals, &invoice, *amount, *fee_reserve)?;
            (
                SpendKind::Melt,
                totals.inputs_sat,
                review,
                Some(quote.as_str()),
            )
        }
    };

    let message_inputs: Vec<(&str, &[u8; 33])> = request
        .inputs
        .iter()
        .map(|input| (input.secret.as_str(), &input.c.0))
        .collect();
    let message_outputs: Vec<(u64, &[u8; 33])> = request
        .outputs
        .iter()
        .map(|output| (output.amount, &output.b.0))
        .collect();
    let message = SigAllMessage::build(&message_inputs, &message_outputs, quote);
    message::check_one_reading(message.output_part())?;
    Ok(CashuAnalysis {
        kind,
        epoch,
        message_hash: message.hash(),
        message: message.into_string(),
        totals,
        outflow_msat: msat(outflow_sat)?,
        review,
    })
}

/// The lock keys of the change epochs, each derived once: at most [`MAX_EPOCHS_AHEAD`] + 1.
struct LockKeys<'a> {
    keys: &'a MintKeys,
    epoch: u32,
    cached: Vec<(u32, Option<PublicKey>)>,
}

impl<'a> LockKeys<'a> {
    fn new(keys: &'a MintKeys, epoch: u32) -> Self {
        Self {
            keys,
            epoch,
            cached: Vec::new(),
        }
    }

    /// `P_e'` for a change epoch `e'`, which the caller has checked is in range.
    fn get(&mut self, epoch: u32) -> Option<PublicKey> {
        if let Some((_, key)) = self.cached.iter().find(|(cached, _)| *cached == epoch) {
            return *key;
        }
        let key = self.keys.lock_pubkey(epoch).ok();
        self.cached.push((epoch, key));
        key
    }
}

fn check_output(
    mint: &PinnedMint,
    lock_keys: &mut LockKeys,
    melt: bool,
    index: usize,
    output: &CashuOutput,
) -> SignerResult<OutputSummary> {
    let unexpected = |message: String| reject(RejectCode::UnexpectedOutput, message);
    let keyset = mint.keyset(output.keyset.as_slice()).ok_or_else(|| {
        unexpected(format!(
            "output {index}: keyset {} is not pinned for this mint",
            short_id(output.keyset.as_slice())
        ))
    })?;
    if !keyset.active {
        return Err(unexpected(format!(
            "output {index}: keyset {} is inactive, and the mint signs only with active keysets",
            short_id(&keyset.id)
        )));
    }
    if melt {
        if output.amount != 0 {
            return Err(unexpected(format!(
                "output {index}: a melt's outputs are blanks of amount 0, not {}",
                output.amount
            )));
        }
    } else if keyset.key(output.amount).is_none() {
        return Err(unexpected(format!(
            "output {index}: keyset {} has no key for {}",
            short_id(&keyset.id),
            output.amount
        )));
    }

    let opening = &output.opening;
    if opening.secret.len() > MAX_SECRET_LEN {
        return Err(unexpected(format!(
            "output {index}: its secret is longer than {MAX_SECRET_LEN} bytes"
        )));
    }
    let r = SecretKey::from_slice(&opening.r.0).map_err(|_| {
        unexpected(format!(
            "output {index}: its blinding factor is not a valid scalar"
        ))
    })?;
    let opens =
        dhke::blind(opening.secret.as_bytes(), &r).is_some_and(|b| b.serialize() == output.b.0);
    if !opens {
        return Err(unexpected(format!(
            "output {index}: B_ is not hash_to_curve(secret) + r·G for its opening"
        )));
    }

    let change_epoch = match &opening.hint {
        Some(hint) => Some(check_change(lock_keys, keyset, index, output, hint)?),
        None => None,
    };
    if melt && change_epoch.is_none() {
        return Err(unexpected(format!(
            "output {index}: every blank of a melt must be this wallet's change, with a hint"
        )));
    }
    Ok(OutputSummary {
        amount: output.amount,
        change_epoch,
    })
}

/// Checks a hinted output is exactly the `fps-cashu-v1` output at the hint's position. A wrong
/// hint is refused rather than taken as leaving, so the phone cannot mislabel an output.
fn check_change(
    lock_keys: &mut LockKeys,
    keyset: &PinnedKeyset,
    index: usize,
    output: &CashuOutput,
    hint: &CashuOutputHint,
) -> SignerResult<u32> {
    let unexpected = |message: String| reject(RejectCode::UnexpectedOutput, message);
    let input_epoch = lock_keys.epoch;
    let in_range = hint
        .epoch
        .checked_sub(input_epoch)
        .is_some_and(|ahead| ahead <= MAX_EPOCHS_AHEAD);
    if !in_range {
        return Err(unexpected(format!(
            "output {index}: change at epoch {} is outside epochs {input_epoch} to {input_epoch} + {MAX_EPOCHS_AHEAD}, where a restore finds it",
            hint.epoch
        )));
    }
    if hint.keyset != output.keyset {
        return Err(unexpected(format!(
            "output {index}: its hint names keyset {}, not its own keyset {}",
            short_id(hint.keyset.as_slice()),
            short_id(&keyset.id)
        )));
    }
    let lock_key = lock_keys.get(hint.epoch);
    let derived = lock_keys
        .keys
        .phone()
        .output_parts(hint.epoch, hint.keyset.as_slice(), hint.counter)
        .ok();
    let matches = match (lock_key, derived) {
        (Some(lock_key), Some((nonce, r))) => {
            secret::build(&nonce, &lock_key) == output.opening.secret
                && r.secret_bytes() == output.opening.r.0
        }
        _ => false,
    };
    if !matches {
        return Err(unexpected(format!(
            "output {index}: it is not this wallet's output at epoch {}, counter {}",
            hint.epoch, hint.counter
        )));
    }
    Ok(hint.epoch)
}

fn check_swap(
    totals: &Totals,
    outputs: &[OutputSummary],
    epoch: u32,
) -> SignerResult<(SpendKind, u64)> {
    if outputs.is_empty() {
        return Err(reject(
            RejectCode::InvalidTransaction,
            "a swap needs outputs; without them the inputs only pay the mint",
        ));
    }
    let balanced = totals
        .inputs_sat
        .checked_sub(totals.fee_sat)
        .is_some_and(|net| net == totals.outputs_sat);
    if !balanced {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!(
                "inputs of {} minus the fee of {} do not equal the outputs of {}",
                format_sats(totals.inputs_sat),
                format_sats(totals.fee_sat),
                format_sats(totals.outputs_sat)
            ),
        ));
    }
    let leaving = outputs.iter().any(|output| output.change_epoch.is_none());
    let later = outputs
        .iter()
        .any(|output| output.change_epoch.is_some_and(|change| change != epoch));
    let kind = match (leaving, later) {
        (true, _) => SpendKind::Send,
        (false, false) => SpendKind::Combine,
        (false, true) => SpendKind::Rotate,
    };
    let outflow = totals
        .leaving_sat
        .checked_add(totals.fee_sat)
        .ok_or_else(overflow)?;
    Ok((kind, outflow))
}

fn check_melt(
    totals: &Totals,
    blanks: usize,
    quote: &str,
    invoice: &str,
    amount: u64,
    fee_reserve: u64,
) -> SignerResult<InvoiceDetails> {
    let printable = quote.bytes().all(|byte| byte.is_ascii_graphic());
    if quote.is_empty() || quote.len() > MAX_QUOTE_LEN || !printable {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!("the quote id must be 1 to {MAX_QUOTE_LEN} printable ASCII characters"),
        ));
    }
    if message::starts_with_output(quote) {
        return Err(reject(
            RejectCode::InvalidTransaction,
            "the quote id begins like an output, so the message could hide another blank",
        ));
    }
    let invoice = parse_invoice(invoice)?;
    let Some(invoice_msat) = invoice.amount_msat else {
        return Err(reject(
            RejectCode::InvoiceMismatch,
            "invoices without an amount are not supported",
        ));
    };
    if amount != invoice_msat / 1000 && amount != invoice_msat.div_ceil(1000) {
        return Err(reject(
            RejectCode::InvoiceMismatch,
            format!(
                "the quote is for {} but the invoice asks for {invoice_msat} msat",
                format_sats(amount)
            ),
        ));
    }
    let required = amount
        .checked_add(fee_reserve)
        .and_then(|sum| sum.checked_add(totals.fee_sat))
        .ok_or_else(overflow)?;
    if totals.inputs_sat < required {
        return Err(reject(
            RejectCode::InvalidTransaction,
            format!(
                "inputs of {} do not cover the amount of {}, the fee reserve of {} and the input fee of {}",
                format_sats(totals.inputs_sat),
                format_sats(amount),
                format_sats(fee_reserve),
                format_sats(totals.fee_sat)
            ),
        ));
    }
    // What the mint does not spend comes back only through the blanks, one power of two each.
    let returnable = totals
        .inputs_sat
        .checked_sub(amount)
        .and_then(|rest| rest.checked_sub(totals.fee_sat))
        .ok_or_else(overflow)?;
    let needed = u64::BITS - returnable.leading_zeros();
    if u32::try_from(blanks).unwrap_or(u32::MAX) < needed {
        return Err(reject(
            RejectCode::PolicyLimit,
            format!(
                "{blanks} blank outputs cannot return up to {} of change; at least {needed} are needed",
                format_sats(returnable)
            ),
        ));
    }
    Ok(invoice)
}

fn swap_review(
    mint: &PinnedMint,
    kind: SpendKind,
    totals: &Totals,
    request: &CashuSignRequest,
    outputs: &[OutputSummary],
) -> SignerResult<Review> {
    let review = match kind {
        SpendKind::Send => Review::new(kind.title(), msat(totals.leaving_sat)?)
            .line("Amount", format_sats(totals.leaving_sat))
            .line("Mint fee", format_sats(totals.fee_sat))
            .line("Change back", format_sats(totals.change_sat)),
        SpendKind::Combine => Review::new(kind.title(), msat(totals.change_sat)?)
            .line("Proofs in", request.inputs.len().to_string())
            .line("Proofs out", outputs.len().to_string())
            .line("Mint fee", format_sats(totals.fee_sat)),
        SpendKind::Rotate | SpendKind::Melt => {
            let newest = outputs
                .iter()
                .filter_map(|output| output.change_epoch)
                .max()
                .unwrap_or(request.epoch);
            Review::new(kind.title(), msat(totals.change_sat)?)
                .line("From key", format!("epoch {}", request.epoch))
                .line("To key", format!("epoch {newest}"))
                .line("Proofs in", request.inputs.len().to_string())
                .line("Proofs out", outputs.len().to_string())
                .line("Mint fee", format_sats(totals.fee_sat))
        }
    };
    Ok(review.line("Mint", mint.url.clone()))
}

fn melt_review(
    mint: &PinnedMint,
    totals: &Totals,
    invoice: &InvoiceDetails,
    amount: u64,
    fee_reserve: u64,
) -> SignerResult<Review> {
    let mut review = Review::new(SpendKind::Melt.title(), msat(amount)?)
        .line("Amount", format_sats(amount))
        .line("To node", short_hex(&invoice.payee.serialize()));
    if let Some(description) = invoice.description.as_ref().filter(|d| !d.is_empty()) {
        review = review.line("Description", description.clone());
    }
    Ok(review
        .line("Fee reserve", format_sats(fee_reserve))
        .line("Mint fee", format_sats(totals.fee_sat))
        .line("Spends at most", format_sats(totals.inputs_sat))
        .line("Mint", mint.url.clone())
        .warn(format!(
            "This device cannot confirm the mint pays this invoice. At most {} can leave; what the mint does not spend comes back as change.",
            format_sats(totals.inputs_sat)
        )))
}

/// The review of an `AddMint`: "Cashu: add mint" for a new mint, or "Cashu: update mint keys" when
/// the mint is already pinned as `existing`, with the keysets the update adds. `pinned` is what
/// [`validate_mint`](crate::keyset::validate_mint) returned for the same `existing`, so an update
/// cannot have changed or dropped a pinned keyset.
pub fn review_add_mint(pinned: &PinnedMint, existing: Option<&PinnedMint>) -> Review {
    let active = pinned.keysets.iter().filter(|keyset| keyset.active).count();
    let fees: BTreeSet<u64> = pinned
        .keysets
        .iter()
        .filter(|keyset| keyset.active)
        .map(|keyset| keyset.input_fee_ppk)
        .collect();
    let fees = fees
        .iter()
        .map(|fee| format!("{fee} ppk"))
        .collect::<Vec<_>>()
        .join(", ");
    let title = match existing {
        None => "Cashu: add mint",
        Some(_) => "Cashu: update mint keys",
    };
    let mut review = Review::new(title, 0)
        .line("Mint", pinned.url.clone())
        .line("Unit", pinned.unit.clone())
        .line(
            "Keysets",
            format!("{}, {active} active", pinned.keysets.len()),
        )
        .line("Input fee", fees);
    match existing {
        None => {
            review = review.warn(
                "Mint keys are trusted as your phone shows them. Check this is the mint you meant to add.",
            );
        }
        Some(existing) => {
            let added = pinned
                .keysets
                .iter()
                .filter(|keyset| existing.keyset(&keyset.id).is_none())
                .count();
            review = review.line("New keysets", added.to_string());
        }
    }
    let old_ids = pinned
        .keysets
        .iter()
        .filter(|keyset| keyset.is_v00())
        .count();
    if old_ids > 0 {
        review = review.warn(format!(
            "{old_ids} of the keysets use the old id format, which does not commit to their fee."
        ));
    }
    review
}

/// `12,345 sats`.
pub fn format_sats(sats: u64) -> String {
    let digits = sats.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 5);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.push_str(" sats");
    out
}

/// First and last four bytes in hex, as Foundation shows fingerprints.
fn short_hex(bytes: &[u8]) -> String {
    let full = hex::encode(bytes);
    match (full.get(..8), full.get(full.len().saturating_sub(8)..)) {
        (Some(head), Some(tail)) if full.len() > 16 => format!("{head}…{tail}"),
        _ => full,
    }
}
