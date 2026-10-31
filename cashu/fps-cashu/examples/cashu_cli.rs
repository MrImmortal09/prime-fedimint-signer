//! The wallet's whole flow against a Prime reached over TCP: the Prime app in the KeyOS simulator,
//! where someone approves each review on screen. Prints every step and checks the balances.
//!
//!     cargo run -p fps-cashu --example cashu_cli -- [address] [--dir DIR] [--mint URL] [--info]
//!
//! `address` is the simulator app's host or `host:port` (default 127.0.0.1:7865). The wallet is
//! kept in `DIR` (default `$TMPDIR/fps-cashu-cli`); adding the mint again finds what this Prime
//! already holds there, so a fresh directory works too. `--info` only asks Prime which mints it
//! has pinned, which needs no approval.
//!
//! Prime asks for approval four times: the mint (add, or update its keys), two sends, and the
//! Lightning payment. The mint defaults to https://testnut.cashu.space, whose fake Lightning
//! backend pays every invoice it issues, and "pays" every invoice it is asked to pay.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{Secp256k1, SecretKey, rand};
use cashu::lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
use fps_cashu::{
    CashuError, CashuLinkError, CashuPrimeLink, CashuWallet, NetworkLink, SIMULATOR_TIMEOUT,
};
use fps_protocol::{
    CashuRequest, CashuResponse, Envelope, FixedBytes, Request, Response, decode_response, encode,
};

/// The app's own link, which the simulator address replaces.
struct NoLink;

#[async_trait]
impl CashuPrimeLink for NoLink {
    async fn exchange(&self, _request: Vec<u8>) -> Result<Vec<u8>, CashuLinkError> {
        Err(CashuLinkError::Failed {
            message: "this example reaches Prime only through the simulator".to_owned(),
        })
    }
}

struct Options {
    address: String,
    dir: PathBuf,
    mint: String,
    info: bool,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        address: "127.0.0.1:7865".to_owned(),
        dir: std::env::temp_dir().join("fps-cashu-cli"),
        mint: "https://testnut.cashu.space".to_owned(),
        info: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => options.dir = args.next().ok_or("--dir needs a directory")?.into(),
            "--mint" => options.mint = args.next().ok_or("--mint needs a URL")?,
            "--info" => options.info = true,
            "-h" | "--help" => {
                return Err(
                    "usage: cashu_cli [address] [--dir DIR] [--mint URL] [--info]".to_owned(),
                );
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => options.address = other.to_owned(),
        }
    }
    Ok(options)
}

/// Prime's Cashu info: its scheme and the mints it pinned.
async fn info(address: &str) -> Result<(), String> {
    let id = FixedBytes(bitcoin::secp256k1::rand::random());
    let request = encode(&Envelope::new(id, Request::Cashu(CashuRequest::Info)));
    let answer = NetworkLink::new(address, SIMULATOR_TIMEOUT)
        .exchange(request)
        .await
        .map_err(|error| error.to_string())?;
    let envelope = decode_response(&answer).map_err(|error| error.to_string())?;
    match envelope.body {
        Response::Cashu(CashuResponse::Info(info)) => {
            println!("Prime at {address}: scheme {}", info.scheme);
            for mint in info.mints {
                println!("  {} ({}, {} keysets)", mint.url, mint.unit, mint.keysets);
            }
            Ok(())
        }
        other => Err(format!("Prime answered {other:?}")),
    }
}

/// A fresh invoice to pay, from a node that does not exist, which the test mint's fake Lightning
/// backend "pays". Not one of the mint's own invoices: the mint settles those internally, and
/// refuses ("Request already paid") once its fake backend has paid them, a few seconds after they
/// are issued, which is sooner than anyone approves on Prime.
fn invoice(amount_sat: u64) -> Result<String, String> {
    let secp = Secp256k1::new();
    let key = SecretKey::new(&mut rand::thread_rng());
    let preimage: [u8; 32] = rand::random();
    InvoiceBuilder::new(Currency::Bitcoin)
        .description("fps-cashu cashu_cli".to_owned())
        .payment_hash(sha256::Hash::hash(&preimage))
        .payment_secret(PaymentSecret(rand::random()))
        .current_timestamp()
        .min_final_cltv_expiry_delta(144)
        .amount_milli_satoshis(amount_sat * 1000)
        .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &key))
        .map(|invoice| invoice.to_string())
        .map_err(|error| error.to_string())
}

