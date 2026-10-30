//! End-to-end check of the Fedimint signer app in the hosted KeyOS simulator.
//!
//! The driver plays the phone. It sends the golden scenario (fedimint-built transactions for the
//! app seed `5e…5e`) through the simulator file bridge, approves and rejects on the simulator's
//! screen with passport-drive, types the PIN when the system asks, and checks every answer on its
//! own terms: signatures against the transaction ids with secp256k1, the released token against
//! the token the scenario's notes make, and the token shown on screen by reading the QR code off a
//! screenshot.
//!
//! ```text
//! fps-sim-e2e prepare         before the app starts: pin the scenario seed, ask for a clean start
//! fps-sim-e2e run             the scenario, against the running app
//! fps-sim-e2e after-restart   after the app restarts: what it stored survived
//! fps-sim-e2e restore         put the developer's own simulator seed back
//! ```
//!
//! `scripts/sim-e2e.sh` runs the whole sequence, starting and stopping the simulator around it.
//! Options: `--shots <dir>` for the screenshots (default `target/sim-e2e`), `--drive <path>` for
//! passport-drive (default `foundation-passport-drive` on `PATH`), and `--pin <digits>` for the
//! simulator's PIN (default 123456, what the hosted security server starts with).

mod scenario;

use std::path::PathBuf;
use std::time::Duration;

use bitcoin::secp256k1::{schnorr, Message, Secp256k1};
use fps_protocol::{
    decode_response, encode, Branch, Delivery, Envelope, FixedBytes, NoteRef, RejectCode,
    ReleaseRequest, Request, RequestId, Response, ViewMaterial, KEY_SCHEME, PROTOCOL_VERSION,
};
use fps_signer_core::{
    analyze, plan_release, AnalysisOptions, EmptyLedger, Federation, FederationKeys,
};
use fps_simdrive::{self as screen, Screen, APPROVE, DONE, REJECT};
use fps_transport::bridge::Bridge;
use scenario::{Case, Scenario, ScenarioLedger};

/// How long the app may take to answer once it has what it needs.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);
/// Where the developer's own seed is kept while the scenario's is pinned.
const SAVED_SEED: &str = "app-seed.hex.saved";

type Result<T, E = String> = std::result::Result<T, E>;

/// Request ids are fixed so a later run can replay them.
fn id(n: u8) -> RequestId {
    FixedBytes([n; 16])
}

const IMPORT: u8 = 2;
const REISSUE: u8 = 5;

/// How the driver answers a review.
#[derive(Clone, Copy)]
enum Choice {
    Approve,
    Reject,
    /// Approve, then type the PIN when the system asks for it.
    ApproveWithPin,
}

struct Driver {
    bridge: Bridge,
    screen: Screen,
    /// The simulator's PIN.
    pin: String,
    scenario: Scenario,
    federation: Federation,
    keys: FederationKeys,
    ledger: ScenarioLedger,
    checks: u32,
}

impl Driver {
    fn new(screen: Screen, pin: String) -> Result<Self> {
        let scenario = Scenario::load();
        let federation = Federation::from_descriptor(&scenario.descriptor)
            .map_err(|error| format!("the scenario federation: {error}"))?;
        let keys = FederationKeys::derive(&scenario.app_seed, &federation.id)
            .map_err(|error| format!("the scenario keys: {error}"))?;
        let ledger = scenario.ledger();
        Ok(Self {
            bridge: Bridge::from_env(),
            screen,
            pin,
            scenario,
            federation,
            keys,
            ledger,
            checks: 0,
        })
    }

    fn pass(&mut self, what: &str) {
        self.checks += 1;
        println!("  ok  {what}");
    }

    fn submit_bytes(&self, bytes: &[u8]) -> Result<()> {
        self.bridge
            .submit(bytes)
            .map(|_| ())
            .map_err(|error| format!("cannot queue a request: {error}"))
    }

    fn submit(&self, n: u8, body: Request) -> Result<()> {
        self.submit_bytes(&encode(&Envelope::new(id(n), body)))
    }

    fn answer(&self, request: &RequestId) -> Result<Response> {
        let bytes = self
            .bridge
            .wait_response(request, ANSWER_TIMEOUT)
            .map_err(|error| format!("cannot read the outbox: {error}"))?
            .ok_or_else(|| format!("no answer to request {} in time", request.to_hex()))?;
        let envelope = decode_response(&bytes).map_err(|error| format!("the answer: {error}"))?;
        if envelope.id != *request || envelope.version != PROTOCOL_VERSION {
            return Err(format!(
                "the answer is addressed to {}",
                envelope.id.to_hex()
            ));
        }
        Ok(envelope.body)
    }

