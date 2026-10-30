//! Transaction analysis: every intent's happy path, and every refusal the device must make.

mod common;

use bitcoin::hashes::{sha256, Hash};
use common::*;
use fps_protocol::{Branch, FixedBytes, InputHint, Intent, RejectCode};
use fps_signer_core::analysis::{AnalysisOptions, InputKind, OutputSummary};
use fps_signer_core::encoding::Encode;
use fps_signer_core::fedimint::lnv2::{LightningInput, OutgoingWitness};
use fps_signer_core::fedimint::mintv2::{MintInput, MintOutput, Note};
use fps_signer_core::fedimint::walletv2::StandardScript;
use fps_signer_core::fedimint::{DynItem, OutPoint};
use fps_signer_core::{analyze, Analysis, EmptyLedger, FederationKeys, Ledger, SignerError};
use lightning_invoice::Currency;

fn run(fixture: &Fixture, draft: &Draft, intent: Intent) -> Result<Analysis, SignerError> {
    run_with(
        fixture,
        draft,
        intent,
        &EmptyLedger,
        &AnalysisOptions::default(),
    )
}

fn run_with(
    fixture: &Fixture,
    draft: &Draft,
    intent: Intent,
    ledger: &dyn Ledger,
    options: &AnalysisOptions,
) -> Result<Analysis, SignerError> {
    analyze(
        &fixture.federation,
        &fixture.keys,
        &fixture.request(draft, intent),
        ledger,
        options,
    )
}

fn code(result: Result<Analysis, SignerError>) -> RejectCode {
    result.expect_err("expected a refusal").code
}

fn reissue(fixture: &mut Fixture) -> Draft {
    let mut draft = Draft::default();
    draft.note_in(fixture, Branch::Spend, 20);
    draft.note_in(fixture, Branch::Spend, 14);
    draft.change(fixture);
    draft
}

/// An invoice payment: returns the draft and the invoice.
fn payment(fixture: &mut Fixture, refund_here: bool) -> (Draft, String, [u8; 32]) {
    let payment_hash = sha256::Hash::hash(b"fixture preimage").to_byte_array();
    let invoice_msat = 30_000_000;
    let contract = fixture.contract(payment_hash, invoice_msat + 550_000, refund_here);
    let mut draft = Draft::default();
    draft.note_in(fixture, Branch::Spend, 26);
    draft.contract_out(&contract);
    draft.change(fixture);
    (
        draft,
        invoice(payment_hash, Some(invoice_msat), Currency::Regtest),
        payment_hash,
    )
}

#[test]
fn a_reissue_is_accepted_and_reviewed_as_a_self_transfer() {
    let mut fixture = Fixture::default();
    let draft = reissue(&mut fixture);
    let analysis = run(&fixture, &draft, Intent::Reissue).unwrap();
    assert_eq!(analysis.inputs.len(), 2);
    assert!(analysis
        .outputs
        .iter()
        .all(|o| matches!(o, OutputSummary::Change { .. })));
    assert_eq!(analysis.totals.external_msat, 0);
    // Only fees leave the wallet.
    assert_eq!(
        analysis.outflow_msat(),
        analysis.totals.module_fees_msat + analysis.totals.overpay_msat
    );
    assert!(analysis.totals.overpay_msat < 1 << 9);
    assert_eq!(analysis.review.title, "Reorganise notes");
}

#[test]
fn an_invoice_payment_is_accepted_and_shows_the_invoice() {
    let mut fixture = Fixture::default();
    let (draft, invoice, payment_hash) = payment(&mut fixture, true);
    let analysis = run(&fixture, &draft, Intent::PayInvoice { invoice }).unwrap();
    let payment = analysis.lightning.as_ref().unwrap();
    assert_eq!(payment.invoice.payment_hash, payment_hash);
    assert_eq!(payment.routing_fee_msat, 550_000);
    assert_eq!(payment.output_index, 0);
    assert_eq!(analysis.lightning_outpoint().unwrap().txid, analysis.txid);
    assert_eq!(analysis.totals.external_msat, 30_550_000);
    assert_eq!(analysis.review.title, "Pay Lightning invoice");
    assert_eq!(analysis.review.amount_msat, 30_000_000);
    let lines: Vec<&str> = analysis
        .review
        .lines
        .iter()
        .map(|l| l.label.as_str())
        .collect();
    assert!(lines.contains(&"Gateway fee"));
    assert!(lines.contains(&"Description"));
}

