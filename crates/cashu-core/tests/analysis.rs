//! Spend analysis: every rule of "What Prime checks before it signs", accepted and refused.

mod common;

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use common::*;
use fps_cashu_core::analysis::{
    format_sats, MAX_EPOCHS_AHEAD, MAX_INPUTS, MAX_OUTPUTS, MAX_SECRET_LEN,
};
use fps_cashu_core::keys::MintKeys;
use fps_cashu_core::{review_add_mint, secret, sign_spend, validate_mint, SpendKind};
use fps_protocol::{ByteBuf, CashuOutputHint, CashuSpendKind, FixedBytes, RejectCode};

fn line<'a>(review: &'a fps_signer_core::review::Review, label: &str) -> &'a str {
    review
        .lines
        .iter()
        .find(|line| line.label == label)
        .map(|line| line.value.as_str())
        .unwrap_or_else(|| panic!("no {label} line in {review:?}"))
}

#[test]
fn a_send_shows_what_leaves_and_counts_it_with_the_fee() {
    let fixture = Fixture::new();
    let analysis = fixture.analyze(&send(&fixture)).unwrap();
    assert_eq!(analysis.kind, SpendKind::Send);
    assert_eq!(analysis.review.title, "Cashu: send ecash");
    assert_eq!(analysis.totals.inputs_sat, 13);
    assert_eq!(analysis.totals.fee_sat, 1);
    assert_eq!(analysis.totals.leaving_sat, 8);
    assert_eq!(analysis.totals.change_sat, 4);
    assert_eq!(analysis.outflow_msat, 9_000);
    assert_eq!(analysis.review.amount_msat, 8_000);
    assert_eq!(line(&analysis.review, "Amount"), "8 sats");
    assert_eq!(line(&analysis.review, "Mint fee"), "1 sats");
    assert_eq!(line(&analysis.review, "Change back"), "4 sats");
    assert_eq!(line(&analysis.review, "Mint"), URL);
    assert!(analysis.review.warnings.is_empty());
}

#[test]
fn combining_and_moving_to_a_new_key_cost_only_the_fee() {
    let fixture = Fixture::new();
    let combine = fixture.swap(
        0,
        vec![
            fixture.proof(B, 0, 0, 2),
            fixture.proof(B, 0, 1, 2),
            fixture.proof(B, 0, 2, 4),
        ],
        vec![fixture.change(C, 0, 0, 8)],
    );
    let analysis = fixture.analyze(&combine).unwrap();
    assert_eq!(analysis.kind, SpendKind::Combine);
    assert_eq!(analysis.review.title, "Cashu: combine ecash");
    assert_eq!(analysis.outflow_msat, 0);
    assert_eq!(analysis.review.amount_msat, 8_000);
    assert_eq!(line(&analysis.review, "Proofs in"), "3");
    assert_eq!(line(&analysis.review, "Proofs out"), "1");

    // Proofs of epoch 4 into keys of epochs 5 and 7, through the fee-charging keyset.
    let rotate = fixture.swap(
        4,
        vec![fixture.proof(A, 4, 0, 16), fixture.proof(A, 4, 1, 16)],
        vec![
            fixture.change(A, 5, 0, 16),
            fixture.change(C, 7, 0, 8),
            fixture.change(C, 4, 3, 4),
            fixture.change(A, 7, 1, 2),
            fixture.change(A, 5, 1, 1),
        ],
    );
    let analysis = fixture.analyze(&rotate).unwrap();
    assert_eq!(analysis.kind, SpendKind::Rotate);
    assert_eq!(analysis.review.title, "Cashu: move to a new key");
    assert_eq!(analysis.outflow_msat, 1_000);
    assert_eq!(line(&analysis.review, "From key"), "epoch 4");
    assert_eq!(line(&analysis.review, "To key"), "epoch 7");
}