fn check(what: &str, got: u64, expected: u64) -> Result<(), String> {
    if got == expected {
        println!("  ok  {what}: {got}");
        Ok(())
    } else {
        Err(format!("{what}: {got}, expected {expected}"))
    }
}

async fn run(options: Options) -> Result<(), String> {
    fps_cashu::cashu_install_tls();
    if options.info {
        return info(&options.address).await;
    }
    let text = |error: CashuError| error.to_string();
    let dir = options.dir.to_string_lossy().into_owned();
    println!("wallet in {dir}, Prime at {}", options.address);
    let wallet = CashuWallet::open(dir, Arc::new(NoLink))
        .await
        .map_err(text)?;
    wallet.set_simulator_address(Some(options.address.clone()));

    println!("\n1. add the mint {}: approve on Prime", options.mint);
    let mint = wallet.add_mint(options.mint.clone()).await.map_err(text)?;
    let url = mint.url.clone();
    println!(
        "  ok  {} ({:?}), holding {} sat",
        url, mint.name, mint.balance_sat
    );
    let mut balance = wallet.balance_sat().await.map_err(text)?;

    println!("\n2. top up 256 sat (the test mint pays its own invoices)");
    let top_up = wallet.top_up(url.clone(), 256).await.map_err(text)?;
    println!("  invoice {}", top_up.invoice);
    let added = wallet.finish_top_up(top_up.quote_id).await.map_err(text)?;
    check("topped up", added, 256)?;
    balance += 256;
    check(
        "balance",
        wallet.balance_sat().await.map_err(text)?,
        balance,
    )?;

    println!("\n3. send 64 sat: approve \"Cashu: send ecash\" on Prime");
    let sent = wallet.send(url.clone(), 64).await.map_err(text)?;
    println!("  token {}", sent.token);
    balance -= 64 + sent.fee_sat;
    check(
        "balance after the send",
        wallet.balance_sat().await.map_err(text)?,
        balance,
    )?;

    println!("\n4. receive that token (no Prime)");
    let received = wallet.receive(sent.token).await.map_err(text)?;
    println!("  ok  received {received} sat after the mint's fee");
    balance += received;
    check(
        "balance",
        wallet.balance_sat().await.map_err(text)?,
        balance,
    )?;

    println!("\n5. send 32 sat: approve \"Cashu: send ecash\" on Prime, then take it back");
    let second = wallet.send(url.clone(), 32).await.map_err(text)?;
    balance -= 32 + second.fee_sat;
    check(
        "balance after the send",
        wallet.balance_sat().await.map_err(text)?,
        balance,
    )?;
    let back = wallet.take_back(second.id).await.map_err(text)?;
    println!("  ok  took back {back} sat after the mint's fee");
    balance += back;
    check(
        "balance",
        wallet.balance_sat().await.map_err(text)?,
        balance,
    )?;

    println!("\n6. pay a fresh 16-sat invoice: approve \"Cashu: pay Lightning invoice\" on Prime");
    let bolt11 = invoice(16)?;
    let payment = wallet.pay(url.clone(), bolt11).await.map_err(text)?;
    println!(
        "  ok  paid {} sat, fee {} sat, change {} sat",
        payment.amount_sat, payment.fee_sat, payment.change_sat
    );
    balance -= payment.amount_sat + payment.fee_sat;
    check(
        "balance",
        wallet.balance_sat().await.map_err(text)?,
        balance,
    )?;

    println!("\nactivity, newest first:");
    for entry in wallet.activity().await.map_err(text)? {
        println!(
            "  {:<9} {:>6} sat  {}",
            entry.kind, entry.amount_sat, entry.detail
        );
    }
    println!("\nCashu flow passed: balance {balance} sat");
    Ok(())
}

#[tokio::main]
async fn main() {
    let result = match options() {
        Ok(options) => run(options).await,
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("FAILED: {error}");
        std::process::exit(1);
    }
}