    /// A request the app answers without asking the user.
    fn ask(&self, n: u8, body: Request) -> Result<Response> {
        self.submit(n, body)?;
        self.answer(&id(n))
    }

    /// A request that needs the user: the review must show, nothing may come back before the user
    /// decides, and the app goes home after.
    fn decide(&mut self, n: u8, body: Request, name: &str, choice: Choice) -> Result<Response> {
        let before = self.screen.capture()?;
        self.screen.clear_logs()?;
        self.submit(n, body)?;
        self.screen.wait_for_log("review on screen")?;
        self.screen.wait_for_new_screen(&before)?;
        if let Some(bytes) = self
            .bridge
            .take_response(&id(n))
            .map_err(|error| format!("cannot read the outbox: {error}"))?
        {
            let early = decode_response(&bytes).map(|envelope| envelope.body);
            return Err(format!(
                "{name}: answered before the user decided: {early:?}"
            ));
        }
        let review = self.screen.capture()?;
        self.screen.shot(&format!("{name}-review"))?;
        self.screen.clear_logs()?;
        match choice {
            Choice::Approve => self.screen.tap(APPROVE)?,
            Choice::Reject => self.screen.tap(REJECT)?,
            Choice::ApproveWithPin => {
                self.screen.tap(APPROVE)?;
                self.screen.wait_for_log("gui_app_lock_screen: Navigated")?;
                self.screen.wait_for_new_screen(&review)?;
                self.screen.shot(&format!("{name}-pin"))?;
                let pin = self.pin.clone();
                self.screen.enter_pin(&pin)?;
            }
        }
        let answer = self.answer(&id(n))?;
        self.screen.wait_for_log("home on screen")?;
        self.screen.shot(&format!("{name}-after"))?;
        Ok(answer)
    }

    fn approve(&mut self, n: u8, body: Request, name: &str) -> Result<Response> {
        self.decide(n, body, name, Choice::Approve)
    }

    fn sign_request(&self, case: &str) -> Request {
        Request::SignTransaction(self.scenario.case(case).request.clone())
    }

    /// Signatures for `case`, each checked with secp256k1 against the key its input commits to.
    fn check_signed(&mut self, case_name: &str, response: Response) -> Result<()> {
        let case: &Case = self.scenario.case(case_name);
        let Response::Signed(signed) = response else {
            return Err(format!(
                "{case_name}: expected signatures, got {response:?}"
            ));
        };
        if signed.txid.0 != case.txid {
            return Err(format!(
                "{case_name}: signed transaction id {}",
                signed.txid.to_hex()
            ));
        }
        let analysis = analyze(
            &self.federation,
            &self.keys,
            &case.request,
            &self.ledger,
            &AnalysisOptions::default(),
        )
        .map_err(|error| {
            format!("{case_name}: analysing the fedimint-built transaction: {error}")
        })?;
        if signed.signatures.len() != analysis.inputs.len() {
            return Err(format!(
                "{case_name}: {} signatures for {} inputs",
                signed.signatures.len(),
                analysis.inputs.len()
            ));
        }
        let secp = Secp256k1::verification_only();
        let message = Message::from_digest(case.txid);
        for (index, (signature, input)) in
            signed.signatures.iter().zip(&analysis.inputs).enumerate()
        {
            let signature = schnorr::Signature::from_slice(&signature.0)
                .map_err(|error| format!("{case_name}: signature {index}: {error}"))?;
            let (key, _) = input.public_key.x_only_public_key();
            secp.verify_schnorr(&signature, &message, &key)
                .map_err(|_| format!("{case_name}: signature {index} does not verify"))?;
        }
        let count = signed.signatures.len();
        self.pass(&format!(
            "{case_name}: signed txid {}…, {count} BIP-340 signature{} verify",
            &signed.txid.to_hex()[..12],
            if count == 1 { "" } else { "s" }
        ));
        Ok(())
    }

    fn release_request(
        &self,
        case: &str,
        delivery: Delivery,
        include_invite: bool,
    ) -> ReleaseRequest {
        ReleaseRequest {
            federation: self.scenario.federation(),
            notes: self.scenario.case(case).issued.clone(),
            include_invite,
            delivery,
        }
    }

    /// The token a release must produce, built from the scenario's notes and keys.
    fn expected_token(&self, request: &ReleaseRequest) -> Result<String> {
        plan_release(&self.federation, &self.keys, request, &EmptyLedger, true)
            .map(|plan| plan.token.to_token_string())
            .map_err(|error| format!("the expected token: {error}"))
    }

