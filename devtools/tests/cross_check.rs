//! Cross-checks the signer core against the real fedimint 0.13.0-beta.1 crates, live:
//!
//! - signatures the device produces pass fedimint's own `Transaction::validate_signatures`;
//! - tokens the device releases decode with fedimint's `ECash` and carry valid notes;
//! - randomly built fedimint transactions decode, re-encode and hash identically;
//! - analysis refuses fedimint-built transactions it must refuse.

use std::collections::BTreeMap;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{schnorr, PublicKey};
use fedimint_core::core::{DynInput, DynOutput};
use fedimint_core::encoding::Encodable;
use fedimint_core::transaction::{Transaction, TransactionSignature};
use fedimint_core::{base32, OutPoint};
use fedimint_lnv2_common::OutgoingWitness;
use fedimint_mintv2_client::ECash;
use fedimint_walletv2_common::StandardScript;
use fps_devtools::*;
use fps_protocol::{self as proto, Branch, Delivery, FixedBytes, InputHint, Intent, RejectCode};
use fps_signer_core::analysis::{AnalysisOptions, EmptyLedger};
use fps_signer_core::encoding::{Decode, Encode};
use fps_signer_core::fedimint::lnv2::{LightningInput, LightningOutput};
use fps_signer_core::fedimint::mintv2::{MintInput, MintOutput};
use fps_signer_core::fedimint::walletv2::WalletOutput;
use fps_signer_core::fedimint::UnsignedTransaction;
use fps_signer_core::{analyze, plan_release, sign_transaction, Federation, FederationKeys};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

struct Setup {
    rng: ChaCha20Rng,
    fed: TestFederation,
    federation: Federation,
    keys: FederationKeys,
}

fn setup(seed: u64) -> Setup {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let fed = TestFederation::new(&mut rng, 4);
    let federation = Federation::from_descriptor(&fed.descriptor()).unwrap();
    let keys = FederationKeys::derive(&rng.gen(), &fed.id_bytes()).unwrap();
    Setup {
        rng,
        fed,
        federation,
        keys,
    }
}

/// A transaction under construction, tracking what the federation will charge.
struct Draft {
    inputs: Vec<DynInput>,
    hints: Vec<InputHint>,
    keys: Vec<PublicKey>,
    outputs: Vec<DynOutput>,
    balance: i128,
}

impl Draft {
    fn new() -> Self {
        Self {
            inputs: vec![],
            hints: vec![],
            keys: vec![],
            outputs: vec![],
            balance: 0,
        }
    }

    fn note(&mut self, s: &mut Setup, branch: Branch, denomination: u8) -> (u8, [u8; 16]) {
        let tweak = grind_tweak(s.keys.view(), &mut s.rng);
        let note = s.fed.issue_note(&s.keys, branch, denomination, tweak);
        self.keys.push(note.nonce);
        self.inputs.push(note_input(note));
        self.hints.push(InputHint::Note {
            tweak: FixedBytes(tweak),
            branch,
        });
        let amount = 1u64 << denomination;
        self.balance += i128::from(amount) - i128::from(fee_msat(MINT_FEE, amount));
        (denomination, tweak)
    }

    fn output(&mut self, output: DynOutput, amount_msat: u64, fee: (u64, u64)) {
        self.outputs.push(output);
        self.balance -= i128::from(amount_msat) + i128::from(fee_msat(fee, amount_msat));
    }

    fn change(&mut self, s: &mut Setup) {
        let (denominations, _leftover) = change_denominations(self.balance as u64);
        for denomination in denominations {
            let tweak = grind_tweak(s.keys.view(), &mut s.rng);
            self.output(
                note_output(&s.keys, Branch::Spend, denomination, tweak),
                1u64 << denomination,
                MINT_FEE,
            );
        }
    }

    fn request(&self, s: &mut Setup, intent: Intent) -> (proto::SignRequest, [u8; 8]) {
        let nonce: [u8; 8] = s.rng.gen();
        (
            proto::SignRequest {
                federation: FixedBytes(s.fed.id_bytes()),
                transaction: unsigned_bytes(&self.inputs, &self.outputs, nonce).into(),
                inputs: self.hints.clone(),
                intent,
            },
            nonce,
        )
    }
}

