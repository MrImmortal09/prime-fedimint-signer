//! The wallet the app drives.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bitcoin::Address;
use bitcoin::address::NetworkUnchecked;
use fedimint_api_client::download_from_invite_code;
use fedimint_client::error::ClientBuildError;
use fedimint_client::module_init::ClientModuleInitRegistry;
use fedimint_client::secret::{PlainRootSecretStrategy, RootSecretStrategy};
use fedimint_client::{Client, ClientBuilder, ClientHandleArc, RootSecret};
use fedimint_connectors::ConnectorRegistry;
use fedimint_core::Amount;
use fedimint_core::base32::{self, FEDIMINT_PREFIX};
use fedimint_core::config::ClientConfig;
use fedimint_core::core::OperationId;
use fedimint_core::db::Database;
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::invite_code::InviteCode;
use fedimint_core::module::registry::ModuleDecoderRegistry;
use fedimint_core::util::SafeUrl;
use fedimint_cursed_redb::MemAndRedb;
use fedimint_lnv2_client::{LightningClientInit, LightningClientModule};
use fedimint_lnv2_common::Bolt11InvoiceDescription;
use fedimint_mintv2_client::{
    ECash, FinalReceiveOperationState, IssuedExternalNote, MintClientInitWithKeys, MintClientModule,
    MintOperationMeta, ReceiveECashError,
};
use fedimint_walletv2_client::{WalletClientInit, WalletClientModule};
use fps_protocol::{Delivery, FixedBytes, KEY_SCHEME, RejectCode, ViewMaterial};
use fps_signer_core::ViewKeys;
use fps_wallet::DeviceKeys;
use fps_wallet::device::{Device, DeviceError};
use fps_wallet::send::SendError;
use lightning_invoice::Bolt11Invoice;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::link::RoutedLink;
use crate::store::{Store, StoredSend};
use crate::{PrimeLink, WalletError};

/// How long a Lightning invoice the wallet issues stays payable.
const INVOICE_EXPIRY_SECS: u32 = 60 * 60;
/// How many operations `activity` lists.
const ACTIVITY_LIMIT: usize = 50;
/// How long a receive or reclaim waits for its change notes, so the balance it leaves is current.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);

/// A federation the wallet belongs to.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FederationSummary {
    /// The federation id, in hex.
    pub id: String,
    pub name: Option<String>,
    pub guardians: u32,
}

/// How a Lightning payment ended.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Payment {
    /// The gateway paid the invoice; the preimage, in hex, proves it.
    Paid { preimage: String },
    /// The gateway gave up and the funds came back, less fees. Prime signed the refund.
    Refunded,
}

/// An invoice that pays into the wallet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LightningInvoice {
    pub invoice: String,
    /// For [`Wallet::await_lightning_receive`].
    pub operation_id: String,
}

/// Ecash Prime released.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EcashSend {
    /// For [`Wallet::reclaim`], if nobody redeems it.
    pub id: String,
    pub amount_msat: u64,
    /// The token, unless Prime showed it only on its own screen.
    pub token: Option<String>,
}

/// An ecash send nobody has reclaimed or forgotten yet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PendingSend {
    pub id: String,
    pub amount_msat: u64,
    pub created_unix: u64,
    /// Prime released it; otherwise only staged, as when the release was interrupted.
    pub released: bool,
    pub token: Option<String>,
}

/// One operation in the wallet's history.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Activity {
    /// What happened, in a few words.
    pub title: String,
    /// The module behind it: `mintv2`, `lnv2` or `walletv2`.
    pub module: String,
    pub time_unix: u64,
}

#[derive(Clone)]
struct Joined {
    client: ClientHandleArc,
    keys: Arc<DeviceKeys>,
}

impl Joined {
    fn mint(
        &self,
    ) -> Result<fedimint_client::ClientModuleInstance<'_, MintClientModule>, WalletError> {
        self.client
            .get_first_module::<MintClientModule>()
            .map_err(WalletError::failed)
    }

    fn lightning(
        &self,
    ) -> Result<fedimint_client::ClientModuleInstance<'_, LightningClientModule>, WalletError> {
        self.client
            .get_first_module::<LightningClientModule>()
            .map_err(|_| WalletError::invalid("this federation has no Lightning module"))
    }

    fn onchain(
        &self,
    ) -> Result<fedimint_client::ClientModuleInstance<'_, WalletClientModule>, WalletError> {
        self.client
            .get_first_module::<WalletClientModule>()
            .map_err(|_| WalletError::invalid("this federation has no on-chain wallet module"))
    }
}