#[test]
fn a_melt_counts_every_input_as_leaving_and_says_why() {
    let fixture = Fixture::new();
    let analysis = fixture.analyze(&melt(&fixture)).unwrap();
    assert_eq!(analysis.kind, SpendKind::Melt);
    assert_eq!(analysis.review.title, "Cashu: pay Lightning invoice");
    // The headline is the quote; what can leave is every input, 104 sats.
    assert_eq!(analysis.review.amount_msat, 101_000);
    assert_eq!(analysis.outflow_msat, 104_000);
    assert_eq!(line(&analysis.review, "Amount"), "101 sats");
    // The payee is the node that signed the invoice (`common::invoice` signs with 0x77…77).
    let node = SecretKey::from_slice(&[0x77; 32])
        .unwrap()
        .public_key(&Secp256k1::new());
    let node = hex::encode(node.serialize());
    assert_eq!(
        line(&analysis.review, "To node"),
        format!("{}…{}", &node[..8], &node[58..])
    );
    assert_eq!(line(&analysis.review, "Description"), "coffee");
    assert_eq!(line(&analysis.review, "Fee reserve"), "2 sats");
    assert_eq!(line(&analysis.review, "Mint fee"), "1 sats");
    assert_eq!(line(&analysis.review, "Spends at most"), "104 sats");
    assert_eq!(line(&analysis.review, "Mint"), URL);
    let warning = analysis.review.warnings.join(" ");
    assert!(
        warning.contains("cannot confirm the mint pays this invoice"),
        "{warning}"
    );
    assert!(warning.contains("At most 104 sats can leave"), "{warning}");
    assert!(analysis
        .message
        .ends_with("0192f4c8-1d2b-7c5a-8e3f-2b7c9d0a1e5f"));
}

#[test]
fn the_signature_covers_prime_s_own_message_with_the_epoch_key() {
    let fixture = Fixture::new();
    for request in [send(&fixture), melt(&fixture)] {
        let analysis = fixture.analyze(&request).unwrap();
        let signature = sign_spend(&fixture.keys, &analysis, [9; 32]).unwrap();
        let lock_key = fixture.keys.lock_pubkey(analysis.epoch).unwrap();
        let (x_only, _) = lock_key.x_only_public_key();
        let digest = bitcoin::hashes::sha256::Hash::hash(analysis.message.as_bytes());
        Secp256k1::verification_only()
            .verify_schnorr(
                &bitcoin::secp256k1::schnorr::Signature::from_slice(&signature).unwrap(),
                &Message::from_digest(digest.to_byte_array()),
                &x_only,
            )
            .unwrap();
        assert_eq!(digest.to_byte_array(), analysis.message_hash);
        // Fresh randomness gives another signature over the same message.
        assert_ne!(
            sign_spend(&fixture.keys, &analysis, [10; 32]).unwrap(),
            signature
        );
    }

    // An analysis whose hash is not its message's is never signed.
    let mut analysis = fixture.analyze(&send(&fixture)).unwrap();
    analysis.message_hash[0] ^= 1;
    let error = sign_spend(&fixture.keys, &analysis, [9; 32]).unwrap_err();
    assert_eq!(error.code, RejectCode::Internal);
}

#[test]
fn requests_for_another_mint_or_of_the_wrong_size_are_refused() {
    let fixture = Fixture::new();
    let mut request = send(&fixture);
    request.mint = FixedBytes([7; 32]);
    fixture.refuse(&request, RejectCode::UnknownFederation, "different mint");

    // Keys of another mint never analyse this one's requests.
    let other = MintKeys::derive(&APP_SEED, "https://other.example").unwrap();
    let error = fps_cashu_core::analyze(&send(&fixture), &fixture.mint, &other).unwrap_err();
    assert_eq!(error.code, RejectCode::UnknownFederation);

    let mut request = send(&fixture);
    request.inputs.clear();
    fixture.refuse(&request, RejectCode::InvalidTransaction, "no inputs");

    let mut request = send(&fixture);
    let input = request.inputs[0].clone();
    request.inputs = vec![input; MAX_INPUTS + 1];
    fixture.refuse(&request, RejectCode::TooLarge, "limits");

    let mut request = send(&fixture);
    let output = request.outputs[0].clone();
    request.outputs = vec![output; MAX_OUTPUTS + 1];
    fixture.refuse(&request, RejectCode::TooLarge, "limits");
}