#[test]
fn an_expired_invoice_is_flagged_when_the_device_has_a_clock() {
    let mut fixture = Fixture::default();
    let (draft, invoice, _) = payment(&mut fixture, true);
    let options = AnalysisOptions {
        now_unix: Some(2_000_000_000),
        ..AnalysisOptions::default()
    };
    let analysis = run_with(
        &fixture,
        &draft,
        Intent::PayInvoice { invoice },
        &EmptyLedger,
        &options,
    )
    .unwrap();
    assert!(analysis
        .review
        .warnings
        .iter()
        .any(|w| w.contains("expired")));
}

#[test]
fn a_withdrawal_is_accepted_and_shows_the_address() {
    let mut fixture = Fixture::default();
    let mut draft = Draft::default();
    draft.note_in(&mut fixture, Branch::Spend, 30);
    draft.pegout_out(StandardScript::P2wpkh([7; 20]), 300_000, 900);
    draft.change(&mut fixture);
    let analysis = run(&fixture, &draft, Intent::Withdraw).unwrap();
    let address = analysis
        .review
        .lines
        .iter()
        .find(|l| l.label == "To address")
        .unwrap();
    assert!(address.value.starts_with("bcrt1q"));
    assert_eq!(analysis.totals.external_msat, 300_900_000);
}

#[test]
fn preparing_a_send_stages_exactly_the_requested_amount() {
    let mut fixture = Fixture::default();
    let mut draft = Draft::default();
    draft.note_in(&mut fixture, Branch::Spend, 24);
    let first = fixture.tweak();
    let second = fixture.tweak();
    draft.note_out(&fixture, Branch::Outgoing, 22, first);
    draft.note_out(&fixture, Branch::Outgoing, 19, second);
    draft.change(&mut fixture);
    let amount = (1 << 22) + (1 << 19);
    let analysis = run(
        &fixture,
        &draft,
        Intent::PrepareSend {
            amount_msat: amount,
        },
    )
    .unwrap();
    assert_eq!(analysis.totals.staged_msat, amount);
    assert_eq!(analysis.staged_notes(), vec![(22, first), (19, second)]);
    // Staged notes stay the device's: they are not outflow.
    assert_eq!(analysis.totals.external_msat, 0);

    assert_eq!(
        code(run(
            &fixture,
            &draft,
            Intent::PrepareSend {
                amount_msat: amount + 1
            }
        )),
        RejectCode::IntentMismatch
    );
}

#[test]
fn a_refund_is_verified_against_the_contract_the_device_funded() {
    let mut fixture = Fixture::default();
    let contract = fixture.contract([5; 32], 12_000_000, true);
    let outpoint = OutPoint {
        txid: [9; 32],
        out_idx: 0,
    };
    let mut draft = Draft::default();
    draft.refund_in(&contract, outpoint, OutgoingWitness::Refund);
    draft.change(&mut fixture);

    // Without a record the refund still goes through, with a warning.
    let analysis = run(&fixture, &draft, Intent::Refund).unwrap();
    assert!(matches!(
        analysis.inputs[0].kind,
        InputKind::Refund {
            verified: false,
            ..
        }
    ));
    assert!(!analysis.review.warnings.is_empty());

    // With the funded contract on record it is verified.
    let mut ledger = TestLedger::default();
    ledger
        .contracts
        .insert((fixture.id(), [9; 32], 0), contract.to_bytes());
    let analysis = run_with(
        &fixture,
        &draft,
        Intent::Refund,
        &ledger,
        &AnalysisOptions::default(),
    )
    .unwrap();
    assert!(matches!(
        analysis.inputs[0].kind,
        InputKind::Refund { verified: true, .. }
    ));
    assert!(analysis.review.warnings.is_empty());

    // A cheaper contract of ours paired with that outpoint is refused.
    let other = fixture.contract([5; 32], 1_000, true);
    let mut cheat = Draft::default();
    cheat.refund_in(&other, outpoint, OutgoingWitness::Refund);
    cheat.change(&mut fixture);
    assert_eq!(
        code(run_with(
            &fixture,
            &cheat,
            Intent::Refund,
            &ledger,
            &AnalysisOptions::default()
        )),
        RejectCode::ForeignInput
    );
}

