//! The wallet the app drives.
//!
//! Every proof the wallet holds is locked by the `fps-cashu-v1` template to `P_0`, the lock key of
//! epoch 0, which only Prime can sign for. Receiving needs no Prime: the phone derives the secrets
//! and blinding factors of its own outputs from the view key `v`. Spending does: the phone builds
//! the SIG_ALL swap or melt, Prime checks it, shows it and signs it, and the phone posts it.
//!
//! One flow runs at a time (they choose proofs and counters); reading balances never waits for one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey as SecpPublicKey, SecretKey as SecpSecretKey, rand};
use cashu::nuts::{
    BlindSignature, BlindedMessage, CurrencyUnit, Id, MeltQuoteState, MeltRequest, MintQuoteState,
    MintRequest, Nut10Secret, P2PKWitness, Proof, PublicKey, SecretKey,
    SpendingConditionVerification, State, SwapRequest, Token, Witness,
};
use cashu::secret::Secret;
use cashu::{Amount, Bolt11Invoice, MintUrl};
use fps_cashu_core::PhoneMintKeys;
use fps_cashu_core::analysis::MAX_INPUTS;
use fps_cashu_core::message::{SigAllMessage, check_one_reading};
use fps_protocol::{
    ByteBuf, CASHU_KEY_SCHEME, CashuDleq, CashuInput, CashuOpening, CashuOutput, CashuOutputHint,
    CashuSignRequest, CashuSigned, CashuSpendKind, FixedBytes,
};

use crate::mint::{MintClient, canonical_url, check_mint_info};
use crate::prime::{CashuPrimeLink, Prime};
use crate::store::{
    SendState, SentProof, Store, StoredActivity, StoredDleq, StoredKeyset, StoredMint, StoredProof,
    StoredQuote, StoredSend,
};
use crate::{CashuError, tls};

/// The lock epoch of every proof and output: rotating keys is not part of this version.
const EPOCH: u32 = 0;
/// How long `finish_top_up` waits for the invoice to be paid.
const TOP_UP_WAIT: Duration = Duration::from_secs(60);
/// How often it asks the mint meanwhile.
const TOP_UP_POLL: Duration = Duration::from_secs(1);
/// Outputs per NUT-09 request when finding where the wallet's outputs at a mint left off.
const RESTORE_BATCH: u64 = 50;
/// How many times a spend derives its outputs again until their message reads only one way.
const MAX_READINGS: usize = 64;

/// A mint the wallet added.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CashuMint {
    pub url: String,
    pub name: Option<String>,
    pub balance_sat: u64,
}

/// A Lightning invoice that tops the wallet up, once paid and finished with
/// [`CashuWallet::finish_top_up`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CashuTopUp {
    pub quote_id: String,
    pub mint_url: String,
    pub invoice: String,
    pub amount_sat: u64,
}

/// Ecash the wallet sent: a `cashuB` token for the recipient, which [`CashuWallet::take_back`]
/// reclaims while nobody has redeemed it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CashuSend {
    pub id: String,
    pub mint_url: String,
    pub amount_sat: u64,
    /// The mint's input fee the send paid.
    pub fee_sat: u64,
    pub token: String,
}

/// A paid Lightning invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CashuPayment {
    /// The invoice's amount.
    pub amount_sat: u64,
    /// Everything spent beyond it: the Lightning fee and the mint's input fee.
    pub fee_sat: u64,
    /// What came back from the fee reserve and the proofs' overshoot.
    pub change_sat: u64,
}

/// One entry of the wallet's history. `kind` is `top_up`, `receive`, `send`, `take_back`, `pay`
/// or `restore`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CashuActivity {
    pub kind: String,
    pub amount_sat: u64,
    pub mint_url: String,
    pub unix_time: u64,
    pub detail: String,
}

/// A watch-only Cashu wallet whose spends Passport Prime signs.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct CashuWallet {
    store: Store,
    prime: Prime,
    flow: tokio::sync::Mutex<()>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn random_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// `amount` as powers of two, smallest first.
fn split(amount: u64) -> Vec<u64> {
    (0..u64::BITS)
        .map(|bit| 1u64 << bit)
        .filter(|part| amount & part != 0)
        .collect()
}

fn corrupt(what: &str) -> CashuError {
    CashuError::failed(format!("the stored wallet has an unreadable {what}"))
}

fn hex32(text: &str) -> Result<[u8; 32], CashuError> {
    hex::decode(text)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| corrupt("scalar"))
}

fn not_added(url: &str) -> CashuError {
    CashuError::invalid(format!("add the mint {url} to this wallet first"))
}

/// A pinned keyset, read.
struct Keyset {
    id: Id,
    bytes: Vec<u8>,
    hex: String,
    fee_ppk: u64,
    active: bool,
    keys: BTreeMap<u64, PublicKey>,
}

