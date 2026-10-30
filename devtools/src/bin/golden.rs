//! Writes the golden vectors the signer core's tests check against.
//!
//!   cargo run --bin golden            (from devtools/)
//!
//! Two files under `crates/signer-core/tests/vectors/`:
//!
//! - `fedimint-0.13.0-beta.1.json`: encodings, hashes and ids produced by the real fedimint crates,
//!   plus full signing scenarios keyed to a fixed app seed.
//! - `key-scheme-fps-v1.json`: the `fps-v1` derivations, frozen so any change to the scheme is
//!   caught, and so other implementations (the phone side) can check themselves.
//!
//! Everything is seeded, so re-running produces identical files.

use std::path::PathBuf;

use bitcoin::hashes::{sha256, Hash};
use fedimint_core::encoding::Encodable;
use fedimint_core::{base32, OutPoint};
use fedimint_lnv2_common::OutgoingWitness;
use fedimint_mintv2_client::{ECash, SpendableNote};
use fedimint_walletv2_common::StandardScript;
use fps_devtools::*;
use fps_protocol::{self as proto, Branch, FixedBytes};
use fps_signer_core::keys::FederationKeys;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde::Serialize;

const FEDIMINT_VERSION: &str = "0.13.0-beta.1";
const APP_SEED: [u8; 32] = [0x5e; 32];

fn h(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(bytes)
}

/// Plain CBOR, as fps-protocol serialises message bodies.
fn cbor<T: Serialize>(value: &T) -> String {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("serialises");
    h(out)
}

#[derive(Serialize)]
struct FedimintVectors {
    generator: String,
    fedimint_version: String,
    bigsize: Vec<BigSizeCase>,
    hash_to_g1: Vec<HashCase>,
    base32: Vec<Base32Case>,
    federations: Vec<FederationCase>,
    mint_inputs: Vec<MintInputCase>,
    mint_outputs: Vec<MintOutputCase>,
    note_signatures: Vec<NoteSignatureCase>,
    contracts: Vec<ContractCase>,
    lightning_inputs: Vec<LightningInputCase>,
    wallet_outputs: Vec<WalletOutputCase>,
    ecash: Vec<EcashCase>,
    scenario: Scenario,
}

#[derive(Serialize)]
struct BigSizeCase {
    value: u64,
    encoded: String,
}

#[derive(Serialize)]
struct HashCase {
    input: String,
    sha3: String,
    sha256: String,
}

#[derive(Serialize)]
struct Base32Case {
    bytes: String,
    encoded: String,
}

#[derive(Serialize)]
struct GuardianCase {
    peer: u16,
    url: String,
    name: String,
}

#[derive(Serialize)]
struct FederationCase {
    guardians: Vec<GuardianCase>,
    encoded_endpoints: String,
    federation_id: String,
}

#[derive(Serialize)]
struct MintInputCase {
    denomination: u8,
    nonce: String,
    signature: String,
    /// The module payload of the `DynInput` (`MintInput::V0`).
    encoded: String,
}

#[derive(Serialize)]
struct MintOutputCase {
    denomination: u8,
    nonce: String,
    tweak: String,
    encoded: String,
}

#[derive(Serialize)]
struct NoteSignatureCase {
    denomination: u8,
    nonce: String,
    signature: String,
    mint_key: String,
    valid: bool,
}

#[derive(Serialize)]
struct ContractCase {
    payment_hash: String,
    amount_msat: u64,
    expiration: u64,
    claim_pk: String,
    refund_pk: String,
    ephemeral_pk: String,
    encoded: String,
    contract_id: String,
    /// The module payload of a `DynOutput` funding it (`LightningOutput::V0(Outgoing(..))`).
    output_encoded: String,
}

#[derive(Serialize)]
struct LightningInputCase {
    txid: String,
    out_idx: u64,
    /// `refund`, `cancel:<signature>` or `claim:<preimage>`.
    witness: String,
    encoded: String,
}

