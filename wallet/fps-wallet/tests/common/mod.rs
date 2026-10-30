//! A federation in this process, Prime in this process, and a watch-only client between them.
//!
//! Prime is the real `fps-signer` service, reached through the same `DeviceLink` the simulator
//! bridge implements, with a test hand on its approve and reject buttons. The gateway is
//! fedimint's LNv2 mock, extended to check the payer's authorisation the way a real gateway does.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bitcoin::hashes::{Hash as _, sha256};
use fedimint_api_client::api::ServerError;
use fedimint_client::module_init::ClientModuleInitRegistry;
use fedimint_client::secret::{PlainRootSecretStrategy, RootSecretStrategy};
use fedimint_client::transaction::TransactionBuilder;
use fedimint_client::{Client, ClientHandleArc, RootSecret};
use fedimint_connectors::ConnectorRegistry;
use fedimint_core::config::{ClientConfig, FederationId};
use fedimint_core::core::OperationId;
use fedimint_core::db::Database;
use fedimint_core::db::mem_impl::MemDatabase;
use fedimint_core::encoding::Encodable as _;
use fedimint_core::secp256k1::rand::rngs::OsRng;
use fedimint_core::secp256k1::schnorr::Signature;
use fedimint_core::secp256k1::{Keypair, Message, SECP256K1, SecretKey};
use fedimint_core::util::SafeUrl;
use fedimint_core::{Amount, OutPoint};
use fedimint_dummy_client::{DummyClientInit, DummyClientModule};
use fedimint_dummy_server::DummyInit;
use fedimint_lnv2_client::LightningClientInit;
use fedimint_lnv2_common::contracts::{IncomingContract, OutgoingContract, PaymentImage};
use fedimint_lnv2_common::gateway_api::{GatewayConnection, PaymentFee, RoutingInfo};
use fedimint_lnv2_common::{Bolt11InvoiceDescription, LightningInvoice};
use fedimint_lnv2_server::LightningInit;
use fedimint_mintv2_client::{MintClientInit, MintClientInitWithKeys};
use fedimint_mintv2_server::MintInit;
use fedimint_testing::federation::FederationTest;
use fedimint_testing::fixtures::Fixtures;
use fps_protocol::RequestId;
use fps_signer::{MemoryStorage, Outcome, Platform, Signer};
use fps_wallet::DeviceKeys;
use fps_wallet::device::{Device, DeviceLink};
use lightning_invoice::{Bolt11Invoice, Currency, InvoiceBuilder, PaymentSecret};

/// The device's app seed in these tests.
pub const APP_SEED: [u8; 32] = [0x5e; 32];

/// What the hand at the device does with the next reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
}

#[derive(Debug, Default)]
struct DevicePlatform;

impl Platform for DevicePlatform {
    fn random_32(&mut self) -> [u8; 32] {
        fedimint_core::secp256k1::rand::random()
    }

    fn now_unix(&self) -> Option<u64> {
        None
    }
}

/// Prime in this process.
pub struct TestDevice {
    signer: Mutex<Signer<MemoryStorage, DevicePlatform>>,
    /// Decisions for the coming reviews, in order; approve once they run out.
    decisions: Mutex<VecDeque<Decision>>,
    /// The title of every review shown, in order.
    reviews: Mutex<Vec<String>>,
    /// How many requests reached the device.
    requests: Mutex<usize>,
}

impl std::fmt::Debug for TestDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestDevice").finish_non_exhaustive()
    }
}

impl TestDevice {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            signer: Mutex::new(
                Signer::open(APP_SEED, MemoryStorage::default(), DevicePlatform)
                    .expect("a fresh device opens"),
            ),
            decisions: Mutex::new(VecDeque::new()),
            reviews: Mutex::new(Vec::new()),
            requests: Mutex::new(0),
        })
    }

    pub fn device(self: &Arc<Self>) -> Device {
        Device::new(self.clone())
    }

    pub fn decide_next(&self, decision: Decision) {
        self.decisions.lock().unwrap().push_back(decision);
    }

    pub fn reviews(&self) -> Vec<String> {
        self.reviews.lock().unwrap().clone()
    }

    pub fn requests(&self) -> usize {
        *self.requests.lock().unwrap()
    }

    pub fn released_notes(&self) -> usize {
        self.signer.lock().unwrap().state().released.len()
    }

    pub fn prepared_notes(&self) -> usize {
        self.signer.lock().unwrap().state().prepared.len()
    }

    pub fn funded_contracts(&self) -> usize {
        self.signer.lock().unwrap().state().contracts.len()
    }
}