impl Keyset {
    fn read(stored: &StoredKeyset) -> Result<Self, CashuError> {
        let bytes = hex::decode(&stored.id).map_err(|_| corrupt("keyset id"))?;
        let id = Id::from_bytes(&bytes).map_err(|_| corrupt("keyset id"))?;
        let keys = stored
            .keys
            .iter()
            .map(|(amount, key)| {
                PublicKey::from_hex(key)
                    .map(|key| (*amount, key))
                    .map_err(|_| corrupt("mint key"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id,
            bytes,
            hex: stored.id.clone(),
            fee_ppk: stored.input_fee_ppk,
            active: stored.active,
            keys,
        })
    }
}

/// What the flows need of a mint the wallet added.
struct MintKeys {
    url: String,
    id: [u8; 32],
    phone: PhoneMintKeys,
    keysets: Vec<Keyset>,
}

impl MintKeys {
    fn read(mint: &StoredMint) -> Result<Self, CashuError> {
        let lock_root = hex::decode(&mint.lock_pubkey)
            .ok()
            .and_then(|bytes| SecpPublicKey::from_slice(&bytes).ok())
            .ok_or_else(|| corrupt("lock key"))?;
        let view_key = hex32(&mint.view_key)?;
        Ok(Self {
            url: mint.url.clone(),
            id: fps_cashu_core::keys::mint_id(&mint.url),
            phone: PhoneMintKeys::new(lock_root, view_key),
            keysets: mint
                .keysets
                .iter()
                .map(Keyset::read)
                .collect::<Result<_, _>>()?,
        })
    }

    fn keyset(&self, hex: &str) -> Option<&Keyset> {
        self.keysets.iter().find(|keyset| keyset.hex == hex)
    }

    /// The keyset new outputs go to: an active one, the cheapest to spend from.
    fn output_keyset(&self) -> Result<&Keyset, CashuError> {
        self.keysets
            .iter()
            .filter(|keyset| keyset.active)
            .min_by_key(|keyset| keyset.fee_ppk)
            .ok_or_else(|| {
                CashuError::invalid(format!(
                    "Prime has pinned no active keyset of the mint {}; add the mint again",
                    self.url
                ))
            })
    }

    /// The input fee of spending proofs of these keysets: `ceil(Σ input_fee_ppk / 1000)`.
    fn fee<'a>(&self, keysets: impl IntoIterator<Item = &'a str>) -> u64 {
        keysets
            .into_iter()
            .map(|hex| self.keyset(hex).map_or(0, |keyset| keyset.fee_ppk))
            .sum::<u64>()
            .div_ceil(1000)
    }

    /// The wallet's own outputs for `amounts`, from counter `next` on, which it advances. A
    /// position whose derivation fails (probability about 2^-128) is skipped.
    fn ours(&self, keyset: &Keyset, amounts: &[u64], next: &mut u64) -> Vec<Output> {
        amounts
            .iter()
            .map(|&amount| {
                loop {
                    let counter = *next;
                    *next += 1;
                    if let Ok(derived) = self.phone.blinded_output(EPOCH, &keyset.bytes, counter) {
                        break Output {
                            amount,
                            keyset: keyset.id,
                            keyset_bytes: keyset.bytes.clone(),
                            secret: derived.secret,
                            r: derived.r,
                            b: derived.b,
                            counter: Some(counter),
                        };
                    }
                }
            })
            .collect()
    }
}

/// Outputs for someone else: plain random secrets, as any wallet can redeem.
fn theirs(keyset: &Keyset, amounts: &[u64]) -> Vec<Output> {
    amounts
        .iter()
        .map(|&amount| {
            loop {
                let secret = hex::encode(rand::random::<[u8; 32]>());
                let r = SecpSecretKey::new(&mut rand::thread_rng());
                if let Some(b) = fps_cashu_core::dhke::blind(secret.as_bytes(), &r) {
                    break Output {
                        amount,
                        keyset: keyset.id,
                        keyset_bytes: keyset.bytes.clone(),
                        secret,
                        r,
                        b,
                        counter: None,
                    };
                }
            }
        })
        .collect()
}

/// An output with its opening.
struct Output {
    amount: u64,
    keyset: Id,
    keyset_bytes: Vec<u8>,
    secret: String,
    r: SecpSecretKey,
    b: SecpPublicKey,
    /// The `fps-cashu-v1` counter of the wallet's own outputs; `None` for someone else's.
    counter: Option<u64>,
}

impl Output {
    fn blinded(&self) -> BlindedMessage {
        BlindedMessage::new(Amount::from(self.amount), self.keyset, self.b.into())
    }

    /// The output as Prime checks it: change carries its hint, so Prime can derive it again.
    fn wire(&self) -> CashuOutput {
        CashuOutput {
            amount: self.amount,
            keyset: ByteBuf(self.keyset_bytes.clone()),
            b: FixedBytes(self.b.serialize()),
            opening: CashuOpening {
                secret: self.secret.clone(),
                r: FixedBytes(self.r.secret_bytes()),
                hint: self.counter.map(|counter| CashuOutputHint {
                    epoch: EPOCH,
                    keyset: ByteBuf(self.keyset_bytes.clone()),
                    counter,
                }),
            },
        }
    }