#[derive(Serialize)]
struct WalletOutputCase {
    kind: String,
    data: String,
    value_sat: u64,
    fee_sat: u64,
    encoded: String,
    regtest_address: String,
}

#[derive(Serialize)]
struct EcashNoteCase {
    denomination: u8,
    secret_key: String,
    signature: String,
}

#[derive(Serialize)]
struct InviteCase {
    peer: u16,
    url: String,
}

#[derive(Serialize)]
struct EcashCase {
    federation: String,
    notes: Vec<EcashNoteCase>,
    invite: Option<InviteCase>,
    token: String,
}

/// Signing requests built from real fedimint objects, keyed to `app_seed`.
#[derive(Serialize)]
struct Scenario {
    app_seed: String,
    /// CBOR of the `FederationDescriptor`.
    descriptor: String,
    federation_id: String,
    transactions: Vec<TransactionCase>,
}

#[derive(Serialize)]
struct NoteRefCase {
    denomination: u8,
    tweak: String,
}

/// An outgoing-branch note with its unblinded mint signature, as the phone fetches it from the
/// federation once the transaction that issued it is accepted.
#[derive(Serialize)]
struct IssuedNoteCase {
    denomination: u8,
    tweak: String,
    signature: String,
}

#[derive(Serialize)]
struct FundedContractCase {
    txid: String,
    out_idx: u64,
    contract: String,
}

#[derive(Serialize)]
struct TransactionCase {
    name: String,
    /// CBOR of the `SignRequest`.
    request: String,
    txid: String,
    /// Notes the device must treat as already released.
    released: Vec<NoteRefCase>,
    /// Contracts the device must remember funding.
    funded: Vec<FundedContractCase>,
    /// The outgoing-branch notes this case stages or reclaims, with their mint signatures: what
    /// the phone sends to release them.
    issued: Vec<IssuedNoteCase>,
    /// For a Lightning payment: what the gateway checks the payer's authorisation against, the
    /// SHA-256 of fedimint's encoding of `LightningInvoice::Bolt11(invoice)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_auth_message: Option<String>,
    inputs_msat: u64,
    outputs_msat: u64,
    module_fees_msat: u64,
    overpay_msat: u64,
    change_msat: u64,
    staged_msat: u64,
    external_msat: u64,
}

struct Built {
    inputs: Vec<fedimint_core::core::DynInput>,
    hints: Vec<proto::InputHint>,
    outputs: Vec<fedimint_core::core::DynOutput>,
    in_msat: u64,
    out_msat: u64,
    fees_msat: u64,
    change_msat: u64,
    staged_msat: u64,
    external_msat: u64,
}

impl Built {
    fn new() -> Self {
        Self {
            inputs: vec![],
            hints: vec![],
            outputs: vec![],
            in_msat: 0,
            out_msat: 0,
            fees_msat: 0,
            change_msat: 0,
            staged_msat: 0,
            external_msat: 0,
        }
    }

    fn spend_note(
        &mut self,
        rng: &mut ChaCha20Rng,
        fed: &TestFederation,
        keys: &FederationKeys,
        branch: Branch,
        denomination: u8,
    ) -> (u8, [u8; 16]) {
        let tweak = grind_tweak(keys.view(), rng);
        let note = fed.issue_note(keys, branch, denomination, tweak);
        self.inputs.push(note_input(note));
        self.hints.push(proto::InputHint::Note {
            tweak: FixedBytes(tweak),
            branch,
        });
        let amount = 1u64 << denomination;
        self.in_msat += amount;
        self.fees_msat += fee_msat(MINT_FEE, amount);
        (denomination, tweak)
    }

    fn note_out(
        &mut self,
        rng: &mut ChaCha20Rng,
        keys: &FederationKeys,
        branch: Branch,
        denomination: u8,
    ) -> (u8, [u8; 16]) {
        let tweak = grind_tweak(keys.view(), rng);
        self.outputs
            .push(note_output(keys, branch, denomination, tweak));
        let amount = 1u64 << denomination;
        self.out_msat += amount;
        self.fees_msat += fee_msat(MINT_FEE, amount);
        match branch {
            Branch::Spend => self.change_msat += amount,
            Branch::Outgoing => self.staged_msat += amount,
        }
        (denomination, tweak)
    }