#[test]
fn cancelled_contracts_refund_too() {
    let mut fixture = Fixture::default();
    let contract = fixture.contract([6; 32], 2_000_000, true);
    let mut draft = Draft::default();
    draft.refund_in(
        &contract,
        OutPoint {
            txid: [1; 32],
            out_idx: 3,
        },
        OutgoingWitness::Cancel([8; 64]),
    );
    draft.change(&mut fixture);
    let analysis = run(&fixture, &draft, Intent::Refund).unwrap();
    assert!(matches!(
        analysis.inputs[0].kind,
        InputKind::Refund { cancel: true, .. }
    ));
}

#[test]
fn reclaiming_released_notes_is_only_allowed_as_a_reclaim() {
    let mut fixture = Fixture::default();
    let mut draft = Draft::default();
    let tweak = draft.note_in(&mut fixture, Branch::Outgoing, 18);
    draft.change(&mut fixture);
    let mut ledger = TestLedger::default();
    ledger.released.push((fixture.id(), 18, tweak));
    let options = AnalysisOptions::default();

    let analysis = run_with(&fixture, &draft, Intent::Reclaim, &ledger, &options).unwrap();
    assert_eq!(analysis.review.amount_msat, 1 << 18);
    assert_eq!(
        code(run_with(
            &fixture,
            &draft,
            Intent::Reissue,
            &ledger,
            &options
        )),
        RejectCode::AlreadyReleased
    );
    // Without the release on record there is nothing to reclaim.
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reclaim)),
        RejectCode::IntentMismatch
    );
}

#[test]
fn notes_of_another_wallet_are_foreign() {
    let mut fixture = Fixture::default();
    let stranger = FederationKeys::derive(&[0x99; 32], &fixture.id()).unwrap();
    let tweak = fixture.tweak();
    let mut draft = Draft::default();
    draft.raw_note_in(
        Fixture::note_for(&stranger, Branch::Spend, 20, tweak),
        tweak,
        Branch::Spend,
    );
    draft.change(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::ForeignInput
    );

    // Our own note under the wrong branch hint is not ours either.
    let mut draft = Draft::default();
    let tweak = fixture.tweak();
    draft.raw_note_in(
        fixture.note(Branch::Spend, 20, tweak),
        tweak,
        Branch::Outgoing,
    );
    draft.change(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::ForeignInput
    );
}

#[test]
fn a_bad_mint_signature_is_caught_when_checking_is_on() {
    let mut fixture = Fixture::default();
    let tweak = fixture.tweak();
    let mut note = fixture.note(Branch::Spend, 20, tweak);
    note.signature = fixture.note(Branch::Spend, 21, tweak).signature;
    let mut draft = Draft::default();
    draft.raw_note_in(note, tweak, Branch::Spend);
    draft.change(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::InvalidNote
    );
    let lenient = AnalysisOptions {
        verify_note_signatures: false,
        ..AnalysisOptions::default()
    };
    assert!(run_with(&fixture, &draft, Intent::Reissue, &EmptyLedger, &lenient).is_ok());
}

