//! A self-contained test federation: its own endpoints and mint keys, so tests can issue notes
//! with valid signatures and build any transaction with the signer core's own encoders (which
//! `vectors.rs` proves equal to fedimint's).

#![allow(dead_code)]

use std::collections::BTreeMap;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use bls12_381::{G2Affine, G2Projective, Scalar};
use fps_protocol::{
    Branch, DenominationKey, FederationDescriptor, FixedBytes, InputHint, Intent, LightningParams,
    MintParams, ModuleInfo, Network, SignRequest, WalletParams,
};
use fps_signer_core::encoding::Encode;
use fps_signer_core::fedimint::endpoints::{encode_api_endpoints, federation_id, Guardian};
use fps_signer_core::fedimint::lnv2::{
    LightningInput, LightningOutput, OutgoingContract, OutgoingWitness, PaymentImage,
};
use fps_signer_core::fedimint::mintv2::{MintInput, MintOutput, Note, MAX_DENOMINATION};
use fps_signer_core::fedimint::walletv2::{StandardScript, WalletOutput};
use fps_signer_core::fedimint::{DynItem, OutPoint, UnsignedTransaction};
use fps_signer_core::{tbs, Federation, FederationKeys};
use group::Curve;

pub const MINT: u16 = 0;
pub const LIGHTNING: u16 = 1;
pub const WALLET: u16 = 2;
pub const META: u16 = 3;
pub const MINT_FEE: (u64, u64) = (100, 100);
pub const LIGHTNING_FEE: (u64, u64) = (1_000, 100);
pub const WALLET_FEE: (u64, u64) = (100_000, 100);

pub fn fee((base, ppm): (u64, u64), amount_msat: u64) -> u64 {
    amount_msat * ppm / 1_000_000 + base
}

fn mint_secret(denomination: u8) -> Scalar {
    Scalar::from(1_000_003u64 * (u64::from(denomination) + 1))
}

pub fn mint_key(denomination: u8) -> G2Affine {
    (G2Projective::generator() * mint_secret(denomination)).to_affine()
}

pub fn guardians(count: u16) -> Vec<Guardian> {
    (0..count)
        .map(|peer| Guardian {
            peer,
            url: format!("wss://guardian-{peer}.fixture.example/"),
            name: format!("Fixture {peer}"),
        })
        .collect()
}

pub fn descriptor(guardians: &[Guardian]) -> FederationDescriptor {
    let endpoints = encode_api_endpoints(guardians);
    let module = |instance, kind: &str| ModuleInfo {
        instance,
        kind: kind.to_owned(),
        major: 1,
        minor: 0,
    };
    FederationDescriptor {
        federation: FixedBytes(federation_id(&endpoints)),
        api_endpoints: endpoints.into(),
        name: Some("Fixture Federation".to_owned()),
        modules: vec![
            module(MINT, "mintv2"),
            module(LIGHTNING, "lnv2"),
            module(WALLET, "walletv2"),
            module(META, "meta"),
        ],
        mint: Some(MintParams {
            instance: MINT,
            keys: (0..=MAX_DENOMINATION)
                .map(|denomination| DenominationKey {
                    denomination,
                    key: FixedBytes(mint_key(denomination).to_compressed()),
                })
                .collect(),
            fee_base_msat: MINT_FEE.0,
            fee_ppm: MINT_FEE.1,
            amount_unit: 0,
        }),
        lightning: Some(LightningParams {
            instance: LIGHTNING,
            network: Network::Regtest,
            fee_base_msat: LIGHTNING_FEE.0,
            fee_ppm: LIGHTNING_FEE.1,
        }),
        wallet: Some(WalletParams {
            instance: WALLET,
            network: Network::Regtest,
            dust_limit_sat: 1_000,
            fee_base_msat: WALLET_FEE.0,
            fee_ppm: WALLET_FEE.1,
        }),
    }
}

pub struct Fixture {
    pub app_seed: [u8; 32],
    pub descriptor: FederationDescriptor,
    pub federation: Federation,
    pub keys: FederationKeys,
    counter: u64,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new([0x42; 32])
    }
}

impl Fixture {
    pub fn new(app_seed: [u8; 32]) -> Self {
        let descriptor = descriptor(&guardians(3));
        let federation = Federation::from_descriptor(&descriptor).expect("fixture is valid");
        let keys = FederationKeys::derive(&app_seed, &federation.id).expect("derivable");
        Self {
            app_seed,
            descriptor,
            federation,
            keys,
            counter: 0,
        }
    }

