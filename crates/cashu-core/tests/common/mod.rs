//! A self-contained test mint: its own private keys, so tests can issue proofs with valid
//! signatures and DLEQ proofs and build any swap or melt with the core's own primitives (which
//! `vectors.rs` proves equal to the `cashu` crate's).

#![allow(dead_code)]

use std::collections::BTreeMap;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use fps_cashu_core::keys::MintKeys;
use fps_cashu_core::keyset::{id_v00, id_v01};
use fps_cashu_core::{analyze, dhke, dleq, validate_mint, CashuAnalysis, PinnedMint};
use fps_protocol::{
    ByteBuf, CashuDleq, CashuInput, CashuKey, CashuKeyset, CashuMintDescriptor, CashuOpening,
    CashuOutput, CashuOutputHint, CashuSignRequest, CashuSpendKind, FixedBytes, RejectCode,
};
use fps_signer_core::error::SignerResult;

pub const APP_SEED: [u8; 32] = [0x42; 32];
pub const URL: &str = "https://mint.test.example";

/// Keyset A: version 01, active, 100 ppk, amounts to 2^31. B: version 00, inactive, free. C:
/// version 01, active, free, with a final expiry and every amount to 2^63.
pub const A: usize = 0;
pub const B: usize = 1;
pub const C: usize = 2;

pub fn scalar(tag: &[u8]) -> SecretKey {
    SecretKey::from_slice(&sha256::Hash::hash(tag).to_byte_array()).expect("valid scalar")
}

pub struct TestKeyset {
    pub id: Vec<u8>,
    pub fee: u64,
    pub active: bool,
    pub expiry: Option<u64>,
    pub secrets: BTreeMap<u64, SecretKey>,
}

impl TestKeyset {
    /// Keys for the amounts 2^0 to 2^(bits − 1), from `index`: keysets of different indexes share
    /// no key.
    pub fn new(
        index: u8,
        bits: u32,
        version_01: bool,
        fee: u64,
        active: bool,
        expiry: Option<u64>,
    ) -> Self {
        let secrets: BTreeMap<u64, SecretKey> = (0..bits)
            .map(|bit| {
                let amount = 1u64 << bit;
                let mut tag = b"test mint key ".to_vec();
                tag.push(index);
                tag.extend_from_slice(&amount.to_be_bytes());
                (amount, scalar(&tag))
            })
            .collect();
        let keys: Vec<(u64, PublicKey)> = secrets
            .iter()
            .map(|(amount, secret)| (*amount, secret.public_key(&Secp256k1::new())))
            .collect();
        let id = if version_01 {
            id_v01(&keys, "sat", fee, expiry).to_vec()
        } else {
            id_v00(&keys).to_vec()
        };
        Self {
            id,
            fee,
            active,
            expiry,
            secrets,
        }
    }

    pub fn wire(&self) -> CashuKeyset {
        CashuKeyset {
            id: ByteBuf(self.id.clone()),
            unit: "sat".to_owned(),
            input_fee_ppk: self.fee,
            final_expiry: self.expiry,
            active: self.active,
            keys: self
                .secrets
                .iter()
                .map(|(amount, secret)| CashuKey {
                    amount: *amount,
                    pubkey: FixedBytes(secret.public_key(&Secp256k1::new()).serialize()),
                })
                .collect(),
        }
    }

    pub fn key(&self, amount: u64) -> &SecretKey {
        self.secrets
            .get(&amount)
            .expect("the keyset has this amount")
    }

    /// The public key per amount, in increasing order of amount.
    pub fn public_keys(&self) -> Vec<(u64, PublicKey)> {
        self.secrets
            .iter()
            .map(|(amount, secret)| (*amount, secret.public_key(&Secp256k1::new())))
            .collect()
    }
}

/// A version 01 keyset of these keys, active, with this fee and no final expiry.
pub fn keyset_of(keys: &[(u64, PublicKey)], fee: u64) -> CashuKeyset {
    CashuKeyset {
        id: ByteBuf(id_v01(keys, "sat", fee, None).to_vec()),
        unit: "sat".to_owned(),
        input_fee_ppk: fee,
        final_expiry: None,
        active: true,
        keys: keys
            .iter()
            .map(|(amount, key)| CashuKey {
                amount: *amount,
                pubkey: FixedBytes(key.serialize()),
            })
            .collect(),
    }
}

/// Whether `text` is a point in lowercase hex, compressed or not.
pub fn is_point_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && hex::decode(text).is_ok_and(|bytes| PublicKey::from_slice(&bytes).is_ok())
}