    fn run(&mut self) -> Result<()> {
        let federation = self.scenario.federation();

        println!("Fresh app");
        match self.ask(1, Request::GetInfo)? {
            Response::Info(info)
                if info.protocol == PROTOCOL_VERSION
                    && info.key_scheme == KEY_SCHEME
                    && info.federations.is_empty()
                    && !info.busy =>
            {
                self.pass("info: protocol 1, fps-v1, no federations")
            }
            other => return Err(format!("info: {other:?}")),
        }

        println!("Pairing");
        let import = Request::ImportFederation(self.scenario.descriptor.clone());
        match self.approve(IMPORT, import, "import")? {
            Response::FederationImported(summary)
                if summary.federation == federation && summary.guardians == 4 =>
            {
                self.pass("import approved on screen: 4 guardians")
            }
            other => return Err(format!("import: {other:?}")),
        }
        let view = self.approve(3, Request::ExportViewMaterial { federation }, "view")?;
        let keys = self.keys.view();
        let expected = ViewMaterial {
            federation,
            key_scheme: KEY_SCHEME.to_owned(),
            spend_key: FixedBytes(keys.spend_key().serialize()),
            chain_code: FixedBytes(keys.chain_code()),
            view_secret: FixedBytes(keys.view_secret()),
        };
        match view {
            Response::ViewMaterial(material) if material == expected => {
                self.pass("view material approved: matches the fps-v1 derivation")
            }
            other => return Err(format!("view material: {other:?}")),
        }
        let staged: Vec<NoteRef> = self
            .scenario
            .case("prepare_send")
            .issued
            .iter()
            .map(|note| NoteRef {
                denomination: note.denomination,
                tweak: note.tweak,
            })
            .collect();
        let expected: Vec<FixedBytes<33>> = staged
            .iter()
            .map(|note| {
                self.keys
                    .note_nonce(Branch::Outgoing, note.denomination, &note.tweak.0)
                    .map(|nonce| FixedBytes(nonce.serialize()))
                    .map_err(|error| format!("expected nonce: {error}"))
            })
            .collect::<Result<_>>()?;
        let nonces = Request::DeriveOutgoingNonces {
            federation,
            notes: staged,
        };
        match self.ask(4, nonces)? {
            Response::OutgoingNonces { nonces } if nonces == expected => {
                self.pass("outgoing nonces: answered without review, as derived")
            }
            other => return Err(format!("outgoing nonces: {other:?}")),
        }

        println!("Spending");
        let response = self.approve(REISSUE, self.sign_request("reissue"), "reissue")?;
        self.check_signed("reissue", response)?;
        let pay = self.sign_request("pay_invoice");
        match self.decide(6, pay.clone(), "pay-rejected", Choice::Reject)? {
            Response::Rejected(rejection) if rejection.code == RejectCode::UserRejected => {
                self.pass("pay_invoice: rejected on screen")
            }
            other => return Err(format!("pay_invoice rejection: {other:?}")),
        }
        let response = self.approve(7, pay, "pay-invoice")?;
        self.check_signed("pay_invoice", response)?;
        let response = self.approve(8, self.sign_request("refund"), "refund")?;
        self.check_signed("refund", response)?;
        // 501,000 sats leave the wallet: over the 100,000 sat PIN threshold.
        let withdraw = self.sign_request("withdraw");
        let response = self.decide(15, withdraw, "withdraw", Choice::ApproveWithPin)?;
        self.check_signed("withdraw", response)?;
        self.pass("withdraw: signed only after the system PIN check");

        println!("Ecash");
        let response = self.approve(9, self.sign_request("prepare_send"), "prepare-send")?;
        self.check_signed("prepare_send", response)?;
        let release = self.release_request("prepare_send", Delivery::ShowOnDevice, true);
        let expected = self.expected_token(&release)?;
        let before = self.screen.capture()?;
        self.screen.clear_logs()?;
        match self.ask(10, Request::ReleaseNotes(release))? {
            Response::Released(released)
                if released.token.is_none()
                    && released.notes == 2
                    && released.amount_msat == (1 << 23) + (1 << 20) =>
            {
                self.pass(
                    "prepared notes released without a second review; the phone gets no token",
                )
            }
            other => return Err(format!("release of the prepared notes: {other:?}")),
        }
        self.screen
            .wait_for_log("result on screen: Scan to receive ecash")?;
        self.screen.wait_for_new_screen(&before)?;
        let shot = self.screen.shot("token-on-screen")?;
        let shown = screen::read_qr(&shot)?;
        if shown != expected {
            return Err(format!(
                "the token on screen is not the expected token: {shown}"
            ));
        }
        self.pass("the token on screen decodes to exactly the expected token, with invite");
        self.screen.clear_logs()?;
        self.screen.tap(DONE)?;
        self.screen.wait_for_log("home on screen")?;

        let release = self.release_request("reclaim", Delivery::ReturnToPhone, false);
        let expected = self.expected_token(&release)?;
        match self.approve(11, Request::ReleaseNotes(release), "release")? {
            Response::Released(released)
                if released.token.as_deref() == Some(expected.as_str()) =>
            {
                self.pass("release approved on screen: the phone gets exactly the expected token")
            }
            other => return Err(format!("release to the phone: {other:?}")),
        }
        let response = self.approve(12, self.sign_request("reclaim"), "reclaim")?;
        self.check_signed("reclaim", response)?;

        println!("Robustness");
        self.check_replay()?;
        self.submit_bytes(b"not a protocol message")?;
        match self.answer(&FixedBytes([0; 16]))? {
            Response::Rejected(rejection) if rejection.code == RejectCode::Malformed => {
                self.pass("garbage: rejected as malformed")
            }
            other => return Err(format!("garbage: {other:?}")),
        }
        self.check_status()?;
        Ok(())
    }

