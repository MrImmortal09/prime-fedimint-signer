//! The whole wallet against the public test mint, https://testnut.cashu.space (cdk-mintd with a
//! fake Lightning backend that pays every invoice it issues), with the real Cashu service of the
//! Prime app in the test process as Prime, approving every review. It needs the network, so it is
//! ignored by default:
//!
//!     cargo test -p fps-cashu --test live -- --ignored --nocapture

use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cashu::nuts::{CurrencyUnit, MintQuoteBolt11Request};
use cashu::{Amount, MintUrl};
use cdk::wallet::{HttpClient, MintConnector};
use cdk::{MintQuoteRequest, MintQuoteResponse};
use fps_cashu::{CashuError, CashuLinkError, CashuPrimeLink, CashuWallet};
use fps_cashu_signer::Device;
use fps_signer::{MemoryStorage, Outcome, Platform};

const MINT: &str = "https://testnut.cashu.space";

struct Clock;

impl Platform for Clock {
    fn random_32(&mut self) -> [u8; 32] {
        bitcoin::secp256k1::rand::random()
    }

    fn now_unix(&self) -> Option<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_secs())
    }
}

/// The Prime app's Cashu service, beside a Fedimint signer with nothing imported, approving every
/// review it is asked for and noting its title.
struct InProcessPrime {
    device: Mutex<Device<MemoryStorage, Clock, MemoryStorage>>,
    reviews: Mutex<Vec<String>>,
}

impl InProcessPrime {
    /// A Prime with a fresh app seed, so its outputs at the test mint were never used before.
    fn new() -> Self {
        let seed: [u8; 32] = bitcoin::secp256k1::rand::random();
        let device = Device::open(
            seed,
            MemoryStorage::default(),
            Clock,
            MemoryStorage::default(),
            Clock,
        )
        .expect("the device opens");
        Self {
            device: Mutex::new(device),
            reviews: Mutex::new(Vec::new()),
        }
    }

