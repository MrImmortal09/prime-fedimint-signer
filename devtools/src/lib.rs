//! Builders that create fedimint 0.13 objects with the real fedimint crates, keyed to the device's
//! `fps-v1` scheme. The golden vector generator and the cross-check tests both use them, so every
//! byte the signer core decodes in those tests was produced by fedimint itself.

use std::collections::BTreeMap;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use fedimint_core::config::{FederationId, GlobalClientConfig, PeerUrl};
use fedimint_core::core::{DynInput, DynOutput, IntoDynInstance};
use fedimint_core::encoding::Encodable;
use fedimint_core::module::CoreConsensusVersion;
use fedimint_core::transaction::Transaction;
use fedimint_core::util::SafeUrl;
use fedimint_core::{Amount, OutPoint, PeerId, TransactionId};
use fedimint_lnv2_common::contracts::{OutgoingContract, PaymentImage};
use fedimint_lnv2_common::{
    LightningInput, LightningInputV0, LightningOutput, LightningOutputV0, OutgoingWitness,
};
use fedimint_mintv2_common::{nonce_message, Denomination, MintInput, MintOutput, Note};
use fedimint_walletv2_common::{StandardScript, WalletOutput, WalletOutputV0};
use fps_protocol as proto;
use fps_protocol::Branch;
use fps_signer_core::keys::{FederationKeys, ViewKeys};
use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
use rand::{Rng, RngCore};

pub const MINT_INSTANCE: u16 = 0;
pub const LIGHTNING_INSTANCE: u16 = 1;
pub const WALLET_INSTANCE: u16 = 2;
/// A module the device does not handle, present in real federations.
pub const META_INSTANCE: u16 = 3;

/// Fee rules of the test federation. Non-zero everywhere so the fee arithmetic is exercised.
pub const MINT_FEE: (u64, u64) = (100, 100);
pub const LIGHTNING_FEE: (u64, u64) = (1_000, 100);
pub const WALLET_FEE: (u64, u64) = (100_000, 100);
pub const DUST_LIMIT_SAT: u64 = 1_000;

pub fn fee_msat((base, ppm): (u64, u64), amount_msat: u64) -> u64 {
    amount_msat * ppm / 1_000_000 + base
}

/// A one-guardian mint with a key per mint v2 denomination.
pub struct TestMint {
    secrets: BTreeMap<u8, tbs::SecretKeyShare>,
    pub keys: BTreeMap<u8, tbs::AggregatePublicKey>,
}

fn random_scalar(rng: &mut impl RngCore) -> bls12_381::Scalar {
    let mut wide = [0u8; 64];
    rng.fill_bytes(&mut wide);
    bls12_381::Scalar::from_bytes_wide(&wide)
}

impl TestMint {
    pub fn new(rng: &mut impl RngCore) -> Self {
        let mut secrets = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for denomination in 0..=fps_signer_core::fedimint::mintv2::MAX_DENOMINATION {
            let secret = tbs::SecretKeyShare(random_scalar(rng));
            // With one guardian the aggregate key is that guardian's share.
            keys.insert(
                denomination,
                tbs::AggregatePublicKey(tbs::derive_pk_share(&secret).0),
            );
            secrets.insert(denomination, secret);
        }
        Self { secrets, keys }
    }

    /// Issues a note the way a federation does: the client blinds the nonce message, the guardian
    /// signs the blinded message, the client unblinds.
    pub fn issue(
        &self,
        denomination: u8,
        nonce: PublicKey,
        blinding: bls12_381::Scalar,
    ) -> tbs::Signature {
        let key = tbs::BlindingKey(blinding);
        let blinded = tbs::blind_message(nonce_message(nonce), key);
        let share = tbs::sign_message(blinded, self.secrets[&denomination]);
        let blinded_signature =
            tbs::aggregate_signature_shares(&BTreeMap::from([(0u64, share)]));
        tbs::unblind_signature(key, blinded_signature)
    }

    pub fn descriptor_keys(&self) -> Vec<proto::DenominationKey> {
        self.keys
            .iter()
            .map(|(denomination, key)| proto::DenominationKey {
                denomination: *denomination,
                key: proto::FixedBytes(key.0.to_compressed()),
            })
            .collect()
    }
}

/// A federation built with fedimint's own config types.
pub struct TestFederation {
    pub id: FederationId,
    pub api_endpoints: BTreeMap<PeerId, PeerUrl>,
    pub encoded_endpoints: Vec<u8>,
    pub mint: TestMint,
    pub network: bitcoin::Network,
    pub name: String,
}