#[test]
fn the_limits_are_inclusive() {
    let fixture = Fixture::new();
    // 64 inputs of 2 sats from the free keyset C into 64 outputs of 2 sats, the change moving on
    // to new counters while the outputs read two ways, as an honest phone's would.
    let inputs: Vec<_> = (0..MAX_INPUTS as u64)
        .map(|counter| fixture.proof(C, 0, counter, 2))
        .collect();
    let analysis = (0..64u64)
        .find_map(|first| {
            let outputs = (0..MAX_OUTPUTS as u64)
                .map(|counter| fixture.change(C, 1, 1_000 + first * 64 + counter, 2))
                .collect();
            match fixture.analyze(&fixture.swap(0, inputs.clone(), outputs)) {
                Ok(analysis) => Some(analysis),
                Err(error) if error.message.contains("more than one way") => None,
                Err(error) => panic!("{}", error.message),
            }
        })
        .expect("counters that read one way");
    assert_eq!(analysis.kind, SpendKind::Rotate);
    assert_eq!(analysis.totals.change_sat, 128);

    // A leaving output whose secret is exactly as long as accepted.
    let mut request = send(&fixture);
    let longest = "a".repeat(MAX_SECRET_LEN);
    request.outputs[1] = opened_output(&fixture.id(A), 8, &longest, &scalar(b"r"), None);
    fixture.analyze(&request).unwrap();
}

#[test]
fn inputs_must_be_the_template_locked_to_the_epoch_key() {
    let fixture = Fixture::new();
    // Locked to epoch 1's key, in a request for epoch 0.
    let mut request = send(&fixture);
    request.inputs[1] = fixture.proof(A, 1, 1, 4);
    fixture.refuse(&request, RejectCode::ForeignInput, "input 1 is not locked");

    // Another wallet's proof, with the same template shape.
    let other = MintKeys::derive(&[1; 32], URL).unwrap();
    let foreign = other.phone().lock_secret(0, &fixture.id(A), 1).unwrap();
    let mut request = send(&fixture);
    request.inputs[1].secret = foreign;
    fixture.refuse(&request, RejectCode::ForeignInput, "input 1");

    // The same lock in any other spelling: upper-case hex, other key order, spaces, more tags.
    let original = request_secret(&fixture);
    let lock = &original[92..158];
    let nonce = &original[18..82];
    let variants = [
        original.to_uppercase(),
        original.replacen(nonce, &nonce.to_uppercase(), 1),
        format!(r#"["P2PK",{{"data":"{lock}","nonce":"{nonce}","tags":[["sigflag","SIG_ALL"]]}}]"#),
        format!(
            r#"["P2PK", {{"nonce":"{nonce}","data":"{lock}","tags":[["sigflag","SIG_ALL"]]}}]"#
        ),
        format!(
            r#"["P2PK",{{"nonce":"{nonce}","data":"{lock}","tags":[["sigflag","SIG_ALL"],["locktime","1"]]}}]"#
        ),
        format!(
            r#"["P2PK",{{"nonce":"{nonce}","data":"{lock}","tags":[["sigflag","SIG_INPUTS"]]}}]"#
        ),
        format!(r#"["P2PK",{{"nonce":"{nonce}","data":"{lock}"}}]"#),
        format!(r#"["HTLC",{{"nonce":"{nonce}","data":"{lock}","tags":[["sigflag","SIG_ALL"]]}}]"#),
        nonce.to_owned(),
        String::new(),
    ];
    assert!(secret::parse(&original).is_some());
    for variant in variants {
        assert!(secret::parse(&variant).is_none(), "{variant}");
        let mut request = send(&fixture);
        request.inputs[0].secret = variant;
        fixture.refuse(&request, RejectCode::ForeignInput, "input 0");
    }
}

fn request_secret(fixture: &Fixture) -> String {
    send(fixture).inputs[0].secret.clone()
}

#[test]
fn inputs_must_be_distinct_proofs_of_pinned_keys() {
    let fixture = Fixture::new();
    let mut request = send(&fixture);
    request.inputs[2] = request.inputs[0].clone();
    fixture.refuse(&request, RejectCode::InvalidTransaction, "input 2 repeats");

    let mut request = send(&fixture);
    request.inputs[0].keyset = ByteBuf(vec![1; 33]);
    fixture.refuse(&request, RejectCode::InvalidNote, "not pinned");

    // 3 is no power of two; 2^40 is one the keyset does not have.
    for amount in [3, 1 << 40, 0] {
        let mut request = send(&fixture);
        request.inputs[0].amount = amount;
        fixture.refuse(&request, RejectCode::InvalidNote, "has no key for");
    }

    let mut request = send(&fixture);
    request.inputs[0].c = FixedBytes([2; 33]);
    request.inputs[0].c.0[1] = 0xff;
    let point = bitcoin::secp256k1::PublicKey::from_slice(&request.inputs[0].c.0);
    if point.is_err() {
        fixture.refuse(&request, RejectCode::InvalidNote, "not a point");
    }
    let mut request = send(&fixture);
    request.inputs[0].c = FixedBytes([0; 33]);
    fixture.refuse(&request, RejectCode::InvalidNote, "not a point");
}

#[test]
fn dleq_proofs_are_checked_when_present_and_required_for_a_melt() {
    let fixture = Fixture::new();
    // A swap without DLEQ proofs is fine; with them, they must verify.
    let mut request = send(&fixture);
    for input in &mut request.inputs {
        input.dleq = None;
    }
    fixture.analyze(&request).unwrap();

    let mut request = send(&fixture);
    request.inputs[1].dleq.as_mut().unwrap().s.0[31] ^= 1;
    fixture.refuse(&request, RejectCode::InvalidNote, "input 1: its DLEQ proof");

    // A proof claiming more than it is worth fails against the key of the claimed amount.
    let mut request = melt(&fixture);
    request.inputs[2].amount = 16;
    fixture.refuse(&request, RejectCode::InvalidNote, "input 2: its DLEQ proof");

    let mut request = melt(&fixture);
    request.inputs[0].dleq = None;
    fixture.refuse(&request, RejectCode::InvalidNote, "needs a DLEQ proof");

    // e, s and r must be scalars.
    let mut request = melt(&fixture);
    request.inputs[0].dleq.as_mut().unwrap().r = FixedBytes([0; 32]);
    fixture.refuse(&request, RejectCode::InvalidNote, "DLEQ");
}

#[test]
fn outputs_must_be_in_active_pinned_keysets_with_keys_for_their_amounts() {
    let fixture = Fixture::new();
    let mut request = send(&fixture);
    request.outputs[1].keyset = ByteBuf(vec![1; 8]);
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "not pinned");

    // Keyset B is inactive: inputs may come from it, outputs may not.
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, 8)],
        vec![fixture.change(B, 0, 0, 8)],
    );
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "inactive");

    // Keyset A has amounts up to 2^31.
    for amount in [0, 3, 1 << 40] {
        let request = fixture.swap(
            0,
            vec![fixture.proof(C, 0, 0, 8)],
            vec![fixture.leaving(A, amount, 1)],
        );
        fixture.refuse(&request, RejectCode::UnexpectedOutput, "has no key for");
    }
}