    /// Fills the rest with change notes and returns the overpay left over.
    fn change(&mut self, rng: &mut ChaCha20Rng, keys: &FederationKeys) -> u64 {
        let available = self.in_msat - self.out_msat - self.fees_msat;
        let (denominations, leftover) = change_denominations(available);
        for denomination in denominations {
            self.note_out(rng, keys, Branch::Spend, denomination);
        }
        leftover
    }

    fn finish(
        self,
        rng: &mut ChaCha20Rng,
        fed: &TestFederation,
        name: &str,
        intent: proto::Intent,
        overpay: u64,
        released: Vec<(u8, [u8; 16])>,
        funded: Vec<FundedContractCase>,
    ) -> TransactionCase {
        let nonce: [u8; 8] = rng.gen();
        let request = proto::SignRequest {
            federation: FixedBytes(fed.id_bytes()),
            transaction: unsigned_bytes(&self.inputs, &self.outputs, nonce).into(),
            inputs: self.hints,
            intent,
        };
        TransactionCase {
            name: name.to_owned(),
            request: cbor(&request),
            txid: h(txid(&self.inputs, &self.outputs, nonce).to_byte_array()),
            released: released
                .into_iter()
                .map(|(denomination, tweak)| NoteRefCase {
                    denomination,
                    tweak: h(tweak),
                })
                .collect(),
            funded,
            issued: Vec::new(),
            gateway_auth_message: None,
            inputs_msat: self.in_msat,
            outputs_msat: self.out_msat,
            module_fees_msat: self.fees_msat,
            overpay_msat: overpay,
            change_msat: self.change_msat,
            staged_msat: self.staged_msat,
            external_msat: self.external_msat,
        }
    }
}