/// A watch-only fedimint wallet whose spend keys Prime holds.
#[derive(uniffi::Object)]
pub struct Wallet {
    store: Store,
    device: Device,
    link: Arc<RoutedLink>,
    joined: RwLock<Option<Joined>>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

async fn connectors() -> ConnectorRegistry {
    ConnectorRegistry::build_from_client_defaults().bind().await
}

async fn open_database(store: &Store) -> Result<Database, WalletError> {
    Ok(MemAndRedb::new(store.database())
        .await
        .map_err(|error| WalletError::failed(format!("cannot open the database: {error}")))?
        .into())
}

fn root_secret(store: &Store) -> Result<RootSecret, WalletError> {
    Ok(RootSecret::StandardDoubleDerive(
        PlainRootSecretStrategy::to_root_secret(&store.phone_secret()?),
    ))
}

/// A client builder whose note keys, refund keys and signer are Prime's.
async fn client_builder(keys: &Arc<DeviceKeys>) -> ClientBuilder {
    let mut inits = ClientModuleInitRegistry::new();
    inits.attach(MintClientInitWithKeys(keys.clone()));
    inits.attach(LightningClientInit {
        refund_keys: Some(keys.clone()),
        ..LightningClientInit::default()
    });
    inits.attach(WalletClientInit);
    let mut builder = Client::builder().await;
    builder.with_module_inits(inits);
    builder.with_transaction_signer(keys.clone());
    builder
}

fn view_keys(material: &ViewMaterial) -> Result<ViewKeys, WalletError> {
    let spend_key = bitcoin::secp256k1::PublicKey::from_slice(&material.spend_key.0)
        .map_err(|error| WalletError::failed(format!("the stored pairing: {error}")))?;
    Ok(ViewKeys::new(
        spend_key,
        material.chain_code.0,
        material.view_secret.0,
    ))
}

fn view_material(keys: &DeviceKeys) -> ViewMaterial {
    let view = keys.view();
    ViewMaterial {
        federation: keys.federation(),
        key_scheme: KEY_SCHEME.to_owned(),
        spend_key: FixedBytes(view.spend_key().serialize()),
        chain_code: FixedBytes(view.chain_code()),
        view_secret: FixedBytes(view.view_secret()),
    }
}

fn summary(config: &ClientConfig) -> FederationSummary {
    FederationSummary {
        id: config.calculate_federation_id().to_string(),
        name: config.global.federation_name().map(str::to_owned),
        guardians: u32::try_from(config.global.api_endpoints.len()).unwrap_or(u32::MAX),
    }
}

fn operation_id(text: &str) -> Result<OperationId, WalletError> {
    OperationId::from_str(text.trim()).map_err(|_| WalletError::invalid("not an operation id"))
}

fn decode_notes(send: &StoredSend) -> Result<Vec<IssuedExternalNote>, WalletError> {
    Vec::<IssuedExternalNote>::consensus_decode_hex(&send.notes, &ModuleDecoderRegistry::default())
        .map_err(|error| WalletError::failed(format!("the stored send: {error}")))
}

impl Wallet {
    async fn joined(&self) -> Result<Joined, WalletError> {
        self.joined
            .read()
            .await
            .clone()
            .ok_or(WalletError::NotJoined)
    }

