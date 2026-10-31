//! Cashu request and response bodies: the second backend, beside Fedimint.
//!
//! These types are reached through `Request::Cashu` and `Response::Cashu`, which the next change
//! adds. Prime signs Cashu spends the way it signs Fedimint spends: the phone builds every
//! operation and talks to the mint, and Prime checks, shows and signs. The design is
//! `docs/plan/12_CASHU.md`; the key scheme, `fps-cashu-v1`, is `docs/KEY_SCHEME_CASHU.md`.
//!
//! Amounts are in the mint's unit, which this version requires to be `sat`. Proof secrets and the
//! quote id are carried verbatim and never re-serialised: the mint signed `hash_to_curve` of the
//! exact secret bytes, and the NUT-11 message Prime signs embeds them as they are.

use serde::{Deserialize, Serialize};

use crate::bytes::{ByteBuf, FixedBytes};

/// Name of the Cashu key derivation scheme, carried only in Cashu answers. See
/// docs/KEY_SCHEME_CASHU.md.
pub const CASHU_KEY_SCHEME: &str = "fps-cashu-v1";

/// A Cashu mint: the SHA-256 of its canonical URL.
pub type MintId = FixedBytes<32>;

/// What the phone asks Prime to do with Cashu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CashuRequest {
    /// The scheme name and the imported mints. Never needs approval.
    Info,
    /// Pin a mint's keysets, or update them, and get the mint's lock material. Needs approval:
    /// the user confirms the mint's URL and keysets.
    AddMint(CashuMintDescriptor),
    /// Review and sign a SIG_ALL swap or melt. Needs approval.
    Sign(CashuSignRequest),
}

/// One public key of a keyset: the mint signs outputs of `amount` with its private half.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuKey {
    pub amount: u64,
    /// Compressed secp256k1 point.
    pub pubkey: FixedBytes<33>,
}

/// A mint keyset as the mint publishes it (NUT-01, NUT-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuKeyset {
    /// The keyset id's bytes: 8 for a version `00` id, 33 for a version `01` id. Prime derives it
    /// from the keys and refuses a keyset whose id does not match.
    pub id: ByteBuf,
    pub unit: String,
    /// Fee per input from this keyset, in parts per thousand of the unit.
    pub input_fee_ppk: u64,
    /// Unix time after which the mint may stop honouring the keyset.
    pub final_expiry: Option<u64>,
    /// Whether the mint still signs new outputs with this keyset.
    pub active: bool,
    /// One key per amount, in increasing order of amount.
    pub keys: Vec<CashuKey>,
}

/// Everything Prime pins for a mint, read by the phone from the mint's keysets.
///
/// Trusted on first use from the paired phone, as federation configs are (decision D12). Prime
/// checks that every keyset id is derived from its keys, so it pins exactly the keys the ids name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuMintDescriptor {
    /// The canonical mint URL, as CDK's `MintUrl` renders it. Prime refuses any other form, so a
    /// mint always gets the same keys.
    pub url: String,
    /// `sat`, the only unit this version accepts.
    pub unit: String,
    pub keysets: Vec<CashuKeyset>,
}

/// A NUT-12 DLEQ proof that the mint signed a proof with the key for its amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuDleq {
    pub e: FixedBytes<32>,
    pub s: FixedBytes<32>,
    /// The blinding factor the proof was minted with.
    pub r: FixedBytes<32>,
}

/// A proof the request spends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuInput {
    pub amount: u64,
    /// The keyset id's bytes.
    pub keyset: ByteBuf,
    /// The proof's secret, verbatim. Prime accepts only its own 192-character P2PK template.
    pub secret: String,
    /// The unblinded mint signature `C`, compressed.
    pub c: FixedBytes<33>,
    /// Required on every input of a melt, where the signature does not commit to input amounts.
    pub dleq: Option<CashuDleq>,
}

/// Where a change output sits in the `fps-cashu-v1` tree, so a restore finds it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuOutputHint {
    /// The lock key's epoch: the request's input epoch or up to three above it.
    pub epoch: u32,
    /// The keyset id's bytes; the output's own keyset.
    pub keyset: ByteBuf,
    pub counter: u64,
}

/// What an output's blinded message hides, so Prime can recompute `B_ = hash_to_curve(secret) +
/// r·G`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuOpening {
    /// The output's secret, verbatim.
    pub secret: String,
    /// The blinding factor.
    pub r: FixedBytes<32>,
    /// Set for change: Prime re-derives the secret and `r` at this position and refuses any
    /// difference. An output without a hint is leaving the wallet.
    pub hint: Option<CashuOutputHint>,
}

impl std::fmt::Debug for CashuOpening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // With the mint's blind signature, the secret and `r` of a leaving output are the
        // recipient's ecash; never let them reach a log through Debug.
        f.debug_struct("CashuOpening")
            .field("hint", &self.hint)
            .finish_non_exhaustive()
    }
}

/// A blinded message the request asks the mint to sign, with its opening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuOutput {
    /// 0 for a melt's blank outputs, which the mint fills with change.
    pub amount: u64,
    /// The keyset id's bytes.
    pub keyset: ByteBuf,
    /// `B_`, compressed.
    pub b: FixedBytes<33>,
    pub opening: CashuOpening,
}

/// Which request the inputs and outputs belong to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CashuSpendKind {
    /// A NUT-03 swap: sending ecash, combining proofs, or moving them to a new lock key.
    Swap,
    /// A NUT-05 melt paying a BOLT11 invoice through a quote.
    Melt {
        /// The quote id, exactly as the mint issued it.
        quote: String,
        /// The invoice the quote was requested for.
        invoice: String,
        /// The quote's amount, in sats.
        amount: u64,
        /// The quote's fee reserve, in sats.
        fee_reserve: u64,
    },
}

/// A SIG_ALL swap or melt to review and sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuSignRequest {
    pub mint: MintId,
    /// The epoch of the lock key every input is locked to.
    pub epoch: u32,
    /// In the order the phone will post them.
    pub inputs: Vec<CashuInput>,
    /// In the order the phone will post them. Every output carries its opening.
    pub outputs: Vec<CashuOutput>,
    pub kind: CashuSpendKind,
}

/// What Prime answers to a Cashu request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CashuResponse {
    Info(CashuInfo),
    MintAdded(CashuLockMaterial),
    Signed(CashuSigned),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuMintSummary {
    pub mint: MintId,
    pub url: String,
    pub unit: String,
    pub keysets: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuInfo {
    /// Key derivation scheme, `fps-cashu-v1`.
    pub scheme: String,
    pub mints: Vec<CashuMintSummary>,
}

/// Watch-only material for one mint: the lock root and the view key. With them the phone derives
/// every lock key, secret and blinding factor, but no private key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuLockMaterial {
    pub mint: MintId,
    /// Key derivation scheme, `fps-cashu-v1`.
    pub scheme: String,
    /// `P = p·G`, compressed.
    pub lock_pubkey: FixedBytes<33>,
    pub view_key: FixedBytes<32>,
}

impl std::fmt::Debug for CashuLockMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CashuLockMaterial")
            .field("mint", &self.mint)
            .field("scheme", &self.scheme)
            .field("lock_pubkey", &self.lock_pubkey)
            .finish_non_exhaustive()
    }
}

/// A SIG_ALL signature, for the witness of the request's first input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuSigned {
    /// SHA-256 of the NUT-11 message Prime built from the request.
    pub message_hash: FixedBytes<32>,
    /// BIP-340, by the lock key of the request's epoch.
    pub signature: FixedBytes<64>,
}