fn scenario(rng: &mut ChaCha20Rng) -> (Scenario, Vec<ContractCase>) {
    let fed = TestFederation::new(rng, 4);
    let keys = FederationKeys::derive(&APP_SEED, &fed.id_bytes()).expect("derivable");
    let mut transactions = Vec::new();
    let mut contracts = Vec::new();

    // A reissue: two notes in, change out.
    let mut built = Built::new();
    built.spend_note(rng, &fed, &keys, Branch::Spend, 20);
    built.spend_note(rng, &fed, &keys, Branch::Spend, 18);
    let overpay = built.change(rng, &keys);
    transactions.push(built.finish(rng, &fed, "reissue", proto::Intent::Reissue, overpay, vec![], vec![]));

    // Paying an invoice: 50,000 sats with a 600 sat gateway fee.
    let mut built = Built::new();
    built.spend_note(rng, &fed, &keys, Branch::Spend, 27);
    let payment_hash: [u8; 32] = sha256::Hash::hash(&rng.gen::<[u8; 32]>()).to_byte_array();
    let invoice_msat = 50_000_000;
    let contract = outgoing_contract(rng, &keys, payment_hash, invoice_msat + 600_000, true);
    let contract_bytes = contract.consensus_encode_to_vec();
    built.outputs.push(contract_output(contract.clone()));
    built.out_msat += contract.amount.msats;
    built.external_msat += contract.amount.msats;
    built.fees_msat += fee_msat(LIGHTNING_FEE, contract.amount.msats);
    let overpay = built.change(rng, &keys);
    let invoice = invoice(rng, payment_hash, invoice_msat);
    let gateway_auth_message = fedimint_lnv2_common::LightningInvoice::Bolt11(
        invoice.parse().expect("the invoice parses"),
    )
    .consensus_hash::<sha256::Hash>();
    let mut pay = built.finish(
        rng,
        &fed,
        "pay_invoice",
        proto::Intent::PayInvoice { invoice },
        overpay,
        vec![],
        vec![],
    );
    pay.gateway_auth_message = Some(h(gateway_auth_message.to_byte_array()));
    let funded_txid = pay.txid.clone();
    transactions.push(pay);
    contracts.push(contract_case(&contract));

    // Refunding that payment after it failed: the contract output was output 0.
    let mut built = Built::new();
    let outpoint = OutPoint {
        txid: fedimint_core::TransactionId::from_byte_array(
            hex::decode(&funded_txid).unwrap().try_into().unwrap(),
        ),
        out_idx: 0,
    };
    built
        .inputs
        .push(refund_input(outpoint, OutgoingWitness::Refund));
    built.hints.push(proto::InputHint::Refund {
        contract: contract_bytes.clone().into(),
    });
    built.in_msat += contract.amount.msats;
    built.fees_msat += fee_msat(LIGHTNING_FEE, contract.amount.msats);
    let overpay = built.change(rng, &keys);
    transactions.push(built.finish(
        rng,
        &fed,
        "refund",
        proto::Intent::Refund,
        overpay,
        vec![],
        vec![FundedContractCase {
            txid: funded_txid,
            out_idx: 0,
            contract: h(&contract_bytes),
        }],
    ));

    // Withdrawing 500,000 sats on chain with a 1,000 sat fee.
    let mut built = Built::new();
    built.spend_note(rng, &fed, &keys, Branch::Spend, 30);
    let script = StandardScript::P2WPKH(bitcoin::hashes::hash160::Hash::from_byte_array(rng.gen()));
    built.outputs.push(pegout_output(script, 500_000, 1_000));
    let pegout_msat = 501_000 * 1000;
    built.out_msat += pegout_msat;
    built.external_msat += pegout_msat;
    built.fees_msat += fee_msat(WALLET_FEE, pegout_msat);
    let overpay = built.change(rng, &keys);
    transactions.push(built.finish(rng, &fed, "withdraw", proto::Intent::Withdraw, overpay, vec![], vec![]));

    // Preparing a 2^23 + 2^20 msat ecash send: staged outgoing notes plus change.
    let mut built = Built::new();
    built.spend_note(rng, &fed, &keys, Branch::Spend, 25);
    let staged = [
        built.note_out(rng, &keys, Branch::Outgoing, 23),
        built.note_out(rng, &keys, Branch::Outgoing, 20),
    ];
    let overpay = built.change(rng, &keys);
    let mut prepare = built.finish(
        rng,
        &fed,
        "prepare_send",
        proto::Intent::PrepareSend {
            amount_msat: (1 << 23) + (1 << 20),
        },
        overpay,
        vec![],
        vec![],
    );
    prepare.issued = issued_notes(&fed, &keys, &staged);
    transactions.push(prepare);

    // Reclaiming two released outgoing notes nobody redeemed.
    let mut built = Built::new();
    let first = built.spend_note(rng, &fed, &keys, Branch::Outgoing, 16);
    let second = built.spend_note(rng, &fed, &keys, Branch::Outgoing, 15);
    let overpay = built.change(rng, &keys);
    let mut reclaim = built.finish(
        rng,
        &fed,
        "reclaim",
        proto::Intent::Reclaim,
        overpay,
        vec![first, second],
        vec![],
    );
    reclaim.issued = issued_notes(&fed, &keys, &[first, second]);
    transactions.push(reclaim);

    let scenario = Scenario {
        app_seed: h(APP_SEED),
        descriptor: cbor(&fed.descriptor()),
        federation_id: h(fed.id_bytes()),
        transactions,
    };
    (scenario, contracts)
}

/// The mint signatures `fed` issues for these outgoing-branch notes. Issuing is deterministic, so
/// these are the signatures of the very notes the scenario's transactions create and spend.
fn issued_notes(
    fed: &TestFederation,
    keys: &FederationKeys,
    notes: &[(u8, [u8; 16])],
) -> Vec<IssuedNoteCase> {
    notes
        .iter()
        .map(|&(denomination, tweak)| IssuedNoteCase {
            denomination,
            tweak: h(tweak),
            signature: h(fed
                .issue_note(keys, Branch::Outgoing, denomination, tweak)
                .signature
                .0
                .to_compressed()),
        })
        .collect()
}