#[async_trait]
impl DeviceLink for TestDevice {
    async fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        *self.requests.lock().unwrap() += 1;
        let mut signer = self.signer.lock().unwrap();
        match signer.handle(&request) {
            Outcome::Reply(reply) => Ok(reply),
            Outcome::Released { reply, .. } => Ok(reply),
            Outcome::NeedsApproval(approval) => {
                self.reviews
                    .lock()
                    .unwrap()
                    .push(approval.review.title.clone());
                let request: RequestId = approval.request;
                let decision = self
                    .decisions
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(Decision::Approve);
                match decision {
                    // The PIN, when asked for, is entered correctly.
                    Decision::Approve => signer
                        .approve(&request, true)
                        .map(|approved| approved.reply)
                        .map_err(|error| error.to_string()),
                    Decision::Reject => signer.reject(&request).map_err(|error| error.to_string()),
                }
            }
        }
    }
}

// The gateway: fedimint's LNv2 mock, plus the authorisation check of a real gateway.

const GATEWAY_SECRET: [u8; 32] = [1; 32];
const PAYABLE_PAYMENT_SECRET: [u8; 32] = [211; 32];
const UNPAYABLE_PAYMENT_SECRET: [u8; 32] = [212; 32];
pub const INVOICE_PREIMAGE: [u8; 32] = [1; 32];

pub fn gateway() -> SafeUrl {
    SafeUrl::parse("https://gateway.xyz").expect("valid url")
}

fn gateway_keypair() -> Keypair {
    SecretKey::from_slice(&GATEWAY_SECRET)
        .expect("32 bytes; within curve order")
        .keypair(SECP256K1)
}

/// An invoice the gateway pays.
pub fn payable_invoice() -> Bolt11Invoice {
    invoice(PAYABLE_PAYMENT_SECRET)
}

/// An invoice the gateway gives up on: it cancels the contract, and the payer gets a refund.
pub fn unpayable_invoice() -> Bolt11Invoice {
    invoice(UNPAYABLE_PAYMENT_SECRET)
}

fn invoice(payment_secret: [u8; 32]) -> Bolt11Invoice {
    let sk = SecretKey::new(&mut OsRng);
    InvoiceBuilder::new(Currency::Regtest)
        .description("coffee".to_owned())
        .payment_hash(sha256::Hash::hash(&INVOICE_PREIMAGE))
        .current_timestamp()
        .min_final_cltv_expiry_delta(0)
        .payment_secret(PaymentSecret(payment_secret))
        .amount_milli_satoshis(1_000_000)
        .expiry_time(Duration::from_secs(3600))
        .build_signed(|message| SECP256K1.sign_ecdsa_recoverable(message, &sk))
        .expect("the invoice builds")
}

#[derive(Debug)]
pub struct CheckingGateway {
    keypair: Keypair,
    /// How many payment requests carried a valid authorisation.
    authorised: Mutex<usize>,
}

impl CheckingGateway {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            keypair: gateway_keypair(),
            authorised: Mutex::new(0),
        })
    }

    pub fn authorised(&self) -> usize {
        *self.authorised.lock().unwrap()
    }
}

#[fedimint_core::apply(fedimint_core::async_trait_maybe_send!)]
impl GatewayConnection for CheckingGateway {
    async fn routing_info(
        &self,
        _gateway_api: SafeUrl,
        _federation_id: &FederationId,
    ) -> Result<Option<RoutingInfo>, ServerError> {
        Ok(Some(RoutingInfo {
            lightning_public_key: self.keypair.public_key(),
            lightning_alias: Some("checking-gateway".to_owned()),
            module_public_key: self.keypair.public_key(),
            send_fee_default: PaymentFee::TRANSACTION_FEE_DEFAULT,
            send_fee_minimum: PaymentFee::TRANSACTION_FEE_DEFAULT,
            expiration_delta_default: 500,
            expiration_delta_minimum: 144,
            receive_fee: PaymentFee::TRANSACTION_FEE_DEFAULT,
            receive_enabled: false,
        }))
    }

    async fn bolt11_invoice(
        &self,
        _gateway_api: SafeUrl,
        _federation_id: FederationId,
        _contract: IncomingContract,
        _invoice_amount: Amount,
        _description: Bolt11InvoiceDescription,
        _expiry_time: u32,
    ) -> Result<Bolt11Invoice, ServerError> {
        Err(ServerError::InvalidRequest(
            "receiving over Lightning is not part of these tests".to_owned(),
        ))
    }

    async fn send_payment(
        &self,
        _gateway_api: SafeUrl,
        _federation_id: FederationId,
        _outpoint: OutPoint,
        contract: OutgoingContract,
        invoice: LightningInvoice,
        auth: Signature,
    ) -> Result<Result<[u8; 32], Signature>, ServerError> {
        // What `GatewayClientModuleV2::send_payment` checks before it pays.
        let message = Message::from_digest(*invoice.consensus_hash::<sha256::Hash>().as_ref());
        if SECP256K1
            .verify_schnorr(&auth, &message, &contract.refund_pk.x_only_public_key().0)
            .is_err()
        {
            return Err(ServerError::InvalidRequest(
                "the payer's authorisation does not verify".to_owned(),
            ));
        }
        *self.authorised.lock().unwrap() += 1;

        let LightningInvoice::Bolt11(invoice) = invoice;
        if *invoice.payment_secret() == PaymentSecret(UNPAYABLE_PAYMENT_SECRET) {
            return Ok(Err(self.keypair.sign_schnorr(contract.forfeit_message())));
        }
        if let PaymentImage::Hash(hash) = contract.payment_image {
            assert_eq!(hash, sha256::Hash::hash(&INVOICE_PREIMAGE));
        }
        Ok(Ok(INVOICE_PREIMAGE))
    }
}