/// Signs with the device and validates with fedimint.
fn sign_and_validate(s: &mut Setup, draft: &Draft, intent: Intent) {
    let (request, nonce) = draft.request(s, intent);
    let analysis = analyze(
        &s.federation,
        &s.keys,
        &request,
        &EmptyLedger,
        &AnalysisOptions::default(),
    )
    .unwrap();
    let mut rng = s.rng.clone();
    let signatures = sign_transaction(&s.keys, &analysis, &mut || rng.gen()).unwrap();
    let signatures: Vec<schnorr::Signature> = signatures
        .iter()
        .map(|bytes| schnorr::Signature::from_slice(bytes).unwrap())
        .collect();

    let mut transaction = Transaction {
        inputs: draft.inputs.clone(),
        outputs: draft.outputs.clone(),
        nonce,
        signatures: TransactionSignature::NaiveMultisig(signatures.clone()),
    };
    assert_eq!(transaction.tx_hash().to_byte_array(), analysis.txid);
    transaction
        .validate_signatures(&draft.keys)
        .expect("fedimint accepts the device's signatures");

    // A signature moved to the wrong input must fail fedimint's check.
    if signatures.len() > 1 {
        let mut swapped = signatures;
        swapped.swap(0, 1);
        transaction.signatures = TransactionSignature::NaiveMultisig(swapped);
        assert!(transaction.validate_signatures(&draft.keys).is_err());
    }
}

#[test]
fn fedimint_accepts_signatures_for_a_reissue() {
    let mut s = setup(1);
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 22);
    draft.note(&mut s, Branch::Spend, 19);
    draft.note(&mut s, Branch::Spend, 12);
    draft.change(&mut s);
    sign_and_validate(&mut s, &draft, Intent::Reissue);
}

#[test]
fn fedimint_accepts_signatures_for_a_lightning_payment() {
    let mut s = setup(2);
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 28);
    draft.note(&mut s, Branch::Spend, 26);
    let payment_hash = sha256::Hash::hash(b"preimage").to_byte_array();
    let invoice_msat = 21_000_000;
    let contract = outgoing_contract(&mut s.rng, &s.keys, payment_hash, invoice_msat + 400_000, true);
    let amount = contract.amount.msats;
    draft.output(contract_output(contract), amount, LIGHTNING_FEE);
    draft.change(&mut s);
    let invoice = invoice(&mut s.rng, payment_hash, invoice_msat);
    sign_and_validate(&mut s, &draft, Intent::PayInvoice { invoice });
}

#[test]
fn fedimint_accepts_signatures_for_a_withdrawal() {
    let mut s = setup(3);
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 31);
    let script = StandardScript::P2TR(random_public_key(&mut s.rng).x_only_public_key().0);
    draft.output(pegout_output(script, 700_000, 2_000), 702_000_000, WALLET_FEE);
    draft.change(&mut s);
    sign_and_validate(&mut s, &draft, Intent::Withdraw);
}

#[test]
fn fedimint_accepts_signatures_for_a_refund_and_a_staged_send() {
    let mut s = setup(4);
    // Refund of a contract this device funded.
    let payment_hash: [u8; 32] = s.rng.gen();
    let contract = outgoing_contract(&mut s.rng, &s.keys, payment_hash, 9_000_000, true);
    let mut draft = Draft::new();
    let outpoint = OutPoint {
        txid: fedimint_core::TransactionId::from_byte_array(s.rng.gen()),
        out_idx: 0,
    };
    draft.inputs.push(refund_input(outpoint, OutgoingWitness::Refund));
    draft.keys.push(contract.refund_pk);
    draft.hints.push(InputHint::Refund {
        contract: contract.consensus_encode_to_vec().into(),
    });
    draft.balance += i128::from(contract.amount.msats) - i128::from(fee_msat(LIGHTNING_FEE, contract.amount.msats));
    draft.change(&mut s);
    sign_and_validate(&mut s, &draft, Intent::Refund);

    // Staging outgoing notes for a send, mixing spend and outgoing inputs.
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 24);
    draft.note(&mut s, Branch::Outgoing, 14);
    for denomination in [21u8, 17] {
        let tweak = grind_tweak(s.keys.view(), &mut s.rng);
        draft.output(note_output(&s.keys, Branch::Outgoing, denomination, tweak), 1 << denomination, MINT_FEE);
    }
    draft.change(&mut s);
    sign_and_validate(
        &mut s,
        &draft,
        Intent::PrepareSend {
            amount_msat: (1 << 21) + (1 << 17),
        },
    );
}

