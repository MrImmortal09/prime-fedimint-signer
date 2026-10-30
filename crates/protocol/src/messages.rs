//! Request and response bodies.
//!
//! Every byte string that carries fedimint data (`transaction`, `api_endpoints`, `contract`) is the
//! exact fedimint consensus encoding, so the device decodes the same bytes the federation will see.
//! Everything else is plain CBOR through serde.

use serde::{Deserialize, Serialize};

use crate::bytes::{ByteBuf, FixedBytes};

/// A 16-byte random id chosen by the phone. Responses echo it.
pub type RequestId = FixedBytes<16>;
/// A fedimint federation id: the SHA-256 of the guardian API endpoints.
pub type FederationId = FixedBytes<32>;
/// A mint v2 note tweak: 16 public bytes published in every note output.
pub type Tweak = FixedBytes<16>;

/// What the phone asks the device to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Request {
    /// Protocol version, key scheme and imported federations. Never needs approval.
    GetInfo,
    /// Remember a federation. Needs approval: the user confirms its name and guardians.
    ImportFederation(FederationDescriptor),
    /// The watch-only key material for one federation. Needs approval: it lets the phone see every
    /// note, though not spend any.
    ExportViewMaterial { federation: FederationId },
    /// Public nonces of hardened outgoing-branch notes, which the phone needs to build the outputs
    /// of a send. Never needs approval: the result is public keys only.
    DeriveOutgoingNonces {
        federation: FederationId,
        notes: Vec<NoteRef>,
    },
    /// Review and sign a fedimint transaction. Needs approval.
    SignTransaction(SignRequest),
    /// Release outgoing-branch notes as an ecash token. Needs approval unless the notes were staged
    /// by an approved `PrepareSend` and the policy allows releasing them without a second tap.
    ReleaseNotes(ReleaseRequest),
    /// The stored response of an earlier request, for transports where the reply is fetched later.
    GetResult { request: RequestId },
    /// Released notes, recent requests and policy counters, for the phone to reconcile on connect.
    GetStatus,
    /// Abandon a request that is waiting for approval.
    Cancel { request: RequestId },
}

/// A mint v2 note position: its denomination exponent and its tweak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteRef {
    /// Exponent: the note is worth `2^denomination` msat.
    pub denomination: u8,
    pub tweak: Tweak,
}

/// Everything the device needs to know about a federation, extracted by the phone from the
/// federation's client config.
///
/// Only `api_endpoints` is authenticated: the device checks that its SHA-256 equals `federation`.
/// The rest is trusted on import from the paired phone (decision D12); none of the checks that
/// protect funds depend on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederationDescriptor {
    pub federation: FederationId,
    /// Consensus encoding of the client config's `api_endpoints: BTreeMap<PeerId, PeerUrl>`.
    pub api_endpoints: ByteBuf,
    /// The `federation_name` meta field, if the federation sets one.
    pub name: Option<String>,
    /// Every module instance in the config, including ones the device does not understand.
    pub modules: Vec<ModuleInfo>,
    pub mint: Option<MintParams>,
    pub lightning: Option<LightningParams>,
    pub wallet: Option<WalletParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleInfo {
    pub instance: u16,
    pub kind: String,
    pub major: u32,
    pub minor: u32,
}

/// The `mintv2` module's client config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MintParams {
    pub instance: u16,
    /// Aggregate public key per denomination (`tbs_agg_pks`), G2 points in compressed form.
    pub keys: Vec<DenominationKey>,
    pub fee_base_msat: u64,
    pub fee_ppm: u64,
    /// fedimint `AmountUnit`; 0 is bitcoin, the only unit this version accepts.
    pub amount_unit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenominationKey {
    pub denomination: u8,
    pub key: FixedBytes<96>,
}

/// The `lnv2` module's client config, minus the threshold encryption keys the device never uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightningParams {
    pub instance: u16,
    pub network: Network,
    pub fee_base_msat: u64,
    pub fee_ppm: u64,
}

/// The `walletv2` module's client config, minus the multisig keys the device never uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalletParams {
    pub instance: u16,
    pub network: Network,
    pub dust_limit_sat: u64,
    pub fee_base_msat: u64,
    pub fee_ppm: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Network {
    Bitcoin,
    Testnet,
    Testnet4,
    Signet,
    Regtest,
}

/// A fedimint transaction to review and sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignRequest {
    pub federation: FederationId,
    /// `encode(inputs) || encode(outputs) || nonce`: exactly the bytes fedimint hashes into the
    /// transaction id. The device decodes it, re-encodes it, and refuses any difference.
    pub transaction: ByteBuf,
    /// How to find the key for each input, in input order.
    pub inputs: Vec<InputHint>,
    /// What the user is doing. The device checks the transaction matches it.
    pub intent: Intent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum InputHint {
    /// A mint v2 note. The denomination and nonce are in the input; the tweak is not.
    Note { tweak: Tweak, branch: Branch },
    /// An LNv2 outgoing contract being refunded or cancelled; the consensus-encoded contract.
    Refund { contract: ByteBuf },
}