impl TestFederation {
    pub fn new(rng: &mut impl RngCore, guardians: u16) -> Self {
        let api_endpoints: BTreeMap<PeerId, PeerUrl> = (0..guardians)
            .map(|peer| {
                (
                    PeerId::new(peer),
                    PeerUrl {
                        url: SafeUrl::parse(&format!("wss://guardian-{peer}.example.com/"))
                            .expect("valid url"),
                        name: format!("Guardian {peer}"),
                    },
                )
            })
            .collect();
        let name = "Test Federation".to_owned();
        let global = GlobalClientConfig {
            api_endpoints: api_endpoints.clone(),
            broadcast_public_keys: None,
            consensus_version: CoreConsensusVersion::new(2, 1),
            meta: BTreeMap::from([("federation_name".to_owned(), name.clone())]),
        };
        Self {
            id: global.calculate_federation_id(),
            encoded_endpoints: api_endpoints.consensus_encode_to_vec(),
            api_endpoints,
            mint: TestMint::new(rng),
            network: bitcoin::Network::Regtest,
            name,
        }
    }

    pub fn id_bytes(&self) -> [u8; 32] {
        self.id.0.to_byte_array()
    }

    /// The descriptor a phone would extract from this federation's client config.
    pub fn descriptor(&self) -> proto::FederationDescriptor {
        let module = |instance, kind: &str| proto::ModuleInfo {
            instance,
            kind: kind.to_owned(),
            major: 1,
            minor: 0,
        };
        proto::FederationDescriptor {
            federation: proto::FixedBytes(self.id_bytes()),
            api_endpoints: self.encoded_endpoints.clone().into(),
            name: Some(self.name.clone()),
            modules: vec![
                module(MINT_INSTANCE, "mintv2"),
                module(LIGHTNING_INSTANCE, "lnv2"),
                module(WALLET_INSTANCE, "walletv2"),
                module(META_INSTANCE, "meta"),
            ],
            mint: Some(proto::MintParams {
                instance: MINT_INSTANCE,
                keys: self.mint.descriptor_keys(),
                fee_base_msat: MINT_FEE.0,
                fee_ppm: MINT_FEE.1,
                amount_unit: 0,
            }),
            lightning: Some(proto::LightningParams {
                instance: LIGHTNING_INSTANCE,
                network: proto::Network::Regtest,
                fee_base_msat: LIGHTNING_FEE.0,
                fee_ppm: LIGHTNING_FEE.1,
            }),
            wallet: Some(proto::WalletParams {
                instance: WALLET_INSTANCE,
                network: proto::Network::Regtest,
                dust_limit_sat: DUST_LIMIT_SAT,
                fee_base_msat: WALLET_FEE.0,
                fee_ppm: WALLET_FEE.1,
            }),
        }
    }

    /// A note this federation issued to `keys` at `(branch, denomination, tweak)`.
    pub fn issue_note(
        &self,
        keys: &FederationKeys,
        branch: Branch,
        denomination: u8,
        tweak: [u8; 16],
    ) -> Note {
        let nonce = keys
            .note_nonce(branch, denomination, &tweak)
            .expect("derivable");
        let blinding = keys.view().blinding_key(branch, denomination, &tweak);
        Note {
            denomination: Denomination(denomination),
            nonce,
            signature: self.mint.issue(denomination, nonce, blinding),
        }
    }
}

/// A tweak that passes the wallet's recovery filter, as a mint v2 client grinds one.
pub fn grind_tweak(view: &ViewKeys, rng: &mut impl RngCore) -> [u8; 16] {
    loop {
        let tweak: [u8; 16] = rng.gen();
        if view.tweak_passes_filter(&tweak) {
            return tweak;
        }
    }
}

pub fn note_input(note: Note) -> DynInput {
    MintInput::new_v0(note).into_dyn(MINT_INSTANCE)
}

/// A mint v2 output issuing a note to `keys`, blinded with fedimint's own `blind_message`.
pub fn note_output(
    keys: &FederationKeys,
    branch: Branch,
    denomination: u8,
    tweak: [u8; 16],
) -> DynOutput {
    let nonce = keys
        .note_nonce(branch, denomination, &tweak)
        .expect("derivable");
    let blinding = tbs::BlindingKey(keys.view().blinding_key(branch, denomination, &tweak));
    let blinded = tbs::blind_message(nonce_message(nonce), blinding);
    MintOutput::new_v0(Denomination(denomination), blinded, tweak).into_dyn(MINT_INSTANCE)
}