#[test]
fn released_tokens_decode_and_verify_with_fedimint() {
    let mut s = setup(5);
    let mut notes = Vec::new();
    let mut nonces = Vec::new();
    for denomination in [9u8, 16, 23] {
        let tweak = grind_tweak(s.keys.view(), &mut s.rng);
        let note = s.fed.issue_note(&s.keys, Branch::Outgoing, denomination, tweak);
        nonces.push(note.nonce);
        notes.push(proto::ReleaseNote {
            denomination,
            tweak: FixedBytes(tweak),
            signature: FixedBytes(note.signature.0.to_compressed()),
        });
    }
    for include_invite in [false, true] {
        let request = proto::ReleaseRequest {
            federation: FixedBytes(s.fed.id_bytes()),
            notes: notes.clone(),
            include_invite,
            delivery: Delivery::ReturnToPhone,
        };
        let plan = plan_release(&s.federation, &s.keys, &request, &EmptyLedger, true).unwrap();
        let token = plan.token.to_token_string();

        let ecash: ECash = base32::decode_prefixed(base32::FEDIMINT_PREFIX, &token).expect("fedimint decodes the token");
        assert_eq!(ecash.mint(), Some(s.fed.id));
        assert_eq!(ecash.amount().msats, (1 << 9) + (1 << 16) + (1 << 23));
        assert_eq!(ecash.amount().msats, plan.amount_msat);
        let decoded = ecash.notes();
        assert_eq!(decoded.len(), 3);
        for (note, nonce) in decoded.iter().zip(&nonces) {
            assert_eq!(note.keypair.public_key(), *nonce);
            let public = fedimint_mintv2_common::Note {
                denomination: note.denomination,
                nonce: *nonce,
                signature: note.signature,
            };
            assert!(fedimint_mintv2_common::verify_note(public, s.fed.mint.keys[&note.denomination.0]));
        }
        match ecash.federation_invite() {
            Some(invite) => {
                assert!(include_invite);
                assert_eq!(invite.federation_id(), s.fed.id);
                assert_eq!(invite.url(), s.fed.api_endpoints.values().next().unwrap().url);
            }
            None => assert!(!include_invite),
        }
    }
}

#[test]
fn random_fedimint_transactions_round_trip_and_hash_identically() {
    let mut s = setup(6);
    for round in 0..40 {
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for _ in 0..s.rng.gen_range(1..6) {
            match s.rng.gen_range(0..2) {
                0 => {
                    let tweak = s.rng.gen();
                    let denomination = s.rng.gen_range(0..42);
                    let note = s.fed.issue_note(&s.keys, Branch::Spend, denomination, tweak);
                    inputs.push(note_input(note));
                }
                _ => {
                    let outpoint = OutPoint {
                        txid: fedimint_core::TransactionId::from_byte_array(s.rng.gen()),
                        out_idx: s.rng.gen_range(0..100_000),
                    };
                    inputs.push(refund_input(outpoint, OutgoingWitness::Refund));
                }
            }
        }
        for _ in 0..s.rng.gen_range(0..6) {
            match s.rng.gen_range(0..4) {
                0 => {
                    let denomination = s.rng.gen_range(0..42);
                    let tweak = s.rng.gen();
                    outputs.push(note_output(&s.keys, Branch::Spend, denomination, tweak));
                }
                1 => {
                    let denomination = s.rng.gen_range(0..42);
                    outputs.push(foreign_note_output(&mut s.rng, denomination));
                }
                2 => {
                    let payment_hash: [u8; 32] = s.rng.gen();
                    let amount = s.rng.gen_range(1..u64::from(u32::MAX));
                    let refund_here = s.rng.gen();
                    let contract = outgoing_contract(&mut s.rng, &s.keys, payment_hash, amount, refund_here);
                    outputs.push(contract_output(contract));
                }
                _ => outputs.push(pegout_output(
                    StandardScript::P2WSH(sha256::Hash::from_byte_array(s.rng.gen())),
                    s.rng.gen_range(0..u64::from(u32::MAX)),
                    s.rng.gen_range(0..1_000_000),
                )),
            }
        }
        let nonce: [u8; 8] = s.rng.gen();
        let bytes = unsigned_bytes(&inputs, &outputs, nonce);
        let tx = UnsignedTransaction::decode_canonical(&bytes)
            .unwrap_or_else(|error| panic!("round {round}: {error}"));
        assert_eq!(tx.to_bytes(), bytes, "round {round}: re-encoding");
        assert_eq!(tx.txid(), txid(&inputs, &outputs, nonce).to_byte_array(), "round {round}: txid");
        for item in &tx.inputs {
            match item.instance {
                MINT_INSTANCE => assert_eq!(MintInput::decode_exact(&item.payload, "input").unwrap().to_bytes(), item.payload),
                LIGHTNING_INSTANCE => assert_eq!(LightningInput::decode_exact(&item.payload, "input").unwrap().to_bytes(), item.payload),
                other => panic!("unexpected input instance {other}"),
            }
        }
        for item in &tx.outputs {
            match item.instance {
                MINT_INSTANCE => assert_eq!(MintOutput::decode_exact(&item.payload, "output").unwrap().to_bytes(), item.payload),
                LIGHTNING_INSTANCE => assert_eq!(LightningOutput::decode_exact(&item.payload, "output").unwrap().to_bytes(), item.payload),
                WALLET_INSTANCE => assert_eq!(WalletOutput::decode_exact(&item.payload, "output").unwrap().to_bytes(), item.payload),
                other => panic!("unexpected output instance {other}"),
            }
        }
    }
}