#[test]
fn every_output_must_open_to_its_blinded_message() {
    let fixture = Fixture::new();
    // A blinding factor or secret changed after blinding.
    let mut request = send(&fixture);
    request.outputs[1].opening.r.0[31] ^= 1;
    fixture.refuse(
        &request,
        RejectCode::UnexpectedOutput,
        "output 1: B_ is not",
    );
    let mut request = send(&fixture);
    request.outputs[1].opening.secret.push('0');
    fixture.refuse(
        &request,
        RejectCode::UnexpectedOutput,
        "output 1: B_ is not",
    );
    let mut request = send(&fixture);
    request.outputs[0].b.0[5] ^= 1;
    fixture.refuse(
        &request,
        RejectCode::UnexpectedOutput,
        "output 0: B_ is not",
    );

    let mut request = send(&fixture);
    request.outputs[1].opening.r = FixedBytes([0; 32]);
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "not a valid scalar");
    let mut request = send(&fixture);
    request.outputs[1].opening.r = FixedBytes([0xff; 32]);
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "not a valid scalar");

    let mut request = send(&fixture);
    let long = "a".repeat(1025);
    request.outputs[1] = opened_output(&fixture.id(A), 8, &long, &scalar(b"r"), None);
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "longer than 1024");
}

#[test]
fn change_must_be_the_wallet_s_output_at_its_hint() {
    let fixture = Fixture::new();
    // Up to three epochs ahead is change; four is refused, and so is an earlier epoch.
    let request = fixture.swap(
        5,
        vec![fixture.proof(C, 5, 0, 8)],
        vec![fixture.change(C, 5 + MAX_EPOCHS_AHEAD, 0, 8)],
    );
    assert_eq!(fixture.analyze(&request).unwrap().kind, SpendKind::Rotate);
    for epoch in [4, 5 + MAX_EPOCHS_AHEAD + 1, u32::MAX] {
        let request = fixture.swap(
            5,
            vec![fixture.proof(C, 5, 0, 8)],
            vec![fixture.change(C, epoch, 0, 8)],
        );
        fixture.refuse(
            &request,
            RejectCode::UnexpectedOutput,
            "outside epochs 5 to 5 + 3",
        );
    }

    // A hint that does not reproduce the output: another counter, epoch or keyset.
    let hints = [
        (0, 11, fixture.id(A)),
        (1, 10, fixture.id(A)),
        (0, 10, fixture.id(C)),
    ];
    for (epoch, counter, keyset) in hints {
        let mut request = send(&fixture);
        request.outputs[0].opening.hint = Some(CashuOutputHint {
            epoch,
            keyset: ByteBuf(keyset),
            counter,
        });
        let words = if request.outputs[0].keyset.0
            != request.outputs[0].opening.hint.as_ref().unwrap().keyset.0
        {
            "its hint names keyset"
        } else {
            "not this wallet's output"
        };
        fixture.refuse(&request, RejectCode::UnexpectedOutput, words);
    }

    // A leaving output dressed up as change.
    let mut request = send(&fixture);
    request.outputs[1].opening.hint = Some(CashuOutputHint {
        epoch: 0,
        keyset: ByteBuf(fixture.id(A)),
        counter: 0,
    });
    fixture.refuse(
        &request,
        RejectCode::UnexpectedOutput,
        "not this wallet's output",
    );

    // Without its hint, the wallet's own output counts as leaving.
    let mut request = send(&fixture);
    request.outputs[0].opening.hint = None;
    let analysis = fixture.analyze(&request).unwrap();
    assert_eq!(analysis.totals.leaving_sat, 12);
    assert_eq!(analysis.outflow_msat, 13_000);
}