    /// A request id the app already answered gets the same signatures back, without a review.
    fn check_replay(&mut self) -> Result<()> {
        let answer = self.ask(REISSUE, self.sign_request("reissue"))?;
        self.check_signed("reissue", answer)?;
        self.pass("replayed request: the stored answer, without a review");
        Ok(())
    }

    fn check_status(&mut self) -> Result<()> {
        match self.ask(13, Request::GetStatus)? {
            Response::Status(status) if status.released.len() == 4 && status.pending.is_none() => {
                self.pass("status: 4 released notes on the ledger, nothing pending")
            }
            other => return Err(format!("status: {other:?}")),
        }
        Ok(())
    }

    fn after_restart(&mut self) -> Result<()> {
        println!("After a restart");
        match self.ask(14, Request::GetInfo)? {
            Response::Info(info)
                if info.federations.len() == 1
                    && info.federations[0].federation == self.scenario.federation() =>
            {
                self.pass("info: the imported federation survived")
            }
            other => return Err(format!("info: {other:?}")),
        }
        self.check_replay()?;
        self.check_status()?;
        Ok(())
    }
}

fn prepare(bridge: &Bridge) -> Result<()> {
    let scenario = Scenario::load();
    let seed = bridge.seed_file();
    let saved = bridge.root().join(SAVED_SEED);
    if seed.exists() && !saved.exists() {
        std::fs::rename(&seed, &saved)
            .map_err(|error| format!("cannot set aside {}: {error}", seed.display()))?;
    }
    bridge
        .pin_app_seed(&scenario.app_seed)
        .and_then(|()| bridge.request_reset())
        .and_then(|()| bridge.create())
        .and_then(|()| bridge.clear_queues())
        .map_err(|error| format!("cannot prepare {}: {error}", bridge.root().display()))?;
    println!(
        "pinned the scenario seed and asked for a clean start in {}",
        bridge.root().display()
    );
    Ok(())
}

fn restore(bridge: &Bridge) -> Result<()> {
    let saved = bridge.root().join(SAVED_SEED);
    let seed = bridge.seed_file();
    let result = if saved.exists() {
        std::fs::rename(&saved, &seed)
    } else {
        // No seed was pinned before: the app goes back to its own.
        std::fs::remove_file(&seed).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })
    };
    result.map_err(|error| format!("cannot restore {}: {error}", seed.display()))?;
    bridge
        .request_reset()
        .map_err(|error| format!("cannot ask for a clean start: {error}"))?;
    println!("unpinned the scenario seed; the app forgets the scenario on its next start");
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let mut shots = PathBuf::from("target/sim-e2e");
    let mut drive = PathBuf::from("foundation-passport-drive");
    let mut pin = "123456".to_owned();
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--shots", Some(value)) => shots = value.into(),
            ("--drive", Some(value)) => drive = value.into(),
            ("--pin", Some(value)) => pin = value,
            _ => {
                eprintln!("unknown option {arg}");
                std::process::exit(2);
            }
        }
    }

    let bridge = Bridge::from_env();
    let result = match command.as_str() {
        "prepare" => prepare(&bridge),
        "restore" => restore(&bridge),
        "run" | "after-restart" => Screen::new(drive, shots.clone())
            .and_then(|screen| Driver::new(screen, pin))
            .and_then(|mut driver| {
                let outcome = if command == "run" {
                    driver.run()
                } else {
                    driver.after_restart()
                };
                println!(
                    "{} checks passed; screenshots in {}",
                    driver.checks,
                    shots.display()
                );
                outcome
            }),
        _ => {
            eprintln!(
                "usage: fps-sim-e2e prepare | run | after-restart | restore \
                 [--shots <dir>] [--drive <path>] [--pin <digits>]"
            );
            std::process::exit(2);
        }
    };
    if let Err(error) = result {
        eprintln!("FAILED: {error}");
        std::process::exit(1);
    }
}