/// A federation with the mint v2 module as the primary module, the dummy module to fund
/// wallets out of thin air, and LNv2.
pub fn fixtures(gateway: Arc<CheckingGateway>) -> Fixtures {
    Fixtures::new_primary(MintClientInit, MintInit)
        .with_module(DummyClientInit, DummyInit)
        .with_module(
            LightningClientInit {
                gateway_conn: Some(gateway),
                ..LightningClientInit::default()
            },
            LightningInit,
        )
}

pub fn root_secret(byte: u8) -> RootSecret {
    RootSecret::StandardDoubleDerive(PlainRootSecretStrategy::to_root_secret(&[byte; 64]))
}

/// The federation's client config.
pub async fn client_config(fed: &FederationTest) -> ClientConfig {
    fed.new_client().await.config().await
}

fn watch_only_inits(
    keys: &Arc<DeviceKeys>,
    gateway: &Arc<CheckingGateway>,
) -> ClientModuleInitRegistry {
    let mut inits = ClientModuleInitRegistry::new();
    inits.attach(MintClientInitWithKeys(keys.clone()));
    inits.attach(DummyClientInit);
    inits.attach(LightningClientInit {
        gateway_conn: Some(gateway.clone()),
        refund_keys: Some(keys.clone()),
        ..LightningClientInit::default()
    });
    inits
}

/// A client that holds no spend key: its notes and refunds are Prime's to sign.
pub async fn watch_only_client(
    config: ClientConfig,
    keys: &Arc<DeviceKeys>,
    gateway: &Arc<CheckingGateway>,
    db: Database,
) -> ClientHandleArc {
    let mut builder = Client::builder().await;
    builder.with_module_inits(watch_only_inits(keys, gateway));
    builder.with_transaction_signer(keys.clone());
    let connectors = ConnectorRegistry::build_from_testing_env().bind().await;
    Arc::new(
        builder
            .preview_with_existing_config(connectors, config, None)
            .await
            .join(db, root_secret(0x77))
            .await
            .expect("the watch-only client joins"),
    )
}

/// A watch-only client on a new phone: a fresh database, the same view material, and a recovery
/// of the wallet's notes from the federation.
pub async fn recovering_client(
    config: ClientConfig,
    keys: &Arc<DeviceKeys>,
    gateway: &Arc<CheckingGateway>,
) -> ClientHandleArc {
    let mut builder = Client::builder().await;
    builder.with_module_inits(watch_only_inits(keys, gateway));
    builder.with_transaction_signer(keys.clone());
    let connectors = ConnectorRegistry::build_from_testing_env().bind().await;
    Arc::new(
        builder
            .preview_with_existing_config(connectors, config, None)
            .await
            .recover(MemDatabase::new().into(), root_secret(0x78), None)
            .await
            .expect("the recovering client starts"),
    )
}

/// Pairs a fresh device with the federation and builds the watch-only client.
pub async fn paired_wallet(
    fed: &FederationTest,
    gateway: &Arc<CheckingGateway>,
) -> (Arc<TestDevice>, Arc<DeviceKeys>, ClientHandleArc) {
    let device = TestDevice::new();
    let config = client_config(fed).await;
    let keys = Arc::new(
        DeviceKeys::pair(device.device(), &config)
            .await
            .expect("pairing succeeds"),
    );
    let client = watch_only_client(config, &keys, gateway, MemDatabase::new().into()).await;
    (device, keys, client)
}

/// Funds `client` out of thin air through the dummy module. No key of the wallet's signs this.
pub async fn fund(client: &ClientHandleArc, amount: Amount) {
    let dummy = client
        .get_first_module::<DummyClientModule>()
        .expect("the dummy module is there");
    let operation_id = OperationId::new_random();
    let range = client
        .finalize_and_submit_transaction(
            operation_id,
            "fund",
            |_| (),
            TransactionBuilder::new().with_inputs(dummy.create_input(amount)),
        )
        .await
        .expect("funding is submitted");
    client
        .await_primary_bitcoin_module_outputs(operation_id, range.into_iter().collect())
        .await
        .expect("funding is issued");
}

/// The balance once every state machine has settled.
pub async fn settled_balance(client: &ClientHandleArc) -> Amount {
    client.wait_for_all_active_state_machines().await;
    client
        .get_balance_for_btc()
        .await
        .expect("a bitcoin balance")
}