    /// The proof the mint's signature on this output makes: `C = C_ − r·K`. A blank's amount is
    /// whatever the mint signed it for.
    fn unblind(
        &self,
        signature: &BlindSignature,
        keyset: &Keyset,
        url: &str,
    ) -> Result<StoredProof, CashuError> {
        let amount = u64::from(signature.amount);
        let blank = self.amount == 0;
        if signature.keyset_id != self.keyset || (!blank && amount != self.amount) {
            return Err(CashuError::mint(format!(
                "the mint {url} signed another amount or keyset than the wallet asked for"
            )));
        }
        let key = keyset.keys.get(&amount).ok_or_else(|| {
            CashuError::mint(format!(
                "the mint {url} signed {amount} sats, for which its keyset has no key"
            ))
        })?;
        let c = cashu::dhke::unblind_message(&signature.c, &SecretKey::from(self.r), key)
            .map_err(|error| CashuError::mint(format!("the mint {url}'s signature: {error}")))?;
        Ok(StoredProof {
            mint_url: url.to_owned(),
            epoch: EPOCH,
            keyset: keyset.hex.clone(),
            counter: self.counter,
            amount,
            secret: self.secret.clone(),
            c: c.to_hex(),
            dleq: signature.dleq.as_ref().map(|dleq| StoredDleq {
                e: dleq.e.to_secret_hex(),
                s: dleq.s.to_secret_hex(),
                r: hex::encode(self.r.secret_bytes()),
            }),
        })
    }
}

/// Unblinds the mint's signatures on `outputs`, which must be one for one.
fn unblind_all(
    mint: &MintKeys,
    keyset: &Keyset,
    outputs: &[Output],
    signatures: &[BlindSignature],
) -> Result<Vec<StoredProof>, CashuError> {
    if signatures.len() != outputs.len() {
        return Err(CashuError::mint(format!(
            "the mint {} signed {} of {} outputs",
            mint.url,
            signatures.len(),
            outputs.len()
        )));
    }
    outputs
        .iter()
        .zip(signatures)
        .map(|(output, signature)| output.unblind(signature, keyset, &mint.url))
        .collect()
}

/// Whether the outputs' part of the NUT-11 message splits only one way, as Prime requires.
fn reads_one_way(outputs: &[Output]) -> bool {
    let keys: Vec<[u8; 33]> = outputs.iter().map(|output| output.b.serialize()).collect();
    let pairs: Vec<(u64, &[u8; 33])> = outputs
        .iter()
        .zip(&keys)
        .map(|(output, key)| (output.amount, key))
        .collect();
    check_one_reading(SigAllMessage::build(&[], &pairs, None).output_part()).is_ok()
}

fn proof(stored: &StoredProof) -> Result<Proof, CashuError> {
    let keyset = hex::decode(&stored.keyset)
        .ok()
        .and_then(|bytes| Id::from_bytes(&bytes).ok())
        .ok_or_else(|| corrupt("keyset id"))?;
    Ok(Proof {
        amount: Amount::from(stored.amount),
        keyset_id: keyset,
        secret: Secret::new(stored.secret.clone()),
        c: PublicKey::from_hex(&stored.c).map_err(|_| corrupt("proof"))?,
        witness: None,
        dleq: None,
        p2pk_e: None,
    })
}

/// A proof as Prime checks it; a melt's inputs carry their DLEQ proofs.
fn input(stored: &StoredProof, with_dleq: bool) -> Result<CashuInput, CashuError> {
    let c = hex::decode(&stored.c)
        .ok()
        .and_then(|bytes| <[u8; 33]>::try_from(bytes).ok())
        .ok_or_else(|| corrupt("proof"))?;
    let dleq = match (&stored.dleq, with_dleq) {
        (Some(dleq), true) => Some(CashuDleq {
            e: FixedBytes(hex32(&dleq.e)?),
            s: FixedBytes(hex32(&dleq.s)?),
            r: FixedBytes(hex32(&dleq.r)?),
        }),
        _ => None,
    };
    Ok(CashuInput {
        amount: stored.amount,
        keyset: ByteBuf(hex::decode(&stored.keyset).map_err(|_| corrupt("keyset id"))?),
        secret: stored.secret.clone(),
        c: FixedBytes(c),
        dleq,
    })
}

/// Proofs worth at least `target` plus the input fee they cost, and that fee: the smallest single
/// proof that covers it, or else the largest proofs first.
fn select(
    mint: &MintKeys,
    candidates: &[StoredProof],
    target: u64,
) -> Option<(Vec<StoredProof>, u64)> {
    let single = candidates
        .iter()
        .filter(|proof| proof.amount >= target + mint.fee([proof.keyset.as_str()]))
        .min_by_key(|proof| proof.amount);
    if let Some(proof) = single {
        return Some((vec![proof.clone()], mint.fee([proof.keyset.as_str()])));
    }
    let mut sorted = candidates.to_vec();
    sorted.sort_by_key(|proof| std::cmp::Reverse(proof.amount));
    let mut chosen = Vec::new();
    let mut sum = 0u64;
    for proof in sorted.into_iter().take(MAX_INPUTS) {
        sum += proof.amount;
        chosen.push(proof);
        let fee = mint.fee(chosen.iter().map(|proof| proof.keyset.as_str()));
        if sum >= target + fee {
            return Some((chosen, fee));
        }
    }
    None
}

/// The witness for the first input: Prime's signature, once the message the mint will check is
/// the one Prime signed.
fn witness(signed: &CashuSigned, message: &str) -> Result<Witness, CashuError> {
    if sha256::Hash::hash(message.as_bytes()).to_byte_array() != signed.message_hash.0 {
        return Err(CashuError::prime(
            "Prime signed another message than the one the mint checks",
        ));
    }
    Ok(Witness::P2PKWitness(P2PKWitness {
        signatures: vec![hex::encode(signed.signature.0)],
    }))
}

fn unsigned(error: impl std::fmt::Display) -> CashuError {
    CashuError::prime(format!(
        "Prime's signature does not unlock the inputs: {error}"
    ))
}

impl CashuWallet {
    fn mint_keys(&self, url: &str) -> Result<(StoredMint, MintKeys), CashuError> {
        let mint = self
            .store
            .read(|data| data.mint(url).cloned())
            .ok_or_else(|| not_added(url))?;
        let keys = MintKeys::read(&mint)?;
        Ok((mint, keys))
    }

    /// The wallet's proofs at `url`.
    fn proofs(&self, url: &str) -> Vec<StoredProof> {
        self.store.read(|data| {
            data.proofs
                .iter()
                .filter(|proof| proof.mint_url == url && proof.epoch == EPOCH)
                .cloned()
                .collect()
        })
    }

    fn next_counter(&self, url: &str, keyset: &Keyset) -> u64 {
        self.store.read(|data| {
            data.mint(url)
                .and_then(|mint| mint.counters.get(&keyset.hex))
                .copied()
                .unwrap_or_default()
        })
    }

    /// Saves the counters a request is about to use, before it leaves the phone.
    fn reserve(&self, url: &str, keyset: &Keyset, next: u64) -> Result<(), CashuError> {
        self.store.update(|data| {
            let mint = data.mint_mut(url).ok_or_else(|| not_added(url))?;
            let slot = mint.counters.entry(keyset.hex.clone()).or_default();
            *slot = (*slot).max(next);
            Ok(())
        })
    }