#[test]
fn a_hint_must_reproduce_both_the_secret_and_r() {
    // Either half alone is not the wallet's output: a stranger's secret blinded with the wallet's
    // own r is a send in disguise, and the wallet's secret under another r is change a restore
    // never finds.
    let fixture = Fixture::new();
    let own = fixture
        .keys
        .phone()
        .blinded_output(0, &fixture.id(A), 10)
        .unwrap();
    let hint = || {
        Some(CashuOutputHint {
            epoch: 0,
            keyset: ByteBuf(fixture.id(A)),
            counter: 10,
        })
    };
    for (secret, r) in [
        ("recipient secret", own.r),
        (own.secret.as_str(), scalar(b"other r")),
    ] {
        let mut request = send(&fixture);
        request.outputs[0] = opened_output(&fixture.id(A), 4, secret, &r, hint());
        fixture.refuse(
            &request,
            RejectCode::UnexpectedOutput,
            "output 0: it is not this wallet's output at epoch 0, counter 10",
        );
    }
}

#[test]
fn a_secret_appears_once_among_inputs_and_outputs() {
    let fixture = Fixture::new();
    // Change at the position of a proof being spent would be born spent.
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, 8)],
        vec![fixture.change(C, 0, 0, 8)],
    );
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "reuses the secret");

    let mut request = send(&fixture);
    let secret = request.outputs[0].opening.secret.clone();
    request.outputs[1] = opened_output(&fixture.id(A), 8, &secret, &scalar(b"other r"), None);
    fixture.refuse(
        &request,
        RejectCode::UnexpectedOutput,
        "output 1 reuses the secret",
    );

    let mut request = send(&fixture);
    request.outputs.push(request.outputs[1].clone());
    fixture.refuse(&request, RejectCode::UnexpectedOutput, "reuses the secret");
}