/// Which key tree a note belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Branch {
    /// Non-hardened, derivable by the phone from the view material. Never released.
    Spend,
    /// Hardened, device-only. The only keys ever released in a token.
    Outgoing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Intent {
    /// Pay a BOLT11 invoice through an LNv2 outgoing contract.
    PayInvoice { invoice: String },
    /// Withdraw on chain through a wallet v2 output.
    Withdraw,
    /// Move notes to the outgoing branch so `amount_msat` can then be released as ecash.
    PrepareSend { amount_msat: u64 },
    /// Self-transfer: consolidate or re-split notes.
    Reissue,
    /// Spend released but unredeemed outgoing notes back to the wallet.
    Reclaim,
    /// Refund a failed or cancelled Lightning payment back to the wallet.
    Refund,
}

/// Release outgoing-branch notes as a fedimint mint v2 ecash token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRequest {
    pub federation: FederationId,
    pub notes: Vec<ReleaseNote>,
    /// Add an invite code so a receiver who has not joined the federation can redeem. The device
    /// takes the guardian URL from its own imported endpoints, not from the phone.
    pub include_invite: bool,
    pub delivery: Delivery,
}

/// An issued outgoing-branch note: its position and the unblinded mint signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseNote {
    pub denomination: u8,
    pub tweak: Tweak,
    pub signature: FixedBytes<48>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    /// The device shows the token as a QR code; the phone never sees the keys.
    ShowOnDevice,
    /// The token goes back to the phone, which can send it over any channel.
    ReturnToPhone,
}

/// What the device answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Response {
    Info(DeviceInfo),
    FederationImported(FederationSummary),
    ViewMaterial(ViewMaterial),
    OutgoingNonces {
        nonces: Vec<FixedBytes<33>>,
    },
    Signed(SignedTransaction),
    Released(Released),
    Status(StatusInfo),
    /// The request is waiting for the user on the device.
    Pending,
    Cancelled,
    Rejected(Rejection),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub protocol: u16,
    /// Key derivation scheme, `fps-v1`. See docs/KEY_SCHEME.md.
    pub key_scheme: String,
    pub federations: Vec<FederationSummary>,
    /// A request is waiting for approval.
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederationSummary {
    pub federation: FederationId,
    pub name: Option<String>,
    pub guardians: u16,
    pub modules: Vec<String>,
}

/// Watch-only material for one federation: the spend xpub and the view secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewMaterial {
    pub federation: FederationId,
    pub key_scheme: String,
    /// `K = k·G`, compressed.
    pub spend_key: FixedBytes<33>,
    pub chain_code: FixedBytes<32>,
    pub view_secret: FixedBytes<32>,
}

impl std::fmt::Debug for ViewMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewMaterial")
            .field("federation", &self.federation)
            .field("key_scheme", &self.key_scheme)
            .field("spend_key", &self.spend_key)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedTransaction {
    pub txid: FixedBytes<32>,
    /// One BIP-340 signature per input, in input order.
    pub signatures: Vec<FixedBytes<64>>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Released {
    pub amount_msat: u64,
    pub notes: u32,
    /// The `fedimint…` token, or `None` when it was shown on the device only.
    pub token: Option<String>,
}

impl std::fmt::Debug for Released {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is the money; never let it reach a log through Debug.
        f.debug_struct("Released")
            .field("amount_msat", &self.amount_msat)
            .field("notes", &self.notes)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusInfo {
    pub released: Vec<ReleasedNote>,
    pub recent: Vec<RequestId>,
    pub outflow_today_msat: u64,
    pub pending: Option<RequestId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedNote {
    pub federation: FederationId,
    pub note: NoteRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    pub code: RejectCode,
    pub message: String,
}

/// Why the device refused. Stable across versions so the phone can branch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RejectCode {
    /// The message is not valid protocol CBOR, or too large.
    Malformed,
    UnsupportedVersion,
    UnknownFederation,
    /// The descriptor's endpoints do not hash to its federation id.
    FederationIdMismatch,
    /// The descriptor is inconsistent or uses something this version does not support.
    InvalidDescriptor,
    /// The fedimint bytes do not decode, or are not in canonical form.
    InvalidTransaction,
    /// A module, module version or variant this version does not support.
    Unsupported,
    /// An input is not the device's own.
    ForeignInput,
    /// A note's mint signature does not verify.
    InvalidNote,
    /// An output is not allowed for the stated intent.
    UnexpectedOutput,
    /// The transaction does not match the stated intent.
    IntentMismatch,
    /// The Lightning contract does not match the invoice.
    InvoiceMismatch,
    /// The note was already released, or is staged for a different purpose.
    AlreadyReleased,
    /// Over a policy limit.
    PolicyLimit,
    UserRejected,
    /// Another request is waiting for approval.
    Busy,
    /// No stored result for that request id.
    NotFound,
    TooLarge,
    Internal,
}