    /// The wallet's own outputs for `amount`, at the next counters, saved before they leave.
    fn own_outputs(
        &self,
        mint: &MintKeys,
        keyset: &Keyset,
        amounts: &[u64],
    ) -> Result<Vec<Output>, CashuError> {
        let mut next = self.next_counter(&mint.url, keyset);
        let outputs = mint.ours(keyset, amounts, &mut next);
        self.reserve(&mint.url, keyset, next)?;
        Ok(outputs)
    }

    /// Outputs for a spend Prime signs: `theirs` for the recipient and the wallet's own for
    /// `ours`, smallest first, derived again at new counters until their message reads only one
    /// way. The counters are saved before the request leaves.
    fn spend_outputs(
        &self,
        mint: &MintKeys,
        keyset: &Keyset,
        theirs_amounts: &[u64],
        ours_amounts: &[u64],
    ) -> Result<Vec<Output>, CashuError> {
        let mut next = self.next_counter(&mint.url, keyset);
        for _ in 0..MAX_READINGS {
            let mut outputs = theirs(keyset, theirs_amounts);
            outputs.extend(mint.ours(keyset, ours_amounts, &mut next));
            outputs.sort_by_key(|output| output.amount);
            if reads_one_way(&outputs) {
                self.reserve(&mint.url, keyset, next)?;
                return Ok(outputs);
            }
        }
        Err(CashuError::failed(
            "no output list for this spend reads one way; try another amount",
        ))
    }