    /// Redeems a token the wallet holds itself. With `resume`, a token an earlier call already
    /// submitted is followed to its end instead of refused, as when a take-back was interrupted.
    async fn redeem(&self, joined: &Joined, token: &str, resume: bool) -> Result<u64, WalletError> {
        let ecash: ECash = base32::decode_prefixed(FEDIMINT_PREFIX, token.trim())
            .map_err(|error| WalletError::invalid(format!("not an ecash token: {error}")))?;
        let amount = ecash.amount();
        let mint = joined.mint()?;
        // `receive` names its operation after the token.
        let operation = match mint.receive(ecash.clone(), Value::Null).await {
            Ok(operation) => operation,
            Err(ReceiveECashError::AlreadyReceived) if resume => {
                OperationId::from_encodable(&ecash)
            }
            Err(error) => return Err(WalletError::failed(error)),
        };
        match mint
            .await_final_receive_operation_state(operation)
            .await
            .map_err(WalletError::failed)?
        {
            FinalReceiveOperationState::Success => {
                settle(joined, operation).await;
                Ok(amount.msats)
            }
            other => Err(WalletError::failed(format!(
                "the federation did not accept the token: {other:?}"
            ))),
        }
    }
}

/// Waits until the change notes of `operation`, a receive or a reclaim, are in the wallet. The
/// federation accepting the transaction ends the operation, but the balance only counts notes once
/// their signatures arrive, so without this the money looks missing until the next refresh.
async fn settle(joined: &Joined, operation: OperationId) {
    let Some(entry) = joined.client.operation_log().get_operation(operation).await else {
        return;
    };
    let change = match entry.try_meta::<MintOperationMeta>() {
        Ok(MintOperationMeta::Receive {
            change_outpoint_range,
            ..
        }) => change_outpoint_range,
        Ok(MintOperationMeta::ClaimExternal {
            change_outpoint_range,
            ..
        }) => change_outpoint_range,
        _ => return,
    };
    // The money is already the wallet's; only what the balance shows waits, so a slow or failed
    // issuance does not fail the call.
    let _ = tokio::time::timeout(
        SETTLE_TIMEOUT,
        joined
            .client
            .await_primary_bitcoin_module_outputs(operation, change.into_iter().collect()),
    )
    .await;
}

#[uniffi::export(async_runtime = "tokio")]
impl Wallet {
    /// Opens the wallet kept in `dir`, reaching Prime through `link`. A wallet that joined a
    /// federation before reopens it.
    #[uniffi::constructor]
    pub async fn open(dir: String, link: Arc<dyn PrimeLink>) -> Result<Arc<Self>, WalletError> {
        let store = Store::new(dir.into())?;
        let link = Arc::new(RoutedLink::new(link));
        let device = Device::new(link.clone());
        let mut joined = None;
        if let Some(material) = store.pairing()? {
            let keys = Arc::new(DeviceKeys::new(
                device.clone(),
                material.federation,
                view_keys(&material)?,
            ));
            let db = open_database(&store).await?;
            match client_builder(&keys)
                .await
                .open(connectors().await, db, root_secret(&store)?)
                .await
            {
                Ok(client) => {
                    joined = Some(Joined {
                        client: Arc::new(client),
                        keys,
                    })
                }
                // Paired, but the join never finished: start over.
                Err(ClientBuildError::DatabaseNotInitialized) => {
                    store.forget_pairing()?;
                    store.remove_database()?;
                }
                Err(error) => return Err(WalletError::failed(error)),
            }
        }
        Ok(Arc::new(Self {
            store,
            device,
            link,
            joined: RwLock::new(joined),
        }))
    }

    /// Sends every request to the app in the KeyOS simulator at `address` over the network, a
    /// host or `host:port` (port 7865 by default), instead of through the app's own link; `None`
    /// goes back to the app's link. A stand-in for Bluetooth while there is no device.
    pub fn set_simulator_address(&self, address: Option<String>) {
        let address = address
            .map(|address| address.trim().to_owned())
            .filter(|address| !address.is_empty());
        self.link.set_simulator(address);
    }

    /// The federation the wallet belongs to, if it joined one.
    pub async fn federation(&self) -> Option<FederationSummary> {
        let joined = self.joined.read().await.clone()?;
        Some(summary(&joined.client.config().await))
    }

