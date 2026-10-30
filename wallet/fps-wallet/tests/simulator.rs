//! The watch-only client with the real KeyOS app signing.
//!
//! The app runs in the hosted KeyOS simulator. The wallet reaches it over the simulator file
//! bridge, and a hand on the simulator's screen approves or rejects each review and reads the
//! tokens the app shows. The federation, the gateway and the chain are the ones `signer.rs` uses,
//! in this process.
//!
//! `scripts/sim-wallet-e2e.sh` starts the simulator and runs this test, which is ignored
//! otherwise. Settings: `FPS_SIM_LINK`, `bridge` (the file bridge, the default) or `net` (the
//! network link, the stand-in for QuantumLink, at `FPS_NET_ADDR`, default `127.0.0.1:7865`);
//! `FPS_SIM_DIR`, the bridge directory; `FPS_SIM_DRIVE`, passport-drive (default
//! `foundation-passport-drive` on `PATH`); `FPS_SIM_SHOTS`, where screenshots go (default
//! `target/sim-wallet`). Amounts stay under the app's PIN threshold, so no review asks for the
//! PIN.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail, ensure};
use common::{
    CheckingGateway, INVOICE_PREIMAGE, client_config, fixtures, fund, gateway, payable_invoice,
    peg_in, received_on_chain, settled_balance, unpayable_invoice, watch_only_client,
};
use fedimint_client_module::error::TransactionSubmitError;
use fedimint_client_module::transaction::TransactionSignerError;
use fedimint_core::Amount;
use fedimint_core::base32::{self, FEDIMINT_PREFIX};
use fedimint_core::db::mem_impl::MemDatabase;
use fedimint_lnv2_client::{FinalSendOperationState, LightningClientModule};
use fedimint_mintv2_client::{
    ECash, ExternalNotesError, FinalReceiveOperationState, MintClientModule,
};
use fedimint_testing::federation::FederationTest;
use fedimint_walletv2_client::WalletClientModule;
use fps_protocol::Delivery;
use fps_simdrive::{Decision, Hand, Screen, Seen};
use fps_transport::bridge::Bridge;
use fps_transport::net::DEFAULT_PORT;
use fps_wallet::DeviceKeys;
use fps_wallet::device::{BridgeLink, Device, NetworkLink};
use fps_wallet::send::SendError;
use serde_json::Value;

/// How long the wallet waits for an answer from the app, review included.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a token may take to appear on the app's screen once released.
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