fn contract_case(contract: &fedimint_lnv2_common::contracts::OutgoingContract) -> ContractCase {
    let fedimint_lnv2_common::contracts::PaymentImage::Hash(hash) = &contract.payment_image else {
        unreachable!("scenario contracts pay hashes")
    };
    ContractCase {
        payment_hash: h(hash.to_byte_array()),
        amount_msat: contract.amount.msats,
        expiration: contract.expiration,
        claim_pk: h(contract.claim_pk.serialize()),
        refund_pk: h(contract.refund_pk.serialize()),
        ephemeral_pk: h(contract.ephemeral_pk.serialize()),
        encoded: h(contract.consensus_encode_to_vec()),
        contract_id: h(contract.contract_id().0.to_byte_array()),
        output_encoded: h(dyn_payload(&contract_output(contract.clone()).consensus_encode_to_vec())),
    }
}

/// Strips a `DynInput`/`DynOutput` encoding down to its module payload.
fn dyn_payload(encoded: &[u8]) -> Vec<u8> {
    let mut reader = fps_signer_core::encoding::Reader::new(encoded);
    reader.read_u16("instance").unwrap();
    let payload = reader.read_byte_vec("payload").unwrap().to_vec();
    reader.finish("dyn item").unwrap();
    payload
}

fn fedimint_vectors() -> FedimintVectors {
    let mut rng = ChaCha20Rng::seed_from_u64(0x00f1_d3a1_2026);

    let bigsize = [
        0u64,
        1,
        0xfc,
        0xfd,
        0xfe,
        0xff,
        0x1234,
        0xffff,
        0x1_0000,
        0x1234_5678,
        0xffff_ffff,
        0x1_0000_0000,
        u64::MAX,
    ]
    .into_iter()
    .map(|value| BigSizeCase {
        value,
        encoded: h(value.consensus_encode_to_vec()),
    })
    .collect();

    let hash_to_g1 = (0..6)
        .map(|i| {
            let input: Vec<u8> = (0..(i * 11)).map(|_| rng.gen()).collect();
            HashCase {
                input: h(&input),
                sha3: h(tbs::Message::from_bytes(&input).0.to_compressed()),
                sha256: h(tbs::Message::from_bytes_sha256(&input).0.to_compressed()),
            }
        })
        .collect();

    let base32 = (0..8)
        .map(|i| {
            let bytes: Vec<u8> = (0..(i * 7)).map(|_| rng.gen()).collect();
            Base32Case {
                encoded: base32::encode_prefixed_bytes(base32::FEDIMINT_PREFIX, &bytes),
                bytes: h(&bytes),
            }
        })
        .collect();

    let federations = [1u16, 4, 7]
        .into_iter()
        .map(|guardians| {
            let fed = TestFederation::new(&mut rng, guardians);
            FederationCase {
                guardians: fed
                    .api_endpoints
                    .iter()
                    .map(|(peer, url)| GuardianCase {
                        peer: peer.to_usize() as u16,
                        url: url.url.to_string(),
                        name: url.name.clone(),
                    })
                    .collect(),
                encoded_endpoints: h(&fed.encoded_endpoints),
                federation_id: h(fed.id_bytes()),
            }
        })
        .collect();

    let fed = TestFederation::new(&mut rng, 1);
    let keys = FederationKeys::derive(&[3; 32], &fed.id_bytes()).unwrap();
    let mut mint_inputs = Vec::new();
    let mut note_signatures = Vec::new();
    for (i, denomination) in [0u8, 9, 20, 41].into_iter().enumerate() {
        let branch = if i % 2 == 0 { Branch::Spend } else { Branch::Outgoing };
        let note = fed.issue_note(&keys, branch, denomination, rng.gen());
        mint_inputs.push(MintInputCase {
            denomination,
            nonce: h(note.nonce.serialize()),
            signature: h(note.signature.0.to_compressed()),
            encoded: h(dyn_payload(&note_input(note).consensus_encode_to_vec())),
        });
        let mint_key = fed.mint.keys[&denomination];
        note_signatures.push(NoteSignatureCase {
            denomination,
            nonce: h(note.nonce.serialize()),
            signature: h(note.signature.0.to_compressed()),
            mint_key: h(mint_key.0.to_compressed()),
            valid: fedimint_mintv2_common::verify_note(note, mint_key),
        });
        // The same signature under the next denomination's key must fail.
        let other_key = fed.mint.keys[&((denomination + 1) % 42)];
        note_signatures.push(NoteSignatureCase {
            denomination,
            nonce: h(note.nonce.serialize()),
            signature: h(note.signature.0.to_compressed()),
            mint_key: h(other_key.0.to_compressed()),
            valid: fedimint_mintv2_common::verify_note(note, other_key),
        });
    }
    let mint_outputs = [0u8, 12, 33, 41]
        .into_iter()
        .map(|denomination| {
            let tweak: [u8; 16] = rng.gen();
            let output = note_output(&keys, Branch::Spend, denomination, tweak);
            let encoded = dyn_payload(&output.consensus_encode_to_vec());
            let nonce = keys
                .view()
                .blinded_nonce(
                    Branch::Spend,
                    &keys.note_nonce(Branch::Spend, denomination, &tweak).unwrap(),
                    denomination,
                    &tweak,
                )
                .to_compressed();
            MintOutputCase {
                denomination,
                nonce: h(nonce),
                tweak: h(tweak),
                encoded: h(encoded),
            }
        })
        .collect();

    let mut contracts = Vec::new();
    for amount in [1_000u64, 1u64 << 40] {
        let payment_hash: [u8; 32] = rng.gen();
        contracts.push(contract_case(&outgoing_contract(&mut rng, &keys, payment_hash, amount, true)));
    }

    let lightning_inputs = [
        ("refund".to_owned(), OutgoingWitness::Refund),
        {
            let signature: [u8; 64] = {
                let mut bytes = [0u8; 64];
                rng.fill(&mut bytes[..]);
                bytes
            };
            // Any 64 bytes encode; validity is the federation's concern.
            let sig = bitcoin::secp256k1::schnorr::Signature::from_slice(&signature).unwrap();
            (format!("cancel:{}", h(signature)), OutgoingWitness::Cancel(sig))
        },
        {
            let preimage: [u8; 32] = rng.gen();
            (format!("claim:{}", h(preimage)), OutgoingWitness::Claim(preimage))
        },
    ]
    .into_iter()
    .map(|(witness_text, witness)| {
        let txid: [u8; 32] = rng.gen();
        let out_idx = rng.gen_range(0..300u64);
        let outpoint = OutPoint {
            txid: fedimint_core::TransactionId::from_byte_array(txid),
            out_idx,
        };
        LightningInputCase {
            txid: h(txid),
            out_idx,
            witness: witness_text,
            encoded: h(dyn_payload(&refund_input(outpoint, witness).consensus_encode_to_vec())),
        }
    })
    .collect();

    let xonly = random_public_key(&mut rng).x_only_public_key().0;
    let wallet_outputs = [
        ("p2pkh", StandardScript::P2PKH(bitcoin::hashes::hash160::Hash::from_byte_array(rng.gen())), rng.gen::<[u8; 20]>().to_vec()),
        ("p2sh", StandardScript::P2SH(bitcoin::hashes::hash160::Hash::from_byte_array(rng.gen())), vec![]),
        ("p2wpkh", StandardScript::P2WPKH(bitcoin::hashes::hash160::Hash::from_byte_array(rng.gen())), vec![]),
        ("p2wsh", StandardScript::P2WSH(sha256::Hash::from_byte_array(rng.gen())), vec![]),
        ("p2tr", StandardScript::P2TR(xonly), vec![]),
    ]
    .into_iter()
    .map(|(kind, script, _)| {
        let data = match &script {
            StandardScript::P2PKH(hash) | StandardScript::P2SH(hash) | StandardScript::P2WPKH(hash) => h(hash.to_byte_array()),
            StandardScript::P2WSH(hash) => h(hash.to_byte_array()),
            StandardScript::P2TR(key) => h(key.serialize()),
            _ => unreachable!(),
        };
        let value_sat = rng.gen_range(1_000..10_000_000u64);
        let fee_sat = rng.gen_range(100..100_000u64);
        let address = bitcoin::Address::from_script(&script.script_pubkey().unwrap(), bitcoin::Network::Regtest)
            .unwrap()
            .to_string();
        WalletOutputCase {
            kind: kind.to_owned(),
            data,
            value_sat,
            fee_sat,
            encoded: h(dyn_payload(&pegout_output(script, value_sat, fee_sat).consensus_encode_to_vec())),
            regtest_address: address,
        }
    })
    .collect();

    let mut ecash = Vec::new();
    for (count, with_invite) in [(1usize, false), (3, true)] {
        let notes: Vec<SpendableNote> = (0..count)
            .map(|i| {
                let denomination = 10 + i as u8 * 3;
                let tweak: [u8; 16] = rng.gen();
                let keypair = keys.note_keypair(Branch::Outgoing, denomination, &tweak).unwrap();
                let blinding = keys.view().blinding_key(Branch::Outgoing, denomination, &tweak);
                SpendableNote {
                    denomination: fedimint_mintv2_common::Denomination(denomination),
                    keypair,
                    signature: fed.mint.issue(denomination, keypair.public_key(), blinding),
                }
            })
            .collect();
        let peer = *fed.api_endpoints.keys().next().unwrap();
        let url = fed.api_endpoints[&peer].url.clone();
        let token = if with_invite {
            let invite = fedimint_core::invite_code::InviteCode::new(url.clone(), peer, fed.id, None);
            ECash::new_with_invite(notes.clone(), &invite)
        } else {
            ECash::new(fed.id, notes.clone())
        };
        ecash.push(EcashCase {
            federation: h(fed.id_bytes()),
            notes: notes
                .iter()
                .map(|note| EcashNoteCase {
                    denomination: note.denomination.0,
                    secret_key: h(note.keypair.secret_bytes()),
                    signature: h(note.signature.0.to_compressed()),
                })
                .collect(),
            invite: with_invite.then(|| InviteCase {
                peer: peer.to_usize() as u16,
                url: url.to_string(),
            }),
            token: base32::encode_prefixed(base32::FEDIMINT_PREFIX, &token),
        });
    }

    let (scenario, scenario_contracts) = scenario(&mut rng);
    contracts.extend(scenario_contracts);

    FedimintVectors {
        generator: "devtools/src/bin/golden.rs".to_owned(),
        fedimint_version: FEDIMINT_VERSION.to_owned(),
        bigsize,
        hash_to_g1,
        base32,
        federations,
        mint_inputs,
        mint_outputs,
        note_signatures,
        contracts,
        lightning_inputs,
        wallet_outputs,
        ecash,
        scenario,
    }
}