/// A mint v2 output issuing a note to somebody else.
pub fn foreign_note_output(rng: &mut impl RngCore, denomination: u8) -> DynOutput {
    let message = tbs::Message::from_bytes_sha256(&rng.gen::<[u8; 32]>());
    let blinded = tbs::blind_message(message, tbs::BlindingKey(random_scalar(rng)));
    MintOutput::new_v0(Denomination(denomination), blinded, rng.gen()).into_dyn(MINT_INSTANCE)
}

pub fn random_public_key(rng: &mut impl RngCore) -> PublicKey {
    let secret = SecretKey::from_slice(&rng.gen::<[u8; 32]>()).expect("valid with overwhelming odds");
    PublicKey::from_secret_key(&Secp256k1::signing_only(), &secret)
}

/// An LNv2 outgoing contract whose refund key is `keys`' (or a stranger's).
pub fn outgoing_contract(
    rng: &mut impl RngCore,
    keys: &FederationKeys,
    payment_hash: [u8; 32],
    amount_msat: u64,
    refund_to_device: bool,
) -> OutgoingContract {
    let ephemeral_pk = random_public_key(rng);
    let refund_pk = if refund_to_device {
        keys.view().refund_key(&ephemeral_pk).expect("derivable")
    } else {
        random_public_key(rng)
    };
    OutgoingContract {
        payment_image: PaymentImage::Hash(sha256::Hash::from_byte_array(payment_hash)),
        amount: Amount::from_msats(amount_msat),
        expiration: 1_000,
        claim_pk: random_public_key(rng),
        refund_pk,
        ephemeral_pk,
    }
}

pub fn contract_output(contract: OutgoingContract) -> DynOutput {
    LightningOutput::V0(LightningOutputV0::Outgoing(contract)).into_dyn(LIGHTNING_INSTANCE)
}

pub fn refund_input(outpoint: OutPoint, witness: OutgoingWitness) -> DynInput {
    LightningInput::V0(LightningInputV0::Outgoing(outpoint, witness)).into_dyn(LIGHTNING_INSTANCE)
}

pub fn pegout_output(destination: StandardScript, value_sat: u64, fee_sat: u64) -> DynOutput {
    WalletOutput::V0(WalletOutputV0 {
        destination,
        value: bitcoin::Amount::from_sat(value_sat),
        fee: bitcoin::Amount::from_sat(fee_sat),
    })
    .into_dyn(WALLET_INSTANCE)
}

/// What the phone sends as `SignRequest::transaction`.
pub fn unsigned_bytes(inputs: &[DynInput], outputs: &[DynOutput], nonce: [u8; 8]) -> Vec<u8> {
    let mut bytes = inputs.to_vec().consensus_encode_to_vec();
    bytes.extend(outputs.to_vec().consensus_encode_to_vec());
    bytes.extend(nonce);
    bytes
}

/// fedimint's transaction id for these parts.
pub fn txid(inputs: &[DynInput], outputs: &[DynOutput], nonce: [u8; 8]) -> TransactionId {
    Transaction::tx_hash_from_parts(inputs, outputs, nonce)
}

/// A regtest BOLT11 invoice signed by a random node.
pub fn invoice(rng: &mut impl RngCore, payment_hash: [u8; 32], amount_msat: u64) -> String {
    let node = SecretKey::from_slice(&rng.gen::<[u8; 32]>()).expect("valid");
    let secp = Secp256k1::new();
    InvoiceBuilder::new(Currency::Regtest)
        .description("coffee".into())
        .payment_hash(sha256::Hash::from_byte_array(payment_hash))
        .payment_secret(PaymentSecret(rng.gen()))
        .duration_since_epoch(std::time::Duration::from_secs(1_800_000_000))
        .min_final_cltv_expiry_delta(144)
        .amount_milli_satoshis(amount_msat)
        .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node))
        .expect("invoice builds")
        .to_string()
}

/// Splits `amount_msat` into change notes of mint v2 client denominations (2^9 and up), each
/// paying its own mint fee, largest first. Returns the denominations and what is left over, which
/// a real client overpays.
pub fn change_denominations(mut amount_msat: u64) -> (Vec<u8>, u64) {
    let mut denominations = Vec::new();
    for denomination in (9..=fps_signer_core::fedimint::mintv2::MAX_DENOMINATION).rev() {
        let cost = (1u64 << denomination) + fee_msat(MINT_FEE, 1u64 << denomination);
        while amount_msat >= cost {
            denominations.push(denomination);
            amount_msat -= cost;
        }
    }
    (denominations, amount_msat)
}

/// A keypair from 32 bytes, for fixtures.
pub fn keypair(bytes: [u8; 32]) -> Keypair {
    Keypair::from_seckey_slice(&Secp256k1::signing_only(), &bytes).expect("valid")
}