fn setting(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

/// The app, over the link `FPS_SIM_LINK` names.
fn prime() -> anyhow::Result<Device> {
    let link = std::env::var("FPS_SIM_LINK").unwrap_or_default();
    Ok(match link.as_str() {
        "" | "bridge" => Device::new(Arc::new(BridgeLink::new(
            Bridge::from_env(),
            ANSWER_TIMEOUT,
        ))),
        "net" => {
            let address = std::env::var("FPS_NET_ADDR")
                .unwrap_or_else(|_| format!("127.0.0.1:{DEFAULT_PORT}"));
            println!("Reaching the app over the network at {address}");
            Device::new(Arc::new(NetworkLink::new(address, ANSWER_TIMEOUT)))
        }
        other => bail!("FPS_SIM_LINK must be bridge or net, not {other:?}"),
    })
}

/// The `n`th token the app showed (counting from one), as the hand read it off the screen.
async fn token_on_screen(hand: &Hand, n: usize) -> anyhow::Result<String> {
    let deadline = tokio::time::Instant::now() + TOKEN_TIMEOUT;
    loop {
        if let Some(error) = hand.failure() {
            bail!("the hand at the simulator stopped: {error}");
        }
        if let Some(token) = hand.codes().get(n - 1) {
            return Ok(token.clone());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("the app showed no token {n} within {TOKEN_TIMEOUT:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A stranger who never met the wallet redeems `token`, and gets `amount`.
async fn redeem(fed: &FederationTest, token: &str, amount: Amount) -> anyhow::Result<()> {
    let ecash: ECash = base32::decode_prefixed(FEDIMINT_PREFIX, token)?;
    ensure!(
        ecash.amount() == amount,
        "the token holds {}",
        ecash.amount()
    );
    let stranger = fed.new_client().await;
    let mint = stranger.get_first_module::<MintClientModule>()?;
    let receive = mint.receive(ecash, Value::Null).await?;
    ensure!(
        mint.await_final_receive_operation_state(receive).await?
            == FinalReceiveOperationState::Success,
        "the stranger could not redeem the token"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the app running in the KeyOS simulator: scripts/sim-wallet-e2e.sh"]
async fn the_app_in_the_simulator_signs_for_the_wallet() -> anyhow::Result<()> {
    let screen = Screen::new(
        setting("FPS_SIM_DRIVE", "foundation-passport-drive"),
        setting("FPS_SIM_SHOTS", "target/sim-wallet"),
    )
    .map_err(anyhow::Error::msg)?;
    let hand = Hand::start(screen).map_err(anyhow::Error::msg)?;
    let prime = prime()?;
    let outcome = scenario(&hand, prime).await;
    // The hand's own failure explains a wallet step that waited for an answer in vain.
    match (outcome, hand.stop()) {
        (Ok(()), Ok(_)) => Ok(()),
        (outcome, Err(error)) => Err(anyhow::anyhow!(
            "the hand at the simulator stopped: {error}; {outcome:?}"
        )),
        (Err(error), Ok(_)) => Err(error),
    }
}

async fn scenario(hand: &Hand, prime: Device) -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fixtures = fixtures(gateway_conn.clone());
    let chain = fixtures.bitcoin();
    let fed = fixtures.new_fed_not_degraded().await;
    let config = client_config(&fed).await;

    println!("Pairing");
    let keys = Arc::new(DeviceKeys::pair(prime, &config).await?);
    assert_eq!(hand.reviews(), ["Add federation", "Connect wallet app"]);
    let wallet = watch_only_client(config, &keys, &gateway_conn, MemDatabase::new().into()).await;
    fund(&wallet, Amount::from_sats(50_000)).await;
    assert_eq!(hand.reviews().len(), 2, "receiving needs no review");

    println!("Lightning");
    let lightning = wallet.get_first_module::<LightningClientModule>()?;
    let payment = lightning
        .send(payable_invoice(), Some(gateway()), Value::Null)
        .await?;
    assert_eq!(
        lightning.await_final_send_operation_state(payment).await?,
        FinalSendOperationState::Success(INVOICE_PREIMAGE)
    );
    assert_eq!(gateway_conn.authorised(), 1);
    // The gateway gives up on this one, and the app signs the refund.
    let payment = lightning
        .send(unpayable_invoice(), Some(gateway()), Value::Null)
        .await?;
    assert_eq!(
        lightning.await_final_send_operation_state(payment).await?,
        FinalSendOperationState::Refunded
    );
    assert_eq!(gateway_conn.authorised(), 2);

    println!("On chain");
    peg_in(&fed, &chain, bitcoin::Amount::from_sat(1_000_000)).await?;
    let onchain = wallet.get_first_module::<WalletClientModule>()?;
    let address = chain.get_new_address().await;
    let value = bitcoin::Amount::from_sat(20_000);
    let withdrawal = onchain
        .send(address.as_unchecked().clone(), value, None, Value::Null)
        .await?;
    assert!(matches!(
        onchain.await_final_send_operation_state(withdrawal).await?,
        fedimint_walletv2_client::FinalSendOperationState::Success(_)
    ));
    assert_eq!(
        received_on_chain(&chain, &address).await?,
        Amount::from_sats(value.to_sat())
    );

    println!("Declined");
    let balance = settled_balance(&wallet).await;
    let mint = wallet.get_first_module::<MintClientModule>()?;
    hand.decide_next(Decision::Reject);
    let declined = keys.prepare_send(&mint, Amount::from_sats(1_024)).await;
    assert!(
        matches!(
            declined,
            Err(SendError::Mint(ExternalNotesError::Failed(
                TransactionSubmitError::Signer(TransactionSignerError::Declined(_))
            )))
        ),
        "{declined:?}"
    );
    assert_eq!(settled_balance(&wallet).await, balance);

    println!("Ecash");
    // Released to the phone without a second review.
    let staged = keys.prepare_send(&mint, Amount::from_sats(1_024)).await?;
    let released = keys
        .release(&staged.notes, Delivery::ReturnToPhone, true)
        .await?;
    let token = released
        .token
        .context("the token comes back to the phone")?;
    redeem(&fed, &token, Amount::from_sats(1_024)).await?;
    // Shown on the app's screen: the stranger scans it from there.
    let staged = keys.prepare_send(&mint, Amount::from_sats(2_048)).await?;
    let released = keys
        .release(&staged.notes, Delivery::ShowOnDevice, false)
        .await?;
    assert_eq!(released.token, None, "the phone never sees it");
    let token = token_on_screen(hand, 1).await?;
    redeem(&fed, &token, Amount::from_sats(2_048)).await?;
    // Shown, never scanned, and taken back.
    let staged = keys.prepare_send(&mint, Amount::from_sats(1_024)).await?;
    keys.release(&staged.notes, Delivery::ShowOnDevice, false)
        .await?;
    token_on_screen(hand, 2).await?;
    let before = settled_balance(&wallet).await;
    let reclaim = keys.reclaim(&mint, &staged.notes).await?;
    assert_eq!(
        mint.await_final_receive_operation_state(reclaim).await?,
        FinalReceiveOperationState::Success
    );
    let after = settled_balance(&wallet).await;
    assert!(
        after > before + Amount::from_sats(1_000),
        "{after} after {before}"
    );

    // Every screen the person at the app dealt with, in order.
    let screens: Vec<String> = hand
        .seen()
        .into_iter()
        .map(|seen| match seen {
            Seen::Review {
                title,
                decision: Decision::Approve,
            } => format!("approved {title}"),
            Seen::Review {
                title,
                decision: Decision::Reject,
            } => format!("rejected {title}"),
            Seen::Result { title, qr } => match qr {
                Some(_) => format!("scanned {title}"),
                None => format!("dismissed {title}"),
            },
        })
        .collect();
    assert_eq!(
        screens,
        [
            "approved Add federation",
            "approved Connect wallet app",
            "approved Pay Lightning invoice",
            "approved Pay Lightning invoice",
            "approved Refund failed payment",
            "approved Withdraw on chain",
            "rejected Prepare ecash to send",
            "approved Prepare ecash to send",
            "approved Prepare ecash to send",
            "scanned Scan to receive ecash",
            "approved Prepare ecash to send",
            "scanned Scan to receive ecash",
            "approved Reclaim unredeemed ecash",
        ]
    );
    println!("{} screens dealt with on the simulator", screens.len());
    Ok(())
}