#[derive(Serialize)]
struct KeySchemeVectors {
    scheme: String,
    generator: String,
    cases: Vec<KeyCase>,
}

#[derive(Serialize)]
struct KeyCase {
    app_seed: String,
    federation_id: String,
    spend_key: String,
    chain_code: String,
    view_secret: String,
    tweak_filter: String,
    notes: Vec<KeyNoteCase>,
    refunds: Vec<KeyRefundCase>,
}

#[derive(Serialize)]
struct KeyNoteCase {
    denomination: u8,
    tweak: String,
    passes_filter: bool,
    spend_nonce: String,
    spend_secret: String,
    spend_blinding_key: String,
    spend_blinded_nonce: String,
    outgoing_nonce: String,
    outgoing_secret: String,
    outgoing_blinding_key: String,
    outgoing_blinded_nonce: String,
}

#[derive(Serialize)]
struct KeyRefundCase {
    ephemeral: String,
    refund_key: String,
    refund_secret: String,
}

fn key_scheme_vectors() -> KeySchemeVectors {
    let mut rng = ChaCha20Rng::seed_from_u64(0x0f95_0001);
    let mut cases = Vec::new();
    for case in 0..3 {
        let app_seed: [u8; 32] = if case == 0 { [0; 32] } else { rng.gen() };
        let federation: [u8; 32] = if case == 0 { [0; 32] } else { rng.gen() };
        let keys = FederationKeys::derive(&app_seed, &federation).unwrap();
        let view = keys.view();
        let mut notes = Vec::new();
        // One tweak that passes the filter, the rest random.
        let mut tweaks: Vec<[u8; 16]> = vec![grind_tweak(view, &mut rng)];
        tweaks.extend((0..3).map(|_| rng.gen::<[u8; 16]>()));
        for (i, tweak) in tweaks.into_iter().enumerate() {
            let denomination = [0u8, 9, 27, 41][i];
            let spend = keys.note_keypair(Branch::Spend, denomination, &tweak).unwrap();
            let outgoing = keys.note_keypair(Branch::Outgoing, denomination, &tweak).unwrap();
            notes.push(KeyNoteCase {
                denomination,
                tweak: h(tweak),
                passes_filter: view.tweak_passes_filter(&tweak),
                spend_nonce: h(spend.public_key().serialize()),
                spend_secret: h(spend.secret_bytes()),
                spend_blinding_key: h(view.blinding_key(Branch::Spend, denomination, &tweak).to_bytes()),
                spend_blinded_nonce: h(view
                    .blinded_nonce(Branch::Spend, &spend.public_key(), denomination, &tweak)
                    .to_compressed()),
                outgoing_nonce: h(outgoing.public_key().serialize()),
                outgoing_secret: h(outgoing.secret_bytes()),
                outgoing_blinding_key: h(view.blinding_key(Branch::Outgoing, denomination, &tweak).to_bytes()),
                outgoing_blinded_nonce: h(view
                    .blinded_nonce(Branch::Outgoing, &outgoing.public_key(), denomination, &tweak)
                    .to_compressed()),
            });
        }
        let refunds = (0..2)
            .map(|_| {
                let ephemeral = random_public_key(&mut rng);
                let keypair = keys.refund_keypair(&ephemeral).unwrap();
                KeyRefundCase {
                    ephemeral: h(ephemeral.serialize()),
                    refund_key: h(keypair.public_key().serialize()),
                    refund_secret: h(keypair.secret_bytes()),
                }
            })
            .collect();
        cases.push(KeyCase {
            app_seed: h(app_seed),
            federation_id: h(federation),
            spend_key: h(view.spend_key().serialize()),
            chain_code: h(view.chain_code()),
            view_secret: h(view.view_secret()),
            tweak_filter: h(view.tweak_filter()),
            notes,
            refunds,
        });
    }
    KeySchemeVectors {
        scheme: fps_signer_core::keys::SCHEME.to_owned(),
        generator: "devtools/src/bin/golden.rs".to_owned(),
        cases,
    }
}

fn write(path: PathBuf, value: &impl Serialize) {
    let mut text = serde_json::to_string_pretty(value).expect("serialises");
    text.push('\n');
    std::fs::write(&path, text).expect("writes");
    println!("wrote {}", path.display());
}

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/signer-core/tests/vectors");
    std::fs::create_dir_all(&dir).expect("creates the vectors directory");
    write(dir.join(format!("fedimint-{FEDIMINT_VERSION}.json")), &fedimint_vectors());
    write(dir.join("key-scheme-fps-v1.json"), &key_scheme_vectors());
}
