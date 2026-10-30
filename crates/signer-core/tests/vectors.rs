//! Checks the signer core against vectors produced by the real fedimint 0.13.0-beta.1 crates
//! (`devtools/src/bin/golden.rs`). Every decoder must read fedimint's bytes, every encoder must
//! reproduce them exactly, and every hash and id must match.

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{schnorr, Message, PublicKey, Secp256k1};
use fps_protocol::{FederationDescriptor, FixedBytes, NoteRef, SignRequest};
use fps_signer_core::analysis::{AnalysisOptions, Ledger};
use fps_signer_core::encoding::{Decode, Encode, Reader, Writer};
use fps_signer_core::fedimint::ecash::{EcashToken, Invite, SpendableNote};
use fps_signer_core::fedimint::endpoints::{
    decode_api_endpoints, encode_api_endpoints, federation_id,
};
use fps_signer_core::fedimint::lnv2::{
    LightningInput, LightningOutput, OutgoingContract, OutgoingWitness, PaymentImage,
};
use fps_signer_core::fedimint::mintv2::{MintInput, MintOutput};
use fps_signer_core::fedimint::primitives::{g1_from_bytes, g2_from_bytes};
use fps_signer_core::fedimint::walletv2::{StandardScript, WalletOutput};
use fps_signer_core::fedimint::{base32, OutPoint, UnsignedTransaction};
use fps_signer_core::{analyze, sign_transaction, tbs, Federation, FederationKeys};
use serde_json::Value;

fn vectors() -> Value {
    let text = include_str!("vectors/fedimint-0.13.0-beta.1.json");
    serde_json::from_str(text).expect("valid json")
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("hex string")).expect("valid hex")
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().expect("right length")
}

fn cases<'a>(vectors: &'a Value, key: &str) -> &'a Vec<Value> {
    let cases = vectors[key].as_array().expect("array of cases");
    assert!(!cases.is_empty(), "no {key} vectors");
    cases
}

#[test]
fn vectors_come_from_the_pinned_fedimint_release() {
    assert_eq!(vectors()["fedimint_version"], "0.13.0-beta.1");
}

#[test]
fn bigsize_matches_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "bigsize") {
        let value = case["value"].as_u64().unwrap();
        let encoded = bytes(&case["encoded"]);
        let mut writer = Writer::new();
        writer.write_bigsize(value);
        assert_eq!(writer.as_bytes(), encoded.as_slice(), "encode {value}");
        let mut reader = Reader::new(&encoded);
        assert_eq!(
            reader.read_bigsize("value").unwrap(),
            value,
            "decode {value}"
        );
        reader.finish("value").unwrap();
    }
}

#[test]
fn hash_to_g1_matches_fedimint_tbs() {
    let vectors = vectors();
    for case in cases(&vectors, "hash_to_g1") {
        let input = bytes(&case["input"]);
        assert_eq!(
            tbs::hash_to_g1_sha3(&input).to_compressed().to_vec(),
            bytes(&case["sha3"])
        );
        assert_eq!(
            tbs::hash_to_g1_sha256(&input).to_compressed().to_vec(),
            bytes(&case["sha256"])
        );
    }
}

#[test]
fn base32_matches_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "base32") {
        let raw = bytes(&case["bytes"]);
        let encoded = case["encoded"].as_str().unwrap();
        assert_eq!(base32::encode_prefixed(&raw), encoded);
        assert_eq!(base32::decode_prefixed(encoded).unwrap(), raw);
        assert_eq!(
            base32::decode_prefixed(&encoded.to_uppercase()).unwrap(),
            raw
        );
    }
}

#[test]
fn federation_ids_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "federations") {
        let encoded = bytes(&case["encoded_endpoints"]);
        assert_eq!(
            federation_id(&encoded).to_vec(),
            bytes(&case["federation_id"])
        );
        let guardians = decode_api_endpoints(&encoded).unwrap();
        let expected = case["guardians"].as_array().unwrap();
        assert_eq!(guardians.len(), expected.len());
        for (guardian, expected) in guardians.iter().zip(expected) {
            assert_eq!(u64::from(guardian.peer), expected["peer"].as_u64().unwrap());
            assert_eq!(guardian.url, expected["url"].as_str().unwrap());
            assert_eq!(guardian.name, expected["name"].as_str().unwrap());
        }
        assert_eq!(encode_api_endpoints(&guardians), encoded);
    }
}

#[test]
fn mint_inputs_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "mint_inputs") {
        let encoded = bytes(&case["encoded"]);
        let input = MintInput::decode_exact(&encoded, "mint input").unwrap();
        assert_eq!(
            u64::from(input.note.denomination),
            case["denomination"].as_u64().unwrap()
        );
        assert_eq!(input.note.nonce.serialize().to_vec(), bytes(&case["nonce"]));
        assert_eq!(
            input.note.signature.to_compressed().to_vec(),
            bytes(&case["signature"])
        );
        assert_eq!(input.to_bytes(), encoded);
    }
}