#[test]
fn unknown_denominations_are_refused() {
    let mut fixture = Fixture::default();
    let tweak = fixture.tweak();
    let mut note = fixture.note(Branch::Spend, 20, tweak);
    note.denomination = 50;
    let mut draft = Draft::default();
    draft.inputs.push(DynItem {
        instance: MINT,
        payload: MintInput { note }.to_bytes(),
    });
    draft.hints.push(InputHint::Note {
        tweak: FixedBytes(tweak),
        branch: Branch::Spend,
    });
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::InvalidNote
    );

    let mut draft = reissue(&mut fixture);
    let tweak = fixture.tweak();
    let mut output = fixture.change_output(Branch::Spend, 20, tweak);
    output.denomination = 45;
    draft.outputs.push(DynItem {
        instance: MINT,
        payload: output.to_bytes(),
    });
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::UnexpectedOutput
    );
}

#[test]
fn inputs_the_phone_signs_are_never_the_devices() {
    let mut fixture = Fixture::default();
    // A peg-in claim through the wallet module.
    let mut draft = reissue(&mut fixture);
    draft.inputs.insert(
        0,
        DynItem {
            instance: WALLET,
            payload: vec![0, 0],
        },
    );
    draft.hints.insert(
        0,
        InputHint::Note {
            tweak: FixedBytes([0; 16]),
            branch: Branch::Spend,
        },
    );
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::ForeignInput
    );

    // An LNv2 preimage claim, and an incoming-contract claim.
    let contract = fixture.contract([1; 32], 1_000_000, true);
    for (payload, expected) in [
        (
            LightningInput::Outgoing {
                outpoint: OutPoint {
                    txid: [2; 32],
                    out_idx: 0,
                },
                witness: OutgoingWitness::Claim([3; 32]),
            }
            .to_bytes(),
            RejectCode::ForeignInput,
        ),
        (
            LightningInput::Incoming(vec![1, 2, 3]).to_bytes(),
            RejectCode::ForeignInput,
        ),
    ] {
        let mut draft = Draft::default();
        draft.inputs.push(DynItem {
            instance: LIGHTNING,
            payload,
        });
        draft.hints.push(InputHint::Refund {
            contract: contract.to_bytes().into(),
        });
        assert_eq!(code(run(&fixture, &draft, Intent::Refund)), expected);
    }

    // A refund whose contract refunds someone else.
    let stranger = fixture.contract([1; 32], 1_000_000, false);
    let mut draft = Draft::default();
    draft.refund_in(
        &stranger,
        OutPoint {
            txid: [2; 32],
            out_idx: 0,
        },
        OutgoingWitness::Refund,
    );
    draft.change(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Refund)),
        RejectCode::ForeignInput
    );
}

#[test]
fn modules_the_device_does_not_handle_are_refused() {
    let mut fixture = Fixture::default();
    let mut draft = reissue(&mut fixture);
    draft.outputs.push(DynItem {
        instance: META,
        payload: vec![0, 0],
    });
    let error = run(&fixture, &draft, Intent::Reissue).unwrap_err();
    assert_eq!(error.code, RejectCode::Unsupported);
    assert!(error.message.contains("meta"));

    let mut draft = reissue(&mut fixture);
    draft.outputs.push(DynItem {
        instance: 77,
        payload: vec![0, 0],
    });
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::Unsupported
    );

    // A future variant of a known module.
    let mut draft = reissue(&mut fixture);
    let mut payload = draft.inputs[0].payload.clone();
    payload[0] = 1;
    draft.inputs[0].payload = payload;
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::Unsupported
    );
}

#[test]
fn ecash_to_a_third_party_is_refused() {
    let mut fixture = Fixture::default();
    let mut draft = reissue(&mut fixture);
    let stranger = FederationKeys::derive(&[0x11; 32], &fixture.id()).unwrap();
    let tweak = fixture.tweak();
    let nonce = stranger.note_nonce(Branch::Spend, 12, &tweak).unwrap();
    let output = MintOutput {
        denomination: 12,
        nonce: stranger
            .view()
            .blinded_nonce(Branch::Spend, &nonce, 12, &tweak),
        tweak,
    };
    draft.outputs.push(DynItem {
        instance: MINT,
        payload: output.to_bytes(),
    });
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::UnexpectedOutput
    );
}