    pub fn id(&self) -> [u8; 32] {
        self.federation.id
    }

    fn next_bytes(&mut self) -> [u8; 32] {
        self.counter += 1;
        sha256::Hash::hash(&self.counter.to_be_bytes()).to_byte_array()
    }

    /// A tweak that passes this wallet's recovery filter. Deterministic.
    pub fn tweak(&mut self) -> [u8; 16] {
        loop {
            let candidate: [u8; 16] = self.next_bytes()[..16].try_into().unwrap();
            if self.keys.view().tweak_passes_filter(&candidate) {
                return candidate;
            }
        }
    }

    /// A tweak that fails the filter.
    pub fn bad_tweak(&mut self) -> [u8; 16] {
        loop {
            let candidate: [u8; 16] = self.next_bytes()[..16].try_into().unwrap();
            if !self.keys.view().tweak_passes_filter(&candidate) {
                return candidate;
            }
        }
    }

    pub fn public_key(&mut self) -> PublicKey {
        let secret = SecretKey::from_slice(&self.next_bytes()).unwrap();
        PublicKey::from_secret_key(&Secp256k1::signing_only(), &secret)
    }

    /// A note of `keys` issued by the fixture mint, with a valid signature.
    pub fn note_for(
        keys: &FederationKeys,
        branch: Branch,
        denomination: u8,
        tweak: [u8; 16],
    ) -> Note {
        let nonce = keys.note_nonce(branch, denomination, &tweak).unwrap();
        let message = tbs::nonce_message(&nonce);
        Note {
            denomination,
            nonce,
            signature: (message * mint_secret(denomination)).to_affine(),
        }
    }

    pub fn note(&self, branch: Branch, denomination: u8, tweak: [u8; 16]) -> Note {
        Self::note_for(&self.keys, branch, denomination, tweak)
    }

    pub fn change_output(&self, branch: Branch, denomination: u8, tweak: [u8; 16]) -> MintOutput {
        let nonce = self.keys.note_nonce(branch, denomination, &tweak).unwrap();
        MintOutput {
            denomination,
            nonce: self
                .keys
                .view()
                .blinded_nonce(branch, &nonce, denomination, &tweak),
            tweak,
        }
    }

    pub fn request(&self, draft: &Draft, intent: Intent) -> SignRequest {
        SignRequest {
            federation: FixedBytes(self.id()),
            transaction: draft.bytes().into(),
            inputs: draft.hints.clone(),
            intent,
        }
    }

    pub fn contract(
        &mut self,
        payment_hash: [u8; 32],
        amount_msat: u64,
        refund_here: bool,
    ) -> OutgoingContract {
        let ephemeral_pk = self.public_key();
        let refund_pk = if refund_here {
            self.keys.view().refund_key(&ephemeral_pk).unwrap()
        } else {
            self.public_key()
        };
        OutgoingContract {
            payment_image: PaymentImage::Hash(payment_hash),
            amount_msat,
            expiration: 500,
            claim_pk: self.public_key(),
            refund_pk,
            ephemeral_pk,
        }
    }
}

/// A transaction under construction with the running balance the federation will check.
#[derive(Default, Clone)]
pub struct Draft {
    pub inputs: Vec<DynItem>,
    pub hints: Vec<InputHint>,
    pub outputs: Vec<DynItem>,
    pub balance: i128,
    pub nonce: [u8; 8],
}

impl Draft {
    pub fn bytes(&self) -> Vec<u8> {
        UnsignedTransaction {
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            nonce: self.nonce,
        }
        .to_bytes()
    }

    pub fn note_in(&mut self, fixture: &mut Fixture, branch: Branch, denomination: u8) -> [u8; 16] {
        let tweak = fixture.tweak();
        let note = fixture.note(branch, denomination, tweak);
        self.raw_note_in(note, tweak, branch);
        tweak
    }

    pub fn raw_note_in(&mut self, note: Note, tweak: [u8; 16], branch: Branch) {
        let amount = 1u64 << note.denomination;
        self.inputs.push(DynItem {
            instance: MINT,
            payload: MintInput { note }.to_bytes(),
        });
        self.hints.push(InputHint::Note {
            tweak: FixedBytes(tweak),
            branch,
        });
        self.balance += i128::from(amount) - i128::from(fee(MINT_FEE, amount));
    }