#[test]
fn the_fee_is_rounded_up_over_all_inputs_and_a_swap_must_balance() {
    let fixture = Fixture::new();
    // Eleven proofs at 100 ppk cost 2 sats, ten cost 1.
    for (count, fee) in [(10u64, 1u64), (11, 2)] {
        let inputs = (0..count)
            .map(|counter| fixture.proof(A, 0, counter, 2))
            .collect();
        let request = fixture.swap(0, inputs, vec![fixture.leaving(C, 2 * count - fee, 1)]);
        let request = if (2 * count - fee).is_power_of_two() {
            request
        } else {
            // Split the outputs into powers of two.
            let mut outputs = Vec::new();
            let mut rest = 2 * count - fee;
            let mut seed = 1;
            while rest > 0 {
                let part = 1u64 << (63 - rest.leading_zeros());
                outputs.push(fixture.leaving(C, part, seed));
                rest -= part;
                seed += 1;
            }
            fixture.swap(0, request.inputs, outputs)
        };
        let analysis = fixture.analyze(&request).unwrap();
        assert_eq!(analysis.totals.fee_sat, fee, "{count} inputs");
    }

    // Free inputs from B mixed with A's: one input at 100 ppk still costs 1 sat.
    let request = fixture.swap(
        0,
        vec![fixture.proof(B, 0, 0, 8), fixture.proof(A, 0, 0, 1)],
        vec![fixture.leaving(C, 8, 1)],
    );
    assert_eq!(fixture.analyze(&request).unwrap().totals.fee_sat, 1);

    // Outputs that keep the fee, or take more than the inputs.
    for outputs in [
        vec![
            fixture.change(A, 0, 10, 4),
            fixture.leaving(A, 8, 1),
            fixture.leaving(A, 1, 2),
        ],
        vec![fixture.change(A, 0, 10, 4), fixture.leaving(A, 4, 1)],
    ] {
        let mut request = send(&fixture);
        request.outputs = outputs;
        fixture.refuse(
            &request,
            RejectCode::InvalidTransaction,
            "do not equal the outputs",
        );
    }

    // Inputs that only pay the fee.
    let mut request = send(&fixture);
    request.inputs.truncate(1);
    request.inputs[0] = fixture.proof(A, 0, 3, 1);
    request.outputs.clear();
    fixture.refuse(
        &request,
        RejectCode::InvalidTransaction,
        "a swap needs outputs",
    );
}

#[test]
fn amounts_that_overflow_are_refused() {
    let fixture = Fixture::new();
    // Two proofs of 2^63 sats overflow a u64; one overflows in msat.
    let big = 1u64 << 63;
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, big), fixture.proof(C, 0, 1, big)],
        vec![fixture.leaving(C, big, 1)],
    );
    fixture.refuse(&request, RejectCode::InvalidTransaction, "amounts overflow");
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, big)],
        vec![fixture.leaving(C, big, 1)],
    );
    fixture.refuse(&request, RejectCode::InvalidTransaction, "amounts overflow");
    // 2^53 sats still converts.
    let fine = 1u64 << 53;
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, fine)],
        vec![fixture.leaving(C, fine, 1)],
    );
    assert_eq!(fixture.analyze(&request).unwrap().outflow_msat, fine * 1000);
}

#[test]
fn a_melt_must_match_its_invoice() {
    let fixture = Fixture::new();
    let request = melt(&fixture);

    // Rounded down, as CDK's fake backend quotes, is accepted too.
    let mut floor = request.clone();
    if let CashuSpendKind::Melt {
        amount,
        fee_reserve,
        ..
    } = &mut floor.kind
    {
        *amount = 100;
        *fee_reserve = 3;
    }
    fixture.analyze(&floor).unwrap();

    for wrong in [99, 102, 1_000] {
        let mut request = request.clone();
        if let CashuSpendKind::Melt { amount, .. } = &mut request.kind {
            *amount = wrong;
        }
        fixture.refuse(
            &request,
            RejectCode::InvoiceMismatch,
            "the invoice asks for 100500 msat",
        );
    }

    let no_amount = fixture.melt(
        0,
        request.inputs.clone(),
        request.outputs.clone(),
        None,
        101,
        2,
    );
    fixture.refuse(&no_amount, RejectCode::InvoiceMismatch, "without an amount");

    let mut garbage = request.clone();
    if let CashuSpendKind::Melt { invoice, .. } = &mut garbage.kind {
        invoice.replace_range(20..24, "qqqq");
    }
    let error = fixture.analyze(&garbage).unwrap_err();
    assert_eq!(error.code, RejectCode::InvoiceMismatch);
}