#[test]
fn change_whose_tweak_recovery_would_miss_is_refused() {
    let mut fixture = Fixture::default();
    let mut draft = Draft::default();
    draft.note_in(&mut fixture, Branch::Spend, 20);
    let tweak = fixture.bad_tweak();
    draft.note_out(&fixture, Branch::Spend, 19, tweak);
    draft.change(&mut fixture);
    let error = run(&fixture, &draft, Intent::Reissue).unwrap_err();
    assert_eq!(error.code, RejectCode::UnexpectedOutput);
    assert!(error.message.contains("recovery"));
}

#[test]
fn repeated_notes_are_refused() {
    let mut fixture = Fixture::default();
    let mut draft = Draft::default();
    let tweak = draft.note_in(&mut fixture, Branch::Spend, 20);
    draft.raw_note_in(fixture.note(Branch::Spend, 20, tweak), tweak, Branch::Spend);
    draft.change(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );

    let mut draft = Draft::default();
    draft.note_in(&mut fixture, Branch::Spend, 21);
    let tweak = fixture.tweak();
    draft.note_out(&fixture, Branch::Spend, 19, tweak);
    draft.note_out(&fixture, Branch::Spend, 19, tweak);
    draft.change(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );
}

#[test]
fn malformed_transactions_are_refused() {
    let mut fixture = Fixture::default();
    let draft = reissue(&mut fixture);
    let request = fixture.request(&draft, Intent::Reissue);
    let check = |bytes: Vec<u8>| {
        let mut request = request.clone();
        request.transaction = bytes.into();
        analyze(
            &fixture.federation,
            &fixture.keys,
            &request,
            &EmptyLedger,
            &AnalysisOptions::default(),
        )
        .unwrap_err()
        .code
    };
    let bytes = draft.bytes();
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(check(trailing), RejectCode::InvalidTransaction);
    assert_eq!(
        check(bytes[..bytes.len() - 1].to_vec()),
        RejectCode::InvalidTransaction
    );
    // The input count written as a non-minimal varint.
    let mut padded = vec![0xfd, 0x00, bytes[0]];
    padded.extend_from_slice(&bytes[1..]);
    assert_eq!(check(padded), RejectCode::InvalidTransaction);

    // No inputs at all.
    let empty = Draft::default();
    assert_eq!(
        code(run(&fixture, &empty, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );

    // Hints that do not line up with the inputs.
    let mut short = draft.clone();
    short.hints.pop();
    assert_eq!(
        code(run(&fixture, &short, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );
    let mut wrong = draft.clone();
    wrong.hints[0] = InputHint::Refund {
        contract: vec![0].into(),
    };
    assert_eq!(
        code(run(&fixture, &wrong, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );
}

#[test]
fn size_limits_are_enforced() {
    let mut fixture = Fixture::default();
    let draft = reissue(&mut fixture);
    let tight = AnalysisOptions {
        max_inputs: 1,
        ..AnalysisOptions::default()
    };
    assert_eq!(
        code(run_with(
            &fixture,
            &draft,
            Intent::Reissue,
            &EmptyLedger,
            &tight
        )),
        RejectCode::TooLarge
    );
}

#[test]
fn unbalanced_and_overpaying_transactions_are_refused() {
    let mut fixture = Fixture::default();
    // Outputs worth more than the inputs.
    let mut draft = Draft::default();
    draft.note_in(&mut fixture, Branch::Spend, 18);
    let tweak = fixture.tweak();
    draft.note_out(&fixture, Branch::Spend, 18, tweak);
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );

    // A large surplus the federation would keep as fees.
    let mut draft = Draft::default();
    draft.note_in(&mut fixture, Branch::Spend, 22);
    let tweak = fixture.tweak();
    draft.note_out(&fixture, Branch::Spend, 20, tweak);
    let error = run(&fixture, &draft, Intent::Reissue).unwrap_err();
    assert_eq!(error.code, RejectCode::PolicyLimit);
    assert!(error.message.contains("overpays"));
}

#[test]
fn outputs_must_match_the_stated_intent() {
    let mut fixture = Fixture::default();
    let (payment_draft, invoice, _) = payment(&mut fixture, true);
    assert_eq!(
        code(run(&fixture, &payment_draft, Intent::Reissue)),
        RejectCode::IntentMismatch
    );
    assert_eq!(
        code(run(&fixture, &payment_draft, Intent::Withdraw)),
        RejectCode::IntentMismatch
    );

    let draft = reissue(&mut fixture);
    assert_eq!(
        code(run(&fixture, &draft, Intent::PayInvoice { invoice })),
        RejectCode::IntentMismatch
    );
    assert_eq!(
        code(run(&fixture, &draft, Intent::Withdraw)),
        RejectCode::IntentMismatch
    );
    assert_eq!(
        code(run(&fixture, &draft, Intent::Refund)),
        RejectCode::IntentMismatch
    );
    assert_eq!(
        code(run(
            &fixture,
            &draft,
            Intent::PrepareSend {
                amount_msat: 1 << 20
            }
        )),
        RejectCode::IntentMismatch
    );
}

#[test]
fn the_contract_must_pay_exactly_the_invoice() {
    let mut fixture = Fixture::default();
    let (draft, _, payment_hash) = payment(&mut fixture, true);
    let pay = |invoice: String| run(&fixture, &draft, Intent::PayInvoice { invoice });

    // A different payment hash.
    assert_eq!(
        code(pay(invoice([1; 32], Some(30_000_000), Currency::Regtest))),
        RejectCode::InvoiceMismatch
    );
    // A different network.
    assert_eq!(
        code(pay(invoice(
            payment_hash,
            Some(30_000_000),
            Currency::Bitcoin
        ))),
        RejectCode::InvoiceMismatch
    );
    // No amount.
    assert_eq!(
        code(pay(invoice(payment_hash, None, Currency::Regtest))),
        RejectCode::InvoiceMismatch
    );
    // An invoice for more than the contract carries.
    assert_eq!(
        code(pay(invoice(
            payment_hash,
            Some(31_000_000),
            Currency::Regtest
        ))),
        RejectCode::InvoiceMismatch
    );
    // Not an invoice.
    assert_eq!(
        code(pay("lnbcrt1nonsense".to_owned())),
        RejectCode::InvoiceMismatch
    );

    // A contract whose refund would not come back.
    let (draft, invoice, _) = payment(&mut fixture, false);
    assert_eq!(
        code(run(&fixture, &draft, Intent::PayInvoice { invoice })),
        RejectCode::UnexpectedOutput
    );
}

#[test]
fn requests_for_another_federation_are_refused() {
    let mut fixture = Fixture::default();
    let draft = reissue(&mut fixture);
    let mut request = fixture.request(&draft, Intent::Reissue);
    request.federation = FixedBytes([0xee; 32]);
    let error = analyze(
        &fixture.federation,
        &fixture.keys,
        &request,
        &EmptyLedger,
        &AnalysisOptions::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, RejectCode::UnknownFederation);
}

#[test]
fn a_note_whose_signature_bytes_are_not_a_point_fails_to_decode() {
    // A note whose signature bytes are not a curve point fails decoding, not analysis.
    let mut fixture = Fixture::default();
    let tweak = fixture.tweak();
    let note: Note = fixture.note(Branch::Spend, 20, tweak);
    let mut payload = MintInput { note }.to_bytes();
    let last = payload.len() - 1;
    payload[last] ^= 0xff;
    let mut draft = Draft::default();
    draft.inputs.push(DynItem {
        instance: MINT,
        payload,
    });
    draft.hints.push(InputHint::Note {
        tweak: FixedBytes(tweak),
        branch: Branch::Spend,
    });
    assert_eq!(
        code(run(&fixture, &draft, Intent::Reissue)),
        RejectCode::InvalidTransaction
    );
}