fn refusal(s: &mut Setup, draft: &Draft, intent: Intent) -> RejectCode {
    let (request, _) = draft.request(s, intent);
    analyze(&s.federation, &s.keys, &request, &EmptyLedger, &AnalysisOptions::default())
        .expect_err("the device must refuse")
        .code
}

#[test]
fn fedimint_built_attacks_are_refused() {
    let mut s = setup(7);

    // Ecash issued to a third party alongside change.
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 20);
    let foreign = foreign_note_output(&mut s.rng, 18);
    draft.output(foreign, 1 << 18, MINT_FEE);
    draft.change(&mut s);
    assert_eq!(refusal(&mut s, &draft, Intent::Reissue), RejectCode::UnexpectedOutput);

    // A contract whose refund would go to someone else.
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 26);
    let payment_hash = s.rng.gen();
    let contract = outgoing_contract(&mut s.rng, &s.keys, payment_hash, 10_100_000, false);
    draft.output(contract_output(contract), 10_100_000, LIGHTNING_FEE);
    draft.change(&mut s);
    let invoice = invoice(&mut s.rng, payment_hash, 10_000_000);
    assert_eq!(refusal(&mut s, &draft, Intent::PayInvoice { invoice }), RejectCode::UnexpectedOutput);

    // A note that belongs to a different wallet.
    let other = FederationKeys::derive(&[9; 32], &s.fed.id_bytes()).unwrap();
    let mut draft = Draft::new();
    let tweak = grind_tweak(other.view(), &mut s.rng);
    draft.inputs.push(note_input(s.fed.issue_note(&other, Branch::Spend, 20, tweak)));
    draft.hints.push(InputHint::Note {
        tweak: FixedBytes(tweak),
        branch: Branch::Spend,
    });
    draft.balance = (1 << 20) - 205;
    draft.change(&mut s);
    assert_eq!(refusal(&mut s, &draft, Intent::Reissue), RejectCode::ForeignInput);

    // Burning value as fees: change far below what the inputs cover.
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 24);
    let tweak = grind_tweak(s.keys.view(), &mut s.rng);
    draft.output(note_output(&s.keys, Branch::Spend, 20, tweak), 1 << 20, MINT_FEE);
    assert_eq!(refusal(&mut s, &draft, Intent::Reissue), RejectCode::PolicyLimit);

    // Stating one intent while doing another: a peg-out declared as a reissue.
    let mut draft = Draft::new();
    draft.note(&mut s, Branch::Spend, 30);
    draft.output(
        pegout_output(StandardScript::P2WPKH(bitcoin::hashes::hash160::Hash::from_byte_array(s.rng.gen())), 100_000, 500),
        100_500_000,
        WALLET_FEE,
    );
    draft.change(&mut s);
    assert_eq!(refusal(&mut s, &draft, Intent::Reissue), RejectCode::IntentMismatch);
}

#[test]
fn fedimint_descriptor_round_trips_through_validation() {
    let mut rng = ChaCha20Rng::seed_from_u64(8);
    let fed = TestFederation::new(&mut rng, 7);
    let federation = Federation::from_descriptor(&fed.descriptor()).unwrap();
    assert_eq!(federation.id, fed.id_bytes());
    assert_eq!(federation.guardians.len(), 7);
    let names: BTreeMap<u16, String> = federation.guardians.iter().map(|g| (g.peer, g.name.clone())).collect();
    for (peer, url) in &fed.api_endpoints {
        assert_eq!(names[&(peer.to_usize() as u16)], url.name);
    }
    assert_eq!(federation.mint.keys.len(), 42);
}