    fn reviews(&self) -> Vec<String> {
        self.reviews
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CashuPrimeLink for InProcessPrime {
    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>, CashuLinkError> {
        let mut device = self.device.lock().unwrap_or_else(PoisonError::into_inner);
        match device.handle(&request) {
            Outcome::Reply(reply) | Outcome::Released { reply, .. } => Ok(reply),
            Outcome::NeedsApproval(approval) => {
                let lines: Vec<String> = approval
                    .review
                    .lines
                    .iter()
                    .map(|line| format!("{}: {}", line.label, line.value))
                    .collect();
                println!(
                    "    Prime approves {:?} [{}]",
                    approval.review.title,
                    lines.join("; ")
                );
                self.reviews
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(approval.review.title.clone());
                device
                    .approve(&approval.request, true)
                    .map(|approved| approved.reply)
                    .map_err(|error| CashuLinkError::Failed {
                        message: format!("{error:?}"),
                    })
            }
        }
    }
}

/// A fresh invoice of the test mint, which its fake Lightning backend pays when melted.
async fn invoice(amount: u64) -> String {
    let client = HttpClient::new(MintUrl::from_str(MINT).unwrap(), None);
    let request = MintQuoteRequest::Bolt11(MintQuoteBolt11Request {
        amount: Amount::from(amount),
        unit: CurrencyUnit::Sat,
        description: None,
        pubkey: None,
    });
    match client.post_mint_quote(request).await.expect("a mint quote") {
        MintQuoteResponse::Bolt11(quote) => quote.request,
        _ => panic!("not a bolt11 quote"),
    }
}

async fn balance(wallet: &CashuWallet) -> u64 {
    wallet.balance_sat().await.expect("a balance")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the network and https://testnut.cashu.space"]
async fn the_wallet_works_against_testnut_with_prime_in_process() {
    fps_cashu::cashu_install_tls();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap().to_owned();
    let prime = Arc::new(InProcessPrime::new());
    let wallet = CashuWallet::open(path.clone(), prime.clone())
        .await
        .unwrap();

    println!("add mint {MINT}");
    let mint = wallet.add_mint(format!("{MINT}/")).await.unwrap();
    assert_eq!(mint.url, MINT);
    assert_eq!(mint.balance_sat, 0);
    assert_eq!(wallet.mints().await.unwrap(), vec![mint.clone()]);
    println!("  added {:?}", mint.name);

    println!("top up 256");
    let top_up = wallet.top_up(MINT.to_owned(), 256).await.unwrap();
    assert_eq!(top_up.amount_sat, 256);
    assert!(top_up.invoice.starts_with("lnbc"));
    assert_eq!(wallet.finish_top_up(top_up.quote_id).await.unwrap(), 256);
    assert_eq!(balance(&wallet).await, 256);
    println!("  balance 256");

    println!("send 64");
    let sent = wallet.send(MINT.to_owned(), 64).await.unwrap();
    assert!(sent.token.starts_with("cashuB"), "{}", sent.token);
    assert_eq!((sent.amount_sat, sent.fee_sat), (64, 1));
    assert_eq!(balance(&wallet).await, 256 - 64 - 1);
    println!(
        "  fee {} sat, balance {}",
        sent.fee_sat,
        balance(&wallet).await
    );

    println!("receive that token");
    let received = wallet.receive(sent.token.clone()).await.unwrap();
    assert_eq!(received, 63, "64 less the mint's fee of 1");
    assert_eq!(balance(&wallet).await, 191 + 63);
    // The wallet received its own send, so the send is no longer waiting to be taken back.
    assert!(wallet.sends().await.unwrap().is_empty());
    println!("  received {received}, balance {}", balance(&wallet).await);
    // A token redeemed once cannot be received again.
    match wallet.receive(sent.token.clone()).await {
        Err(CashuError::Invalid { message }) => println!("  again: {message}"),
        other => panic!("the token was received twice: {other:?}"),
    }
    assert_eq!(balance(&wallet).await, 254);

    println!("send 32, then take it back");
    let second = wallet.send(MINT.to_owned(), 32).await.unwrap();
    assert_eq!(second.fee_sat, 1);
    assert_eq!(balance(&wallet).await, 254 - 33);
    let pending = wallet.sends().await.unwrap();
    assert!(pending.iter().any(|send| send.id == second.id));
    let back = wallet.take_back(second.id.clone()).await.unwrap();
    assert_eq!(back, 31, "32 less the mint's fee of 1");
    assert_eq!(balance(&wallet).await, 221 + 31);
    assert!(
        wallet
            .sends()
            .await
            .unwrap()
            .iter()
            .all(|send| send.id != second.id)
    );
    assert!(wallet.take_back(second.id.clone()).await.is_err());
    println!("  took back {back}, balance {}", balance(&wallet).await);

    println!("pay a fresh 16-sat invoice");
    let before = balance(&wallet).await;
    let payment = wallet
        .pay(MINT.to_owned(), invoice(16).await)
        .await
        .unwrap();
    assert_eq!(payment.amount_sat, 16);
    assert!(payment.fee_sat >= 1, "at least the input fee");
    assert_eq!(balance(&wallet).await, before - 16 - payment.fee_sat);
    println!(
        "  paid 16, fee {} sat, change {} sat, balance {}",
        payment.fee_sat,
        payment.change_sat,
        balance(&wallet).await
    );

    assert_eq!(
        prime.reviews(),
        [
            "Cashu: add mint",
            "Cashu: send ecash",
            "Cashu: send ecash",
            "Cashu: pay Lightning invoice",
        ]
    );
    let kinds: Vec<String> = wallet
        .activity()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.kind)
        .collect();
    assert_eq!(
        kinds,
        ["pay", "take_back", "send", "receive", "send", "top_up"]
    );

    // Everything is on disk.
    let final_balance = balance(&wallet).await;
    drop(wallet);
    let reopened = CashuWallet::open(path.clone(), prime.clone())
        .await
        .unwrap();
    assert_eq!(balance(&reopened).await, final_balance);
    assert_eq!(
        reopened.mints().await.unwrap()[0].balance_sat,
        final_balance
    );
    assert_eq!(reopened.activity().await.unwrap().len(), 6);

    // A new phone paired with the same Prime finds the same proofs again, and its outputs
    // continue after the ones already used.
    let other_dir = tempfile::tempdir().unwrap();
    let other = CashuWallet::open(other_dir.path().to_str().unwrap().to_owned(), prime.clone())
        .await
        .unwrap();
    let restored = other.add_mint(MINT.to_owned()).await.unwrap();
    assert_eq!(restored.balance_sat, final_balance);
    println!("  a new phone with the same Prime restores {final_balance}");
    let top_up = other.top_up(MINT.to_owned(), 8).await.unwrap();
    assert_eq!(other.finish_top_up(top_up.quote_id).await.unwrap(), 8);
    assert_eq!(balance(&other).await, final_balance + 8);
}