#[test]
fn mint_outputs_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "mint_outputs") {
        let encoded = bytes(&case["encoded"]);
        let output = MintOutput::decode_exact(&encoded, "mint output").unwrap();
        assert_eq!(
            u64::from(output.denomination),
            case["denomination"].as_u64().unwrap()
        );
        // `nonce` was computed with the signer core's blinding, `encoded` with fedimint's
        // `blind_message`: equal bytes mean the two blind identically.
        assert_eq!(output.nonce.to_compressed().to_vec(), bytes(&case["nonce"]));
        assert_eq!(output.tweak.to_vec(), bytes(&case["tweak"]));
        assert_eq!(output.to_bytes(), encoded);
    }
}

#[test]
fn note_signature_checks_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "note_signatures") {
        let nonce = PublicKey::from_slice(&bytes(&case["nonce"])).unwrap();
        let signature = g1_from_bytes(&array(&case["signature"])).unwrap();
        let key = g2_from_bytes(&array(&case["mint_key"])).unwrap();
        assert_eq!(
            tbs::verify(&tbs::nonce_message(&nonce), &signature, &key),
            case["valid"].as_bool().unwrap()
        );
    }
}

#[test]
fn outgoing_contracts_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "contracts") {
        let encoded = bytes(&case["encoded"]);
        let contract = OutgoingContract::decode_exact(&encoded, "contract").unwrap();
        assert_eq!(
            contract.payment_image,
            PaymentImage::Hash(array(&case["payment_hash"]))
        );
        assert_eq!(contract.amount_msat, case["amount_msat"].as_u64().unwrap());
        assert_eq!(contract.expiration, case["expiration"].as_u64().unwrap());
        assert_eq!(
            contract.claim_pk.serialize().to_vec(),
            bytes(&case["claim_pk"])
        );
        assert_eq!(
            contract.refund_pk.serialize().to_vec(),
            bytes(&case["refund_pk"])
        );
        assert_eq!(
            contract.ephemeral_pk.serialize().to_vec(),
            bytes(&case["ephemeral_pk"])
        );
        assert_eq!(contract.to_bytes(), encoded);
        assert_eq!(contract.contract_id().to_vec(), bytes(&case["contract_id"]));

        let output_encoded = bytes(&case["output_encoded"]);
        let output = LightningOutput::decode_exact(&output_encoded, "lightning output").unwrap();
        assert_eq!(output, LightningOutput::Outgoing(contract));
        assert_eq!(output.to_bytes(), output_encoded);
    }
}

#[test]
fn lightning_inputs_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "lightning_inputs") {
        let encoded = bytes(&case["encoded"]);
        let input = LightningInput::decode_exact(&encoded, "lightning input").unwrap();
        let LightningInput::Outgoing { outpoint, witness } = &input else {
            panic!("expected an outgoing input");
        };
        assert_eq!(outpoint.txid.to_vec(), bytes(&case["txid"]));
        assert_eq!(outpoint.out_idx, case["out_idx"].as_u64().unwrap());
        let text = case["witness"].as_str().unwrap();
        match witness {
            OutgoingWitness::Refund => assert_eq!(text, "refund"),
            OutgoingWitness::Cancel(signature) => {
                assert_eq!(text, format!("cancel:{}", hex::encode(signature)))
            }
            OutgoingWitness::Claim(preimage) => {
                assert_eq!(text, format!("claim:{}", hex::encode(preimage)))
            }
        }
        assert_eq!(input.to_bytes(), encoded);
    }
}

#[test]
fn wallet_outputs_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "wallet_outputs") {
        let encoded = bytes(&case["encoded"]);
        let output = WalletOutput::decode_exact(&encoded, "wallet output").unwrap();
        let data = bytes(&case["data"]);
        let kind = case["kind"].as_str().unwrap();
        match (&output.destination, kind) {
            (StandardScript::P2pkh(hash), "p2pkh")
            | (StandardScript::P2sh(hash), "p2sh")
            | (StandardScript::P2wpkh(hash), "p2wpkh") => assert_eq!(hash.to_vec(), data),
            (StandardScript::P2wsh(hash), "p2wsh") => assert_eq!(hash.to_vec(), data),
            (StandardScript::P2tr(key), "p2tr") => assert_eq!(key.serialize().to_vec(), data),
            (other, kind) => panic!("decoded {other:?} for a {kind} vector"),
        }
        assert_eq!(output.value_sat, case["value_sat"].as_u64().unwrap());
        assert_eq!(output.fee_sat, case["fee_sat"].as_u64().unwrap());
        assert_eq!(
            output
                .destination
                .address(bitcoin::Network::Regtest)
                .unwrap(),
            case["regtest_address"].as_str().unwrap()
        );
        assert_eq!(output.to_bytes(), encoded);
    }
}