    /// Joins the federation `invite_code` names and pairs it with Prime: the person at Prime
    /// approves the federation, then the export of its view material. Two exchanges with Prime.
    pub async fn join(&self, invite_code: String) -> Result<FederationSummary, WalletError> {
        let mut joined = self.joined.write().await;
        if joined.is_some() {
            return Err(WalletError::AlreadyJoined);
        }
        let invite = InviteCode::from_str(invite_code.trim())
            .map_err(|error| WalletError::invalid(format!("not an invite code: {error}")))?;
        let connectors = connectors().await;
        let (config, _) = download_from_invite_code(&connectors, &invite)
            .await
            .map_err(|error| {
                WalletError::failed(format!("cannot reach the federation: {error}"))
            })?;
        let keys = Arc::new(DeviceKeys::pair(self.device.clone(), &config).await?);
        // Kept before joining, so a join that stops halfway is recognised and started over.
        self.store.save_pairing(&view_material(&keys))?;
        self.store.remove_database()?;
        let db = open_database(&self.store).await?;
        let client = client_builder(&keys)
            .await
            .preview_with_existing_config(connectors, config.clone(), invite.api_secret())
            .await
            .join(db, root_secret(&self.store)?)
            .await
            .map_err(WalletError::failed)?;
        *joined = Some(Joined {
            client: Arc::new(client),
            keys,
        });
        Ok(summary(&config))
    }

    /// Leaves the federation: the phone forgets the pairing, the client's database and its record
    /// of ecash sent. Prime keeps the federation and its keys, so joining it again and pairing
    /// recovers the balance from the federation.
    pub async fn leave(&self) -> Result<(), WalletError> {
        let mut joined = self.joined.write().await;
        let Some(left) = joined.take() else {
            return Err(WalletError::NotJoined);
        };
        drop(left);
        self.store.forget_pairing()?;
        self.store.remove_database()?;
        for send in self.store.sends()? {
            self.store.remove_send(&send.id)?;
        }
        Ok(())
    }

    /// The spendable balance, in millisatoshis.
    pub async fn balance_msat(&self) -> Result<u64, WalletError> {
        let joined = self.joined().await?;
        Ok(joined
            .client
            .get_balance_for_btc()
            .await
            .map_err(WalletError::failed)?
            .msats)
    }

    /// Redeems an ecash token into the wallet, without Prime. Returns the amount received.
    pub async fn receive_ecash(&self, token: String) -> Result<u64, WalletError> {
        let joined = self.joined().await?;
        self.redeem(&joined, &token, false).await
    }

    /// An invoice for `amount_msat` that pays into the wallet without Prime. `gateway` picks the
    /// Lightning gateway; otherwise the federation's are used.
    pub async fn receive_lightning(
        &self,
        amount_msat: u64,
        description: String,
        gateway: Option<String>,
    ) -> Result<LightningInvoice, WalletError> {
        let joined = self.joined().await?;
        let gateway = gateway
            .map(|url| SafeUrl::parse(&url))
            .transpose()
            .map_err(|error| WalletError::invalid(format!("not a gateway address: {error}")))?;
        let (invoice, operation) = joined
            .lightning()?
            .receive(
                Amount::from_msats(amount_msat),
                INVOICE_EXPIRY_SECS,
                Bolt11InvoiceDescription::Direct(description),
                gateway,
                Value::Null,
            )
            .await
            .map_err(WalletError::failed)?;
        Ok(LightningInvoice {
            invoice: invoice.to_string(),
            operation_id: operation.fmt_full().to_string(),
        })
    }

    /// Waits until an invoice from [`Wallet::receive_lightning`] is paid (true) or expires
    /// (false).
    pub async fn await_lightning_receive(&self, operation_id: String) -> Result<bool, WalletError> {
        let joined = self.joined().await?;
        let state = joined
            .lightning()?
            .await_final_receive_operation_state(self::operation_id(&operation_id)?)
            .await
            .map_err(WalletError::failed)?;
        match state {
            fedimint_lnv2_client::FinalReceiveOperationState::Claimed => Ok(true),
            fedimint_lnv2_client::FinalReceiveOperationState::Expired => Ok(false),
            other => Err(WalletError::failed(format!(
                "the payment could not be claimed: {other:?}"
            ))),
        }
    }