#[test]
fn a_melt_must_cover_the_quote_and_return_all_change_through_its_own_blanks() {
    let fixture = Fixture::new();
    let request = melt(&fixture);

    let mut short = request.clone();
    if let CashuSpendKind::Melt { fee_reserve, .. } = &mut short.kind {
        *fee_reserve = 3;
    }
    fixture.refuse(&short, RejectCode::InvalidTransaction, "do not cover");

    // Up to 2 sats can come back: two blanks are needed (bit length of 2), one is too few.
    let mut few = request.clone();
    few.outputs.truncate(1);
    fixture.refuse(
        &few,
        RejectCode::PolicyLimit,
        "1 blank outputs cannot return up to 2 sats",
    );

    // One proof of 128 for 21 sats: 106 can come back, which needs seven blanks.
    let input = fixture.proof(A, 0, 40, 128);
    let six = fixture.melt(
        0,
        vec![input.clone()],
        fixture.blanks(A, 0, 41, 6),
        Some(21_000),
        21,
        1,
    );
    fixture.refuse(&six, RejectCode::PolicyLimit, "at least 7 are needed");
    let seven = fixture.melt(
        0,
        vec![input],
        fixture.blanks(A, 0, 41, 7),
        Some(21_000),
        21,
        1,
    );
    let analysis = fixture.analyze(&seven).unwrap();
    assert_eq!(analysis.outflow_msat, 128_000);

    // Nothing to return, no blanks needed: 64 sats for a 63-sat quote and the 1-sat fee.
    let exact = fixture.melt(
        0,
        vec![fixture.proof(A, 0, 50, 64)],
        vec![],
        Some(63_000),
        63,
        0,
    );
    assert_eq!(fixture.analyze(&exact).unwrap().outflow_msat, 64_000);

    // Blanks carry amount 0, are the wallet's change, in an active keyset.
    let mut paid = request.clone();
    paid.outputs[0] = fixture.change(A, 0, 30, 1);
    fixture.refuse(&paid, RejectCode::UnexpectedOutput, "blanks of amount 0");
    let mut foreign = request.clone();
    foreign.outputs[1] = fixture.leaving(A, 0, 1);
    fixture.refuse(
        &foreign,
        RejectCode::UnexpectedOutput,
        "must be this wallet's change",
    );
    let mut inactive = request.clone();
    inactive.outputs[1] = fixture.change(B, 0, 31, 0);
    fixture.refuse(&inactive, RejectCode::UnexpectedOutput, "inactive");
}

#[test]
fn melt_quote_ids_are_printable_and_cannot_hide_an_output() {
    let fixture = Fixture::new();
    let lock = fixture.keys.lock_pubkey(0).unwrap().serialize();
    let hidden_output = format!("0{}", hex::encode(lock));
    for quote in [
        String::new(),
        "a".repeat(129),
        "quote with spaces".to_owned(),
        "quote\u{7}".to_owned(),
        "ünicode".to_owned(),
        format!("{hidden_output}0192f4c8-1d2b-7c5a-8e3f-2b7c9d0a1e5f"),
    ] {
        let mut request = melt(&fixture);
        if let CashuSpendKind::Melt { quote: slot, .. } = &mut request.kind {
            *slot = quote.clone();
        }
        fixture.refuse(&request, RejectCode::InvalidTransaction, "the quote id");
    }
    for quote in [
        "cF8911fzT88aEi1d-6boZZkq5lYxbUSVs-HbJxK0",
        "1",
        &"a".repeat(128),
    ] {
        let mut request = melt(&fixture);
        if let CashuSpendKind::Melt { quote: slot, .. } = &mut request.kind {
            *slot = quote.to_owned();
        }
        fixture.analyze(&request).unwrap();
    }
}

#[test]
fn an_output_list_with_two_readings_is_refused() {
    // 1,024 followed by a `B_` ending in three digits reads either way when the key beginning
    // `024`, as half of all keys of that form are, is a point: the amount 1 and that key, then the
    // three digits in front of the next amount. Prime grinds nothing, so search the wallet's own
    // change for such an output.
    let fixture = Fixture::new();
    let phone = fixture.keys.phone();
    let id = fixture.id(C);
    let counter = (100u64..)
        .find(|counter| {
            let b = hex::encode(
                phone
                    .blinded_output(0, &id, *counter)
                    .unwrap()
                    .b
                    .serialize(),
            );
            b[63..].bytes().all(|byte| byte.is_ascii_digit())
                && &b[63..64] != "0"
                && is_point_hex(&format!("024{}", &b[..63]))
        })
        .unwrap();
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, 1024), fixture.proof(C, 0, 1, 2)],
        vec![
            fixture.change(C, 0, counter, 1024),
            fixture.change(C, 0, counter + 1, 2),
        ],
    );
    fixture.refuse(
        &request,
        RejectCode::InvalidTransaction,
        "more than one way",
    );

    // The same outputs the other way round read one way only.
    let request = fixture.swap(
        0,
        vec![fixture.proof(C, 0, 0, 1024), fixture.proof(C, 0, 1, 2)],
        vec![
            fixture.change(C, 0, counter + 1, 2),
            fixture.change(C, 0, counter, 1024),
        ],
    );
    assert_eq!(fixture.analyze(&request).unwrap().kind, SpendKind::Combine);
}