    /// Finds where this Prime's outputs at the mint left off (NUT-09), so no counter is used twice
    /// even if another phone or an earlier install used them, and returns the proofs among them
    /// that are still unspent (NUT-07).
    async fn recover(
        client: &MintClient,
        mint: &MintKeys,
        counters: &mut BTreeMap<String, u64>,
    ) -> Result<Vec<StoredProof>, CashuError> {
        let mut found = Vec::new();
        for keyset in mint.keysets.iter().filter(|keyset| keyset.active) {
            let mut next = counters.get(&keyset.hex).copied().unwrap_or_default();
            let mut start = 0u64;
            loop {
                let mut counter = start;
                let outputs: Vec<Output> = mint
                    .ours(keyset, &[0; RESTORE_BATCH as usize], &mut counter)
                    .into_iter()
                    .collect();
                let answer = client
                    .restore(outputs.iter().map(Output::blinded).collect())
                    .await?;
                if answer.signatures.is_empty() {
                    break;
                }
                for (blinded, signature) in answer.outputs.iter().zip(&answer.signatures) {
                    let Some(output) = outputs
                        .iter()
                        .find(|output| PublicKey::from(output.b) == blinded.blinded_secret)
                    else {
                        continue;
                    };
                    if let Some(counter) = output.counter {
                        next = next.max(counter + 1);
                    }
                    found.push(output.unblind(signature, keyset, &mint.url)?);
                }
                start = counter;
            }
            counters.insert(keyset.hex.clone(), next);
        }
        if found.is_empty() {
            return Ok(found);
        }
        let ys = found
            .iter()
            .map(|proof| cashu::dhke::hash_to_curve(proof.secret.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CashuError::failed)?;
        let states = client.states(ys.clone()).await?;
        Ok(found
            .into_iter()
            .zip(ys)
            .filter(|(_, y)| {
                states
                    .iter()
                    .any(|state| state.y == *y && state.state == State::Unspent)
            })
            .map(|(proof, _)| proof)
            .collect())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl CashuWallet {
    /// Opens the wallet kept in `dir`, reaching Prime through `link`.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub async fn open(dir: String, link: Arc<dyn CashuPrimeLink>) -> Result<Arc<Self>, CashuError> {
        tls::install_tls();
        let store = Store::open(&PathBuf::from(dir))?;
        Ok(Arc::new(Self {
            store,
            prime: Prime::new(link),
            flow: tokio::sync::Mutex::new(()),
        }))
    }

    /// Sends every Prime request to the app in the KeyOS simulator at `address` over the
    /// network, a host or `host:port` (port 7865 by default), instead of through the app's link;
    /// `None` goes back to the app's link.
    pub fn set_simulator_address(&self, address: Option<String>) {
        let address = address
            .map(|address| address.trim().to_owned())
            .filter(|address| !address.is_empty());
        self.prime.set_simulator(address);
    }

    /// The mints the wallet added, with what it holds at each.
    pub async fn mints(&self) -> Result<Vec<CashuMint>, CashuError> {
        Ok(self.store.read(|data| {
            data.mints
                .iter()
                .map(|mint| CashuMint {
                    url: mint.url.clone(),
                    name: mint.name.clone(),
                    balance_sat: data.balance(Some(&mint.url)),
                })
                .collect()
        }))
    }

    /// Adds the mint at `url`, or updates its keys: Prime approves "Cashu: add mint" and pins the
    /// mint's keysets. Proofs this Prime's wallet already holds there come back with it.
    pub async fn add_mint(&self, url: String) -> Result<CashuMint, CashuError> {
        let _flow = self.flow.lock().await;
        let url = canonical_url(&url)?;
        let client = MintClient::new(&url)?;
        let info = client.info().await?;
        check_mint_info(&info)?;
        let descriptor = client.descriptor().await?;
        let material = self.prime.add_mint(descriptor.clone()).await?;
        if material.mint.0 != fps_cashu_core::keys::mint_id(&url)
            || material.scheme != CASHU_KEY_SCHEME
        {
            return Err(CashuError::prime(
                "Prime answered with the keys of another mint or scheme",
            ));
        }
        SecpPublicKey::from_slice(&material.lock_pubkey.0)
            .map_err(|_| CashuError::prime("Prime's lock key is not a valid point"))?;
        let keysets = descriptor
            .keysets
            .iter()
            .map(|keyset| StoredKeyset {
                id: hex::encode(&keyset.id.0),
                input_fee_ppk: keyset.input_fee_ppk,
                final_expiry: keyset.final_expiry,
                active: keyset.active,
                keys: keyset
                    .keys
                    .iter()
                    .map(|key| (key.amount, hex::encode(key.pubkey.0)))
                    .collect(),
            })
            .collect();
        let mut counters = self
            .store
            .read(|data| data.mint(&url).map(|mint| mint.counters.clone()))
            .unwrap_or_default();
        let mut stored = StoredMint {
            url: url.clone(),
            name: info.name.clone(),
            lock_pubkey: hex::encode(material.lock_pubkey.0),
            view_key: hex::encode(material.view_key.0),
            keysets,
            counters: BTreeMap::new(),
        };
        let keys = MintKeys::read(&stored)?;
        let recovered = Self::recover(&client, &keys, &mut counters).await?;
        stored.counters = counters;
        self.store.update(|data| {
            match data.mint_mut(&url) {
                Some(existing) => {
                    // Prime keeps a keyset the mint no longer lists pinned, as inactive; so does
                    // the wallet, for the proofs it may still hold there.
                    let mut updated = stored.clone();
                    for old in &existing.keysets {
                        if updated.keysets.iter().all(|keyset| keyset.id != old.id) {
                            updated.keysets.push(StoredKeyset {
                                active: false,
                                ..old.clone()
                            });
                        }
                    }
                    *existing = updated;
                }
                None => data.mints.push(stored.clone()),
            }
            let new: Vec<StoredProof> = recovered
                .iter()
                .filter(|proof| data.proofs.iter().all(|held| held.secret != proof.secret))
                .cloned()
                .collect();
            let amount: u64 = new.iter().map(|proof| proof.amount).sum();
            if !new.is_empty() {
                data.record(StoredActivity {
                    kind: "restore".to_owned(),
                    amount_sat: amount,
                    mint_url: url.clone(),
                    unix_time: now_unix(),
                    detail: format!("{} proofs this Prime already held", new.len()),
                });
            }
            data.proofs.extend(new);
            Ok(CashuMint {
                url: url.clone(),
                name: stored.name.clone(),
                balance_sat: data.balance(Some(&url)),
            })
        })
    }

    /// What the wallet holds, at every mint.
    pub async fn balance_sat(&self) -> Result<u64, CashuError> {
        Ok(self.store.read(|data| data.balance(None)))
    }

    /// A Lightning invoice for `amount_sat` from the mint. Once it is paid,
    /// [`finish_top_up`](Self::finish_top_up) mints the ecash.
    pub async fn top_up(
        &self,
        mint_url: String,
        amount_sat: u64,
    ) -> Result<CashuTopUp, CashuError> {
        if amount_sat == 0 {
            return Err(CashuError::invalid("top up at least 1 sat"));
        }
        let url = canonical_url(&mint_url)?;
        self.mint_keys(&url)?;
        let quote = MintClient::new(&url)?.mint_quote(amount_sat).await?;
        let top_up = CashuTopUp {
            quote_id: quote.quote.clone(),
            mint_url: url.clone(),
            invoice: quote.request.clone(),
            amount_sat,
        };
        self.store.update(|data| {
            data.quotes.push(StoredQuote {
                id: quote.quote.clone(),
                mint_url: url.clone(),
                amount_sat,
                invoice: quote.request.clone(),
                created_unix: now_unix(),
            });
            Ok(())
        })?;
        Ok(top_up)
    }

    /// Waits up to about a minute for the top-up's invoice to be paid, then mints its ecash into
    /// proofs only Prime can spend. Returns the sats added.
    pub async fn finish_top_up(&self, quote_id: String) -> Result<u64, CashuError> {
        let _flow = self.flow.lock().await;
        let quote = self
            .store
            .read(|data| {
                data.quotes
                    .iter()
                    .find(|quote| quote.id == quote_id)
                    .cloned()
            })
            .ok_or_else(|| CashuError::invalid("this wallet has no such top-up"))?;
        let (_, mint) = self.mint_keys(&quote.mint_url)?;
        let client = MintClient::new(&mint.url)?;
        let deadline = Instant::now() + TOP_UP_WAIT;
        loop {
            match client.mint_quote_state(&quote.id).await?.state {
                MintQuoteState::Paid => break,
                MintQuoteState::Issued => {
                    self.store.update(|data| {
                        data.quotes.retain(|stored| stored.id != quote.id);
                        Ok(())
                    })?;
                    return Err(CashuError::invalid("this top-up was minted already"));
                }
                MintQuoteState::Unpaid if Instant::now() >= deadline => {
                    return Err(CashuError::invalid(
                        "the invoice is not paid yet; finish the top-up once it is",
                    ));
                }
                MintQuoteState::Unpaid => tokio::time::sleep(TOP_UP_POLL).await,
            }
        }
        let keyset = mint.output_keyset()?;
        let outputs = self.own_outputs(&mint, keyset, &split(quote.amount_sat))?;
        let response = client
            .mint(MintRequest {
                quote: quote.id.clone(),
                outputs: outputs.iter().map(Output::blinded).collect(),
                signature: None,
            })
            .await?;
        let proofs = unblind_all(&mint, keyset, &outputs, &response.signatures)?;
        let amount: u64 = proofs.iter().map(|proof| proof.amount).sum();
        self.store.update(|data| {
            data.quotes.retain(|stored| stored.id != quote.id);
            data.proofs.extend(proofs.iter().cloned());
            data.record(StoredActivity {
                kind: "top_up".to_owned(),
                amount_sat: amount,
                mint_url: mint.url.clone(),
                unix_time: now_unix(),
                detail: "Lightning".to_owned(),
            });
            Ok(())
        })?;
        Ok(amount)
    }

    /// Receives a `cashuA` or `cashuB` token of a mint the wallet added: swaps its proofs into
    /// proofs only Prime can spend. Prime is not asked. Returns the sats added, after the mint's
    /// fee.
    pub async fn receive(&self, token: String) -> Result<u64, CashuError> {
        let _flow = self.flow.lock().await;
        let text = token.trim();
        let text = text
            .strip_prefix("cashu:")
            .or_else(|| text.strip_prefix("CASHU:"))
            .unwrap_or(text);
        let token = Token::from_str(text)
            .map_err(|error| CashuError::invalid(format!("not a Cashu token: {error}")))?;
        if token.unit().is_some_and(|unit| unit != CurrencyUnit::Sat) {
            return Err(CashuError::invalid("only tokens in sats can be received"));
        }
        let url = token
            .mint_url()
            .map_err(|error| CashuError::invalid(format!("the token's mint: {error}")))?;
        let url = canonical_url(&url.to_string())?;
        let (_, mint) = self.mint_keys(&url)?;
        let client = MintClient::new(&url)?;
        let listed = client.keysets().await?;
        let proofs = token
            .proofs(&listed)
            .map_err(|error| CashuError::invalid(format!("the token's proofs: {error}")))?;
        if proofs.is_empty() {
            return Err(CashuError::invalid("the token holds no ecash"));
        }
        if proofs
            .iter()
            .any(|proof| Nut10Secret::try_from(&proof.secret).is_ok())
        {
            return Err(CashuError::invalid(
                "this token is locked (P2PK or HTLC); only plain tokens can be received",
            ));
        }
        let mut total = 0u64;
        let mut fee_ppk = 0u64;
        for proof in &proofs {
            let keyset = listed
                .iter()
                .find(|keyset| keyset.id == proof.keyset_id)
                .ok_or_else(|| {
                    CashuError::invalid(format!(
                        "the token names keyset {}, which the mint does not list",
                        proof.keyset_id
                    ))
                })?;
            if keyset.unit != CurrencyUnit::Sat {
                return Err(CashuError::invalid("only tokens in sats can be received"));
            }
            total += u64::from(proof.amount);
            fee_ppk += keyset.input_fee_ppk;
        }
        let fee = fee_ppk.div_ceil(1000);
        let net = total
            .checked_sub(fee)
            .filter(|net| *net > 0)
            .ok_or_else(|| CashuError::invalid("the token is worth no more than the mint's fee"))?;
        let keyset = mint.output_keyset()?;
        let outputs = self.own_outputs(&mint, keyset, &split(net))?;
        let secrets: Vec<String> = proofs
            .iter()
            .map(|proof| proof.secret.to_string())
            .collect();
        let request = SwapRequest::new(proofs, outputs.iter().map(Output::blinded).collect());
        let response = client.swap(request).await.map_err(|error| match error {
            cdk::Error::TokenAlreadySpent => CashuError::invalid("this token was redeemed already"),
            other => CashuError::mint(format!("Receiving at the mint {url} failed: {other}")),
        })?;
        let received = unblind_all(&mint, keyset, &outputs, &response.signatures)?;
        self.store.update(|data| {
            // A token this wallet sent and now receives itself is back: its send is done.
            for send in &mut data.sends {
                let ours = send
                    .proofs
                    .iter()
                    .any(|sent| secrets.contains(&sent.secret));
                if send.state == SendState::Pending && ours {
                    send.state = SendState::TakenBack;
                }
            }
            data.proofs.extend(received.iter().cloned());
            data.record(StoredActivity {
                kind: "receive".to_owned(),
                amount_sat: net,
                mint_url: url.clone(),
                unix_time: now_unix(),
                detail: format!("fee {fee} sat"),
            });
            Ok(())
        })?;
        Ok(net)
    }

    /// Sends `amount_sat` from the mint as a `cashuB` token: a SIG_ALL swap Prime approves as
    /// "Cashu: send ecash", into the recipient's plain proofs and the wallet's change.
    pub async fn send(&self, mint_url: String, amount_sat: u64) -> Result<CashuSend, CashuError> {
        let _flow = self.flow.lock().await;
        if amount_sat == 0 {
            return Err(CashuError::invalid("send at least 1 sat"));
        }
        let url = canonical_url(&mint_url)?;
        let (_, mint) = self.mint_keys(&url)?;
        let held = self.proofs(&url);
        let (inputs, fee) = select(&mint, &held, amount_sat).ok_or_else(|| {
            CashuError::invalid(format!(
                "not enough ecash at {url}: {} sats held, {amount_sat} and the mint's fee needed",
                held.iter().map(|proof| proof.amount).sum::<u64>()
            ))
        })?;
        let total: u64 = inputs.iter().map(|proof| proof.amount).sum();
        let change = total - amount_sat - fee;
        let keyset = mint.output_keyset()?;
        let outputs = self.spend_outputs(&mint, keyset, &split(amount_sat), &split(change))?;
        let signed = self
            .prime
            .sign(CashuSignRequest {
                mint: FixedBytes(mint.id),
                epoch: EPOCH,
                inputs: inputs
                    .iter()
                    .map(|proof| input(proof, false))
                    .collect::<Result<_, _>>()?,
                outputs: outputs.iter().map(Output::wire).collect(),
                kind: CashuSpendKind::Swap,
            })
            .await?;
        let mut request = SwapRequest::new(
            inputs.iter().map(proof).collect::<Result<_, _>>()?,
            outputs.iter().map(Output::blinded).collect(),
        );
        let witness = witness(&signed, &request.sig_all_msg_to_sign())?;
        if let Some(first) = request.inputs_mut().first_mut() {
            first.witness = Some(witness);
        }
        request.verify_spending_conditions().map_err(unsigned)?;
        let response = MintClient::new(&url)?
            .swap(request)
            .await
            .map_err(|error| {
                CashuError::mint(format!("Sending at the mint {url} failed: {error}"))
            })?;
        let made = unblind_all(&mint, keyset, &outputs, &response.signatures)?;
        let mut given = Vec::new();
        let mut sent = Vec::new();
        let mut kept = Vec::new();
        for (output, made) in outputs.iter().zip(made) {
            if output.counter.is_some() {
                kept.push(made);
                continue;
            }
            given.push(proof(&made)?);
            sent.push(SentProof {
                keyset: made.keyset.clone(),
                amount: made.amount,
                secret: made.secret.clone(),
                r: hex::encode(output.r.secret_bytes()),
                c: made.c.clone(),
            });
        }
        let mint_url = MintUrl::from_str(&url).map_err(CashuError::failed)?;
        let token = Token::new(mint_url, given, None, CurrencyUnit::Sat).to_string();
        let send = CashuSend {
            id: random_id(),
            mint_url: url.clone(),
            amount_sat,
            fee_sat: fee,
            token: token.clone(),
        };
        let spent: Vec<&str> = inputs.iter().map(|proof| proof.secret.as_str()).collect();
        self.store.update(|data| {
            data.proofs
                .retain(|proof| !spent.contains(&proof.secret.as_str()));
            data.proofs.extend(kept.iter().cloned());
            data.sends.push(StoredSend {
                id: send.id.clone(),
                mint_url: url.clone(),
                amount_sat,
                fee_sat: fee,
                token: token.clone(),
                proofs: sent.clone(),
                created_unix: now_unix(),
                state: SendState::Pending,
            });
            data.record(StoredActivity {
                kind: "send".to_owned(),
                amount_sat,
                mint_url: url.clone(),
                unix_time: now_unix(),
                detail: format!("fee {fee} sat"),
            });
            Ok(())
        })?;
        Ok(send)
    }

    /// Takes back a send nobody has redeemed: swaps its proofs back into the wallet's own. Prime
    /// is not asked. Fails if the recipient redeemed it already. Returns the sats back, after the
    /// mint's fee.
    pub async fn take_back(&self, send_id: String) -> Result<u64, CashuError> {
        let _flow = self.flow.lock().await;
        let send = self
            .store
            .read(|data| data.sends.iter().find(|send| send.id == send_id).cloned())
            .ok_or_else(|| CashuError::invalid("this wallet has no such send"))?;
        match send.state {
            SendState::Pending => {}
            SendState::TakenBack => {
                return Err(CashuError::invalid("this send was taken back already"));
            }
            SendState::Redeemed => {
                return Err(CashuError::invalid(
                    "the recipient redeemed this send already",
                ));
            }
        }
        let (_, mint) = self.mint_keys(&send.mint_url)?;
        let url = mint.url.clone();
        let proofs = send
            .proofs
            .iter()
            .map(|sent| {
                proof(&StoredProof {
                    mint_url: url.clone(),
                    epoch: EPOCH,
                    keyset: sent.keyset.clone(),
                    counter: None,
                    amount: sent.amount,
                    secret: sent.secret.clone(),
                    c: sent.c.clone(),
                    dleq: None,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fee = mint.fee(send.proofs.iter().map(|sent| sent.keyset.as_str()));
        let net = send
            .amount_sat
            .checked_sub(fee)
            .filter(|net| *net > 0)
            .ok_or_else(|| CashuError::invalid("the send is worth no more than the mint's fee"))?;
        let keyset = mint.output_keyset()?;
        let outputs = self.own_outputs(&mint, keyset, &split(net))?;
        let client = MintClient::new(&url)?;
        let ys: Vec<PublicKey> = proofs
            .iter()
            .map(|proof| cashu::dhke::hash_to_curve(proof.secret.as_bytes()))
            .collect::<Result<_, _>>()
            .map_err(CashuError::failed)?;
        let request = SwapRequest::new(proofs, outputs.iter().map(Output::blinded).collect());
        let response = match client.swap(request).await {
            Ok(response) => response,
            Err(error) => {
                let redeemed = matches!(error, cdk::Error::TokenAlreadySpent)
                    || client
                        .states(ys)
                        .await
                        .is_ok_and(|states| states.iter().any(|state| state.state == State::Spent));
                if redeemed {
                    self.store.update(|data| {
                        if let Some(stored) =
                            data.sends.iter_mut().find(|stored| stored.id == send.id)
                        {
                            stored.state = SendState::Redeemed;
                        }
                        Ok(())
                    })?;
                    return Err(CashuError::mint(
                        "the recipient redeemed this send already; there is nothing to take back",
                    ));
                }
                return Err(CashuError::mint(format!(
                    "Taking the send back at the mint {url} failed: {error}"
                )));
            }
        };
        let back = unblind_all(&mint, keyset, &outputs, &response.signatures)?;
        self.store.update(|data| {
            if let Some(stored) = data.sends.iter_mut().find(|stored| stored.id == send.id) {
                stored.state = SendState::TakenBack;
            }
            data.proofs.extend(back.iter().cloned());
            data.record(StoredActivity {
                kind: "take_back".to_owned(),
                amount_sat: net,
                mint_url: url.clone(),
                unix_time: now_unix(),
                detail: format!("fee {fee} sat"),
            });
            Ok(())
        })?;
        Ok(net)
    }

    /// Pays a BOLT11 invoice from the mint: a melt quote, then a SIG_ALL melt Prime approves as
    /// "Cashu: pay Lightning invoice", with blank outputs for the change.
    pub async fn pay(&self, mint_url: String, invoice: String) -> Result<CashuPayment, CashuError> {
        let _flow = self.flow.lock().await;
        let url = canonical_url(&mint_url)?;
        let (_, mint) = self.mint_keys(&url)?;
        let text = invoice.trim();
        let text = text
            .strip_prefix("lightning:")
            .or_else(|| text.strip_prefix("LIGHTNING:"))
            .unwrap_or(text);
        let bolt11 = Bolt11Invoice::from_str(text)
            .map_err(|error| CashuError::invalid(format!("not a Lightning invoice: {error}")))?;
        if bolt11.amount_milli_satoshis().is_none() {
            return Err(CashuError::invalid(
                "invoices without an amount are not supported",
            ));
        }
        let client = MintClient::new(&url)?;
        let quote = client.melt_quote(bolt11).await?;
        let amount = u64::from(quote.amount);
        let reserve = u64::from(quote.fee_reserve);
        // Prime checks every input of a melt against its DLEQ proof.
        let held: Vec<StoredProof> = self
            .proofs(&url)
            .into_iter()
            .filter(|proof| proof.dleq.is_some())
            .collect();
        let (inputs, fee) = select(&mint, &held, amount + reserve).ok_or_else(|| {
            CashuError::invalid(format!(
                "not enough ecash at {url}: {} sats held, {amount} plus a fee reserve of {reserve} and the mint's fee needed",
                held.iter().map(|proof| proof.amount).sum::<u64>()
            ))
        })?;
        let total: u64 = inputs.iter().map(|proof| proof.amount).sum();
        // Enough blanks for all the mint might return.
        let returnable = total - amount - fee;
        let blanks = vec![0u64; (u64::BITS - returnable.leading_zeros()) as usize];
        let keyset = mint.output_keyset()?;
        let outputs = self.spend_outputs(&mint, keyset, &[], &blanks)?;
        let signed = self
            .prime
            .sign(CashuSignRequest {
                mint: FixedBytes(mint.id),
                epoch: EPOCH,
                inputs: inputs
                    .iter()
                    .map(|proof| input(proof, true))
                    .collect::<Result<_, _>>()?,
                outputs: outputs.iter().map(Output::wire).collect(),
                kind: CashuSpendKind::Melt {
                    quote: quote.quote.clone(),
                    invoice: text.to_owned(),
                    amount,
                    fee_reserve: reserve,
                },
            })
            .await?;
        let blinded: Vec<BlindedMessage> = outputs.iter().map(Output::blinded).collect();
        let mut request = MeltRequest::new(
            quote.quote.clone(),
            inputs.iter().map(proof).collect::<Result<_, _>>()?,
            (!blinded.is_empty()).then_some(blinded),
        );
        let witness = witness(&signed, &request.sig_all_msg_to_sign())?;
        if let Some(first) = request.inputs_mut().first_mut() {
            first.witness = Some(witness);
        }
        request.verify_spending_conditions().map_err(unsigned)?;
        let answer = client.melt(request).await?;
        match answer.state {
            MeltQuoteState::Paid => {}
            MeltQuoteState::Pending => {
                return Err(CashuError::mint(format!(
                    "the payment is still pending at the mint {url}"
                )));
            }
            state => {
                return Err(CashuError::mint(format!(
                    "the mint {url} did not pay the invoice ({state})"
                )));
            }
        }
        let signatures = answer.change.unwrap_or_default();
        if signatures.len() > outputs.len() {
            return Err(CashuError::mint(format!(
                "the mint {url} returned more change than the wallet left room for"
            )));
        }
        let change = outputs
            .iter()
            .zip(&signatures)
            .map(|(output, signature)| output.unblind(signature, keyset, &url))
            .collect::<Result<Vec<_>, _>>()?;
        let change_sat: u64 = change.iter().map(|proof| proof.amount).sum();
        let fee_sat = total.saturating_sub(amount).saturating_sub(change_sat);
        let spent: Vec<&str> = inputs.iter().map(|proof| proof.secret.as_str()).collect();
        self.store.update(|data| {
            data.proofs
                .retain(|proof| !spent.contains(&proof.secret.as_str()));
            data.proofs.extend(change.iter().cloned());
            data.record(StoredActivity {
                kind: "pay".to_owned(),
                amount_sat: amount,
                mint_url: url.clone(),
                unix_time: now_unix(),
                detail: format!("fee {fee_sat} sat"),
            });
            Ok(())
        })?;
        Ok(CashuPayment {
            amount_sat: amount,
            fee_sat,
            change_sat,
        })
    }

    /// The sends nobody is known to have redeemed and the wallet has not taken back, newest
    /// first.
    pub async fn sends(&self) -> Result<Vec<CashuSend>, CashuError> {
        Ok(self.store.read(|data| {
            data.sends
                .iter()
                .rev()
                .filter(|send| send.state == SendState::Pending)
                .map(|send| CashuSend {
                    id: send.id.clone(),
                    mint_url: send.mint_url.clone(),
                    amount_sat: send.amount_sat,
                    fee_sat: send.fee_sat,
                    token: send.token.clone(),
                })
                .collect()
        }))
    }

    /// The wallet's history, newest first.
    pub async fn activity(&self) -> Result<Vec<CashuActivity>, CashuError> {
        Ok(self.store.read(|data| {
            data.activity
                .iter()
                .rev()
                .map(|entry| CashuActivity {
                    kind: entry.kind.clone(),
                    amount_sat: entry.amount_sat,
                    mint_url: entry.mint_url.clone(),
                    unix_time: entry.unix_time,
                    detail: entry.detail.clone(),
                })
                .collect()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_split_into_powers_of_two() {
        assert_eq!(split(0), Vec::<u64>::new());
        assert_eq!(split(1), [1]);
        assert_eq!(split(191), [1, 2, 4, 8, 16, 32, 128]);
        assert_eq!(split(256), [256]);
    }
}