pub struct Fixture {
    pub keys: MintKeys,
    pub keysets: Vec<TestKeyset>,
    pub descriptor: CashuMintDescriptor,
    pub mint: PinnedMint,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Self {
        let keysets = vec![
            TestKeyset::new(0, 32, true, 100, true, None),
            TestKeyset::new(1, 32, false, 0, false, None),
            TestKeyset::new(2, 64, true, 0, true, Some(1_900_000_000)),
        ];
        let descriptor = CashuMintDescriptor {
            url: URL.to_owned(),
            unit: "sat".to_owned(),
            keysets: keysets.iter().map(TestKeyset::wire).collect(),
        };
        let mint = validate_mint(&descriptor, None).expect("the fixture validates");
        Self {
            keys: MintKeys::derive(&APP_SEED, URL).expect("derives"),
            keysets,
            descriptor,
            mint,
        }
    }

    pub fn id(&self, keyset: usize) -> Vec<u8> {
        self.keysets[keyset].id.clone()
    }

    /// A proof of `amount` the mint issued to the wallet's output `(epoch, counter)` of `keyset`,
    /// with its DLEQ proof.
    pub fn proof(&self, keyset: usize, epoch: u32, counter: u64, amount: u64) -> CashuInput {
        let output = self
            .keys
            .phone()
            .blinded_output(epoch, &self.id(keyset), counter)
            .expect("derives");
        self.issue(keyset, amount, &output.secret, &output.r)
    }

    /// A proof the mint issued to `(secret, r)`: `C = a·Y`, and the NUT-12 proof the mint attaches.
    pub fn issue(&self, keyset: usize, amount: u64, secret: &str, r: &SecretKey) -> CashuInput {
        let a = self.keysets[keyset].key(amount);
        let secp = Secp256k1::new();
        let y = dhke::hash_to_curve(secret.as_bytes()).expect("a point");
        let c = y.mul_tweak(&secp, &Scalar::from(*a)).expect("signs");
        CashuInput {
            amount,
            keyset: ByteBuf(self.id(keyset)),
            secret: secret.to_owned(),
            c: FixedBytes(c.serialize()),
            dleq: Some(dleq_proof(secret, r, a)),
        }
    }

    /// Change at the wallet's output `(epoch, counter)` of `keyset`, with its hint.
    pub fn change(&self, keyset: usize, epoch: u32, counter: u64, amount: u64) -> CashuOutput {
        let output = self
            .keys
            .phone()
            .blinded_output(epoch, &self.id(keyset), counter)
            .expect("derives");
        CashuOutput {
            amount,
            keyset: ByteBuf(self.id(keyset)),
            b: FixedBytes(output.b.serialize()),
            opening: CashuOpening {
                secret: output.secret,
                r: FixedBytes(output.r.secret_bytes()),
                hint: Some(CashuOutputHint {
                    epoch,
                    keyset: ByteBuf(self.id(keyset)),
                    counter,
                }),
            },
        }
    }

    /// An output leaving the wallet: a plain 64-hex secret and a blinding factor from `seed`.
    pub fn leaving(&self, keyset: usize, amount: u64, seed: u8) -> CashuOutput {
        let secret = hex::encode(sha256::Hash::hash(&[b'x', seed]).to_byte_array());
        let r = scalar(&[b'r', seed]);
        opened_output(&self.id(keyset), amount, &secret, &r, None)
    }

    pub fn swap(
        &self,
        epoch: u32,
        inputs: Vec<CashuInput>,
        outputs: Vec<CashuOutput>,
    ) -> CashuSignRequest {
        CashuSignRequest {
            mint: FixedBytes(self.mint.id),
            epoch,
            inputs,
            outputs,
            kind: CashuSpendKind::Swap,
        }
    }

    pub fn melt(
        &self,
        epoch: u32,
        inputs: Vec<CashuInput>,
        outputs: Vec<CashuOutput>,
        invoice_msat: Option<u64>,
        amount: u64,
        fee_reserve: u64,
    ) -> CashuSignRequest {
        CashuSignRequest {
            mint: FixedBytes(self.mint.id),
            epoch,
            inputs,
            outputs,
            kind: CashuSpendKind::Melt {
                quote: "0192f4c8-1d2b-7c5a-8e3f-2b7c9d0a1e5f".to_owned(),
                invoice: invoice(invoice_msat),
                amount,
                fee_reserve,
            },
        }
    }

    /// `count` blanks of `keyset` at epoch `epoch`, from counter `first`.
    pub fn blanks(&self, keyset: usize, epoch: u32, first: u64, count: u64) -> Vec<CashuOutput> {
        (first..first + count)
            .map(|counter| self.change(keyset, epoch, counter, 0))
            .collect()
    }

    pub fn analyze(&self, request: &CashuSignRequest) -> SignerResult<CashuAnalysis> {
        analyze(request, &self.mint, &self.keys)
    }