    /// Pays a BOLT11 invoice. Prime reviews the invoice and signs; if the gateway gives up,
    /// Prime signs the refund too. `gateway` picks the Lightning gateway; otherwise the
    /// federation's are used.
    pub async fn pay_invoice(
        &self,
        invoice: String,
        gateway: Option<String>,
    ) -> Result<Payment, WalletError> {
        let joined = self.joined().await?;
        let invoice = Bolt11Invoice::from_str(invoice.trim())
            .map_err(|error| WalletError::invalid(format!("not a Lightning invoice: {error}")))?;
        let gateway = gateway
            .map(|url| SafeUrl::parse(&url))
            .transpose()
            .map_err(|error| WalletError::invalid(format!("not a gateway address: {error}")))?;
        let lightning = joined.lightning()?;
        let operation = lightning
            .send(invoice, gateway, Value::Null)
            .await
            .map_err(WalletError::failed)?;
        match lightning
            .await_final_send_operation_state(operation)
            .await
            .map_err(WalletError::failed)?
        {
            fedimint_lnv2_client::FinalSendOperationState::Success(preimage) => Ok(Payment::Paid {
                preimage: hex::encode(preimage),
            }),
            fedimint_lnv2_client::FinalSendOperationState::Refunded => Ok(Payment::Refunded),
            fedimint_lnv2_client::FinalSendOperationState::Failure => {
                Err(WalletError::failed("the payment failed"))
            }
        }
    }

    /// A bitcoin address that pays into the wallet once the deposit is final. Prime is not
    /// needed.
    pub async fn deposit_address(&self) -> Result<String, WalletError> {
        let joined = self.joined().await?;
        Ok(joined.onchain()?.receive().await.to_string())
    }

    /// Sends `amount_sat` to `address` on chain. Prime reviews and signs. Returns the bitcoin
    /// transaction id.
    pub async fn withdraw(&self, address: String, amount_sat: u64) -> Result<String, WalletError> {
        let joined = self.joined().await?;
        let address: Address<NetworkUnchecked> = address
            .trim()
            .parse()
            .map_err(|error| WalletError::invalid(format!("not a bitcoin address: {error}")))?;
        let onchain = joined.onchain()?;
        let operation = onchain
            .send(
                address,
                bitcoin::Amount::from_sat(amount_sat),
                None,
                Value::Null,
            )
            .await
            .map_err(|error| match error {
                fedimint_walletv2_client::SendError::Failed(error) => WalletError::failed(error),
                other => WalletError::invalid(other),
            })?;
        match onchain
            .await_final_send_operation_state(operation)
            .await
            .map_err(WalletError::failed)?
        {
            fedimint_walletv2_client::FinalSendOperationState::Success(txid) => {
                Ok(txid.to_string())
            }
            fedimint_walletv2_client::FinalSendOperationState::Aborted => {
                Err(WalletError::failed("the federation aborted the withdrawal"))
            }
            fedimint_walletv2_client::FinalSendOperationState::Failure => {
                Err(WalletError::failed("the withdrawal failed"))
            }
        }
    }

    /// Sends `amount_msat` as ecash: Prime approves staging the notes, then releases them as a
    /// token, to the phone or, with `show_on_prime`, only on its own screen. Three exchanges with
    /// Prime: the notes' keys, the approval, and the release.
    pub async fn send_ecash(
        &self,
        amount_msat: u64,
        show_on_prime: bool,
    ) -> Result<EcashSend, WalletError> {
        let joined = self.joined().await?;
        let mint = joined.mint()?;
        let staged = joined
            .keys
            .prepare_send(&mint, Amount::from_msats(amount_msat))
            .await?;
        let mut send = StoredSend {
            id: staged.operation_id.fmt_full().to_string(),
            amount_msat: staged.amount().msats,
            created_unix: now_unix(),
            notes: staged.notes.consensus_encode_to_hex(),
            released: false,
            token: None,
        };
        // Kept before the release, so notes staged on Prime are never lost track of.
        self.store.put_send(send.clone())?;
        let delivery = if show_on_prime {
            Delivery::ShowOnDevice
        } else {
            Delivery::ReturnToPhone
        };
        let released = joined.keys.release(&staged.notes, delivery, true).await?;
        send.released = true;
        send.token = released.token.clone();
        self.store.put_send(send.clone())?;
        Ok(EcashSend {
            id: send.id,
            amount_msat: released.amount_msat,
            token: released.token,
        })
    }