    pub fn refund_in(
        &mut self,
        contract: &OutgoingContract,
        outpoint: OutPoint,
        witness: OutgoingWitness,
    ) {
        self.inputs.push(DynItem {
            instance: LIGHTNING,
            payload: LightningInput::Outgoing { outpoint, witness }.to_bytes(),
        });
        self.hints.push(InputHint::Refund {
            contract: contract.to_bytes().into(),
        });
        self.balance +=
            i128::from(contract.amount_msat) - i128::from(fee(LIGHTNING_FEE, contract.amount_msat));
    }

    pub fn note_out(
        &mut self,
        fixture: &Fixture,
        branch: Branch,
        denomination: u8,
        tweak: [u8; 16],
    ) {
        let output = fixture.change_output(branch, denomination, tweak);
        self.raw_out(MINT, output.to_bytes(), 1u64 << denomination, MINT_FEE);
    }

    pub fn raw_out(&mut self, instance: u16, payload: Vec<u8>, amount_msat: u64, rule: (u64, u64)) {
        self.outputs.push(DynItem { instance, payload });
        self.balance -= i128::from(amount_msat) + i128::from(fee(rule, amount_msat));
    }

    pub fn contract_out(&mut self, contract: &OutgoingContract) {
        self.raw_out(
            LIGHTNING,
            LightningOutput::Outgoing(*contract).to_bytes(),
            contract.amount_msat,
            LIGHTNING_FEE,
        );
    }

    pub fn pegout_out(&mut self, destination: StandardScript, value_sat: u64, fee_sat: u64) {
        let output = WalletOutput {
            destination,
            value_sat,
            fee_sat,
        };
        self.raw_out(
            WALLET,
            output.to_bytes(),
            output.amount_msat().unwrap(),
            WALLET_FEE,
        );
    }

    /// Fills the remaining balance with change notes of 2^9 msat and up; returns the overpay.
    pub fn change(&mut self, fixture: &mut Fixture) -> u64 {
        assert!(self.balance >= 0, "draft is underfunded");
        let mut left = self.balance as u64;
        for denomination in (9..=MAX_DENOMINATION).rev() {
            let cost = (1u64 << denomination) + fee(MINT_FEE, 1u64 << denomination);
            while left >= cost {
                let tweak = fixture.tweak();
                self.note_out(fixture, Branch::Spend, denomination, tweak);
                left -= cost;
            }
        }
        left
    }
}

/// A regtest invoice for `payment_hash`, signed by a fixed node key.
pub fn invoice(
    payment_hash: [u8; 32],
    amount_msat: Option<u64>,
    currency: lightning_invoice::Currency,
) -> String {
    let node = SecretKey::from_slice(&[0x77; 32]).unwrap();
    let secp = Secp256k1::new();
    let builder = lightning_invoice::InvoiceBuilder::new(currency)
        .description("fixture payment".into())
        .payment_hash(sha256::Hash::from_byte_array(payment_hash))
        .payment_secret(lightning_invoice::PaymentSecret([3; 32]))
        .duration_since_epoch(std::time::Duration::from_secs(1_800_000_000))
        .min_final_cltv_expiry_delta(144);
    let signed = match amount_msat {
        Some(amount) => builder
            .amount_milli_satoshis(amount)
            .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node)),
        None => builder.build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node)),
    };
    signed.unwrap().to_string()
}

/// Ledger fixture.
#[derive(Default)]
pub struct TestLedger {
    pub released: Vec<([u8; 32], u8, [u8; 16])>,
    pub contracts: BTreeMap<([u8; 32], [u8; 32], u64), Vec<u8>>,
}

impl fps_signer_core::Ledger for TestLedger {
    fn is_released(&self, federation: &[u8; 32], denomination: u8, tweak: &[u8; 16]) -> bool {
        self.released.contains(&(*federation, denomination, *tweak))
    }

    fn approved_contract(&self, federation: &[u8; 32], outpoint: &OutPoint) -> Option<Vec<u8>> {
        self.contracts
            .get(&(*federation, outpoint.txid, outpoint.out_idx))
            .cloned()
    }
}