    /// Analyses `request`, expecting a refusal with `code` whose message contains `words`.
    pub fn refuse(&self, request: &CashuSignRequest, code: RejectCode, words: &str) {
        match self.analyze(request) {
            Ok(analysis) => panic!(
                "accepted, expected {code:?} ({words}): {:?}",
                analysis.review
            ),
            Err(error) => {
                assert_eq!(error.code, code, "{}", error.message);
                assert!(
                    error.message.contains(words),
                    "expected {words:?} in {:?}",
                    error.message
                );
            }
        }
    }
}

/// An output with this opening.
pub fn opened_output(
    keyset: &[u8],
    amount: u64,
    secret: &str,
    r: &SecretKey,
    hint: Option<CashuOutputHint>,
) -> CashuOutput {
    let b = dhke::blind(secret.as_bytes(), r).expect("blinds");
    CashuOutput {
        amount,
        keyset: ByteBuf(keyset.to_vec()),
        b: FixedBytes(b.serialize()),
        opening: CashuOpening {
            secret: secret.to_owned(),
            r: FixedBytes(r.secret_bytes()),
            hint,
        },
    }
}

/// The NUT-12 proof the mint attaches when it signs `B' = hash_to_curve(secret) + r·G` with `a`:
/// `R1 = ρ·G`, `R2 = ρ·B'`, `e = hash_e(R1, R2, A, C')`, `s = ρ + e·a`.
pub fn dleq_proof(secret: &str, r: &SecretKey, a: &SecretKey) -> CashuDleq {
    let secp = Secp256k1::new();
    let y = dhke::hash_to_curve(secret.as_bytes()).expect("a point");
    let blinded = y.combine(&r.public_key(&secp)).expect("not infinity");
    let signed = blinded.mul_tweak(&secp, &Scalar::from(*a)).expect("signs");
    let mint_key = a.public_key(&secp);
    for attempt in 0u8.. {
        let mut tag = b"dleq nonce".to_vec();
        tag.extend_from_slice(secret.as_bytes());
        tag.push(attempt);
        let rho = scalar(&tag);
        let r1 = rho.public_key(&secp);
        let r2 = blinded
            .mul_tweak(&secp, &Scalar::from(rho))
            .expect("tweaks");
        let e_bytes = dleq::hash_e(&[r1, r2, mint_key, signed]);
        let Ok(e) = SecretKey::from_slice(&e_bytes) else {
            continue;
        };
        let e_a = a.mul_tweak(&Scalar::from(e)).expect("tweaks");
        let s = rho.add_tweak(&Scalar::from(e_a)).expect("tweaks");
        return CashuDleq {
            e: FixedBytes(e.secret_bytes()),
            s: FixedBytes(s.secret_bytes()),
            r: FixedBytes(r.secret_bytes()),
        };
    }
    unreachable!("a nonce works within 256 attempts")
}

/// A signed BOLT11 invoice on mainnet, as the phone would get from a payee.
pub fn invoice(amount_msat: Option<u64>) -> String {
    let node = SecretKey::from_slice(&[0x77; 32]).expect("valid key");
    let secp = Secp256k1::new();
    let builder = lightning_invoice::InvoiceBuilder::new(lightning_invoice::Currency::Bitcoin)
        .description("coffee".into())
        .payment_hash(sha256::Hash::from_byte_array([5; 32]))
        .payment_secret(lightning_invoice::PaymentSecret([3; 32]))
        .duration_since_epoch(std::time::Duration::from_secs(1_800_000_000))
        .min_final_cltv_expiry_delta(144);
    let signed = match amount_msat {
        Some(amount) => builder
            .amount_milli_satoshis(amount)
            .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node)),
        None => builder.build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node)),
    };
    signed.expect("signs").to_string()
}

/// The standard send: 8 + 4 + 1 from keyset A (3 inputs at 100 ppk: 1 sat fee), 8 leaving and 4
/// of change.
pub fn send(fixture: &Fixture) -> CashuSignRequest {
    fixture.swap(
        0,
        vec![
            fixture.proof(A, 0, 0, 8),
            fixture.proof(A, 0, 1, 4),
            fixture.proof(A, 0, 2, 1),
        ],
        vec![fixture.change(A, 0, 10, 4), fixture.leaving(A, 8, 1)],
    )
}

/// A melt of 100,500 msat: a quote of 101 sats, 2 of fee reserve, 1 of input fee, and two blanks
/// for up to 2 sats of change.
pub fn melt(fixture: &Fixture) -> CashuSignRequest {
    fixture.melt(
        0,
        vec![
            fixture.proof(A, 0, 20, 64),
            fixture.proof(A, 0, 21, 32),
            fixture.proof(A, 0, 22, 8),
        ],
        fixture.blanks(A, 0, 30, 2),
        Some(100_500),
        101,
        2,
    )
}