    /// Ecash sends not yet reclaimed or forgotten, oldest first.
    pub async fn pending_sends(&self) -> Result<Vec<PendingSend>, WalletError> {
        Ok(self
            .store
            .sends()?
            .into_iter()
            .map(|send| PendingSend {
                id: send.id,
                amount_msat: send.amount_msat,
                created_unix: send.created_unix,
                released: send.released,
                token: send.token,
            })
            .collect())
    }

    /// Takes back an ecash send nobody redeemed. A token the phone holds is redeemed by the phone
    /// alone; a token shown only on Prime comes back with a reclaim Prime signs; notes Prime
    /// staged but never released are released to the phone first. Returns the amount.
    pub async fn reclaim(&self, id: String) -> Result<u64, WalletError> {
        let joined = self.joined().await?;
        let mut send = self
            .store
            .send(&id)?
            .ok_or_else(|| WalletError::invalid("no such send"))?;
        let notes = decode_notes(&send)?;
        if !send.released {
            match joined
                .keys
                .release(&notes, Delivery::ReturnToPhone, false)
                .await
            {
                Ok(released) => send.token = released.token,
                // Prime released them already and its answer never reached the phone, as when
                // the answer code was not scanned. Prime refuses a second release; a reclaim it
                // signs takes them back.
                Err(SendError::Device(DeviceError::Rejected {
                    code: RejectCode::AlreadyReleased,
                    ..
                })) => send.token = None,
                Err(error) => return Err(error.into()),
            }
            send.released = true;
            // Kept before redeeming, so a retry needs neither Prime nor a second release.
            self.store.put_send(send.clone())?;
        }
        let amount = match &send.token {
            Some(token) => self.redeem(&joined, token, true).await?,
            None => {
                let mint = joined.mint()?;
                let operation = joined.keys.reclaim(&mint, &notes).await?;
                match mint
                    .await_final_receive_operation_state(operation)
                    .await
                    .map_err(WalletError::failed)?
                {
                    FinalReceiveOperationState::Success => settle(&joined, operation).await,
                    other => {
                        return Err(WalletError::failed(format!(
                            "the federation refused the reclaim, so the receiver probably redeemed it: {other:?}"
                        )));
                    }
                }
                send.amount_msat
            }
        };
        self.store.remove_send(&id)?;
        Ok(amount)
    }

    /// Stops listing an ecash send, for one the receiver redeemed.
    pub async fn forget_send(&self, id: String) -> Result<(), WalletError> {
        self.store.remove_send(&id)
    }

    /// The latest operations, newest first.
    pub async fn activity(&self) -> Result<Vec<Activity>, WalletError> {
        let joined = self.joined().await?;
        let operations = joined
            .client
            .operation_log()
            .paginate_operations_rev(ACTIVITY_LIMIT, None)
            .await;
        Ok(operations
            .into_iter()
            .map(|(key, entry)| {
                let module = entry.operation_module_kind().to_owned();
                let meta: Value = entry.meta();
                Activity {
                    title: title(&module, &meta),
                    module,
                    time_unix: key
                        .creation_time
                        .duration_since(UNIX_EPOCH)
                        .map(|elapsed| elapsed.as_secs())
                        .unwrap_or_default(),
                }
            })
            .collect())
    }
}

/// A few words for an operation, from its module and the variant of its meta.
fn title(module: &str, meta: &Value) -> String {
    let variant = meta
        .as_object()
        .and_then(|object| object.keys().next())
        .map(String::as_str)
        .unwrap_or_default();
    match (module, variant) {
        ("lnv2", "send") | ("lnv2", "Send") => "Lightning payment",
        ("lnv2", "receive") | ("lnv2", "Receive") => "Lightning received",
        ("lnv2", _) => "Lightning",
        ("walletv2", "send") | ("walletv2", "Send") => "On-chain withdrawal",
        ("walletv2", "receive") | ("walletv2", "Receive") => "On-chain deposit",
        ("walletv2", _) => "On chain",
        ("mintv2", "receive") | ("mintv2", "Receive") => "Ecash received",
        ("mintv2", "issue_external") | ("mintv2", "IssueExternal") => "Ecash prepared",
        ("mintv2", "claim_external") | ("mintv2", "ClaimExternal") => "Ecash reclaimed",
        ("mintv2", _) => "Ecash",
        _ => "Other",
    }
    .to_owned()
}