#[test]
fn format_sats_groups_thousands() {
    assert_eq!(format_sats(0), "0 sats");
    assert_eq!(format_sats(999), "999 sats");
    assert_eq!(format_sats(1_000), "1,000 sats");
    assert_eq!(format_sats(1_234_567), "1,234,567 sats");
    assert_eq!(format_sats(u64::MAX), "18,446,744,073,709,551,615 sats");
}

#[test]
fn adding_and_updating_a_mint_have_their_own_reviews() {
    let fixture = Fixture::new();
    let review = review_add_mint(&fixture.mint, None);
    assert_eq!(review.title, "Cashu: add mint");
    assert_eq!(review.amount_msat, 0);
    assert_eq!(line(&review, "Mint"), URL);
    assert_eq!(line(&review, "Unit"), "sat");
    assert_eq!(line(&review, "Keysets"), "3, 2 active");
    assert_eq!(line(&review, "Input fee"), "0 ppk, 100 ppk");
    let warnings = review.warnings.join(" ");
    assert!(
        warnings.contains("trusted as your phone shows them"),
        "{warnings}"
    );
    assert!(
        warnings.contains("1 of the keysets use the old id format"),
        "{warnings}"
    );

    // The mint adds a keyset, deactivates A and no longer lists B, which stays pinned.
    let added = TestKeyset::new(3, 8, true, 0, true, None);
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].active = false;
    descriptor.keysets.remove(B);
    descriptor.keysets.push(added.wire());
    let updated = validate_mint(&descriptor, Some(&fixture.mint)).unwrap();
    let review = review_add_mint(&updated, Some(&fixture.mint));
    assert_eq!(review.title, "Cashu: update mint keys");
    assert_eq!(review.amount_msat, 0);
    assert_eq!(line(&review, "Mint"), URL);
    assert_eq!(line(&review, "Keysets"), "4, 2 active");
    assert_eq!(line(&review, "Input fee"), "0 ppk");
    assert_eq!(line(&review, "New keysets"), "1");
    let warnings = review.warnings.join(" ");
    assert!(!warnings.contains("trusted as your phone"), "{warnings}");
    assert!(
        warnings.contains("1 of the keysets use the old id format"),
        "{warnings}"
    );
}

#[test]
fn reviews_never_reuse_a_fedimint_title() {
    // The simulator drivers key on review titles; every Cashu title starts with "Cashu".
    for kind in [
        SpendKind::Send,
        SpendKind::Combine,
        SpendKind::Rotate,
        SpendKind::Melt,
    ] {
        assert!(kind.title().starts_with("Cashu: "), "{}", kind.title());
    }
}

#[test]
fn the_vector_collision_is_refused_by_the_one_reading_rule_alone() {
    // The negative vector built from the wallet's own change (devtools-cashu): every other rule
    // passes, and the analysis refuses only because the outputs read two ways.
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("vectors/cashu-0.18.3.json")).unwrap();
    let mint = &vectors["mint"];
    let descriptor: fps_protocol::CashuMintDescriptor = ciborium::from_reader(
        hex::decode(mint["descriptor"].as_str().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    let pinned = validate_mint(&descriptor, None).unwrap();
    let seed: [u8; 32] = hex::decode(mint["app_seed"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let keys = MintKeys::derive(&seed, mint["url"].as_str().unwrap()).unwrap();
    let case = vectors["refused"]["spends"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "collision-from-own-change")
        .unwrap();
    let request: fps_protocol::CashuSignRequest = ciborium::from_reader(
        hex::decode(case["request"].as_str().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    let error = fps_cashu_core::analyze(&request, &pinned, &keys).unwrap_err();
    assert_eq!(error.code, RejectCode::InvalidTransaction);
    assert!(
        error.message.contains("more than one way"),
        "{}",
        error.message
    );

    // Prime would have shown 4 sats leaving: 1,024 and 1 are its change.
    assert_eq!(
        request.outputs.iter().map(|o| o.amount).collect::<Vec<_>>(),
        [1024, 1, 4]
    );
    assert!(request.outputs[0].opening.hint.is_some() && request.outputs[1].opening.hint.is_some());
    assert!(request.outputs[2].opening.hint.is_none());
}