#[test]
fn ecash_tokens_match_fedimint() {
    let vectors = vectors();
    for case in cases(&vectors, "ecash") {
        let notes = case["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| SpendableNote {
                denomination: note["denomination"].as_u64().unwrap() as u8,
                secret_key: array(&note["secret_key"]),
                signature: g1_from_bytes(&array(&note["signature"])).unwrap(),
            })
            .collect();
        let invite = case["invite"].as_object().map(|invite| Invite {
            peer: invite["peer"].as_u64().unwrap() as u16,
            url: invite["url"].as_str().unwrap().to_owned(),
        });
        let token = EcashToken {
            federation: array(&case["federation"]),
            notes,
            invite,
        };
        assert_eq!(token.to_token_string(), case["token"].as_str().unwrap());
    }
}

/// The ledger a scenario case describes.
struct CaseLedger {
    federation: [u8; 32],
    released: Vec<NoteRef>,
    funded: Vec<(OutPoint, Vec<u8>)>,
}

impl Ledger for CaseLedger {
    fn is_released(&self, federation: &[u8; 32], denomination: u8, tweak: &[u8; 16]) -> bool {
        *federation == self.federation
            && self
                .released
                .iter()
                .any(|note| note.denomination == denomination && note.tweak.0 == *tweak)
    }

    fn approved_contract(&self, federation: &[u8; 32], outpoint: &OutPoint) -> Option<Vec<u8>> {
        (*federation == self.federation)
            .then(|| {
                self.funded
                    .iter()
                    .find(|(o, _)| o == outpoint)
                    .map(|(_, c)| c.clone())
            })
            .flatten()
    }
}

#[test]
fn scenario_transactions_analyse_and_sign_as_fedimint_built_them() {
    let vectors = vectors();
    let scenario = &vectors["scenario"];
    let app_seed: [u8; 32] = array(&scenario["app_seed"]);
    let descriptor: FederationDescriptor =
        ciborium::from_reader(bytes(&scenario["descriptor"]).as_slice()).unwrap();
    let federation = Federation::from_descriptor(&descriptor).unwrap();
    assert_eq!(federation.id.to_vec(), bytes(&scenario["federation_id"]));
    let keys = FederationKeys::derive(&app_seed, &federation.id).unwrap();
    let secp = Secp256k1::verification_only();

    for case in cases(scenario, "transactions") {
        let name = case["name"].as_str().unwrap();
        let request: SignRequest =
            ciborium::from_reader(bytes(&case["request"]).as_slice()).unwrap();
        let ledger = CaseLedger {
            federation: federation.id,
            released: case["released"]
                .as_array()
                .unwrap()
                .iter()
                .map(|note| NoteRef {
                    denomination: note["denomination"].as_u64().unwrap() as u8,
                    tweak: FixedBytes(array(&note["tweak"])),
                })
                .collect(),
            funded: case["funded"]
                .as_array()
                .unwrap()
                .iter()
                .map(|funded| {
                    (
                        OutPoint {
                            txid: array(&funded["txid"]),
                            out_idx: funded["out_idx"].as_u64().unwrap(),
                        },
                        bytes(&funded["contract"]),
                    )
                })
                .collect(),
        };

        // The phone's bytes are fedimint's canonical encoding of the parts it hashes.
        let tx = UnsignedTransaction::decode_canonical(request.transaction.as_slice()).unwrap();
        assert_eq!(tx.txid().to_vec(), bytes(&case["txid"]), "{name}: txid");

        let analysis = analyze(
            &federation,
            &keys,
            &request,
            &ledger,
            &AnalysisOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            analysis.txid.to_vec(),
            bytes(&case["txid"]),
            "{name}: analysed txid"
        );
        let totals = &analysis.totals;
        for (field, value) in [
            ("inputs_msat", totals.inputs_msat),
            ("outputs_msat", totals.outputs_msat),
            ("module_fees_msat", totals.module_fees_msat),
            ("overpay_msat", totals.overpay_msat),
            ("change_msat", totals.change_msat),
            ("staged_msat", totals.staged_msat),
            ("external_msat", totals.external_msat),
        ] {
            assert_eq!(value, case[field].as_u64().unwrap(), "{name}: {field}");
        }

        let mut counter = 0u8;
        let signatures = sign_transaction(&keys, &analysis, &mut || {
            counter += 1;
            [counter; 32]
        })
        .unwrap();
        assert_eq!(
            signatures.len(),
            analysis.inputs.len(),
            "{name}: one signature per input"
        );
        let message = Message::from_digest(analysis.txid);
        for (signature, input) in signatures.iter().zip(&analysis.inputs) {
            let signature = schnorr::Signature::from_slice(signature).unwrap();
            secp.verify_schnorr(
                &signature,
                &message,
                &input.public_key.x_only_public_key().0,
            )
            .unwrap_or_else(|_| panic!("{name}: signature does not verify"));
        }
        assert_eq!(
            bitcoin::hashes::sha256::Hash::hash(request.transaction.as_slice()).to_byte_array(),
            analysis.txid
        );
    }
}
