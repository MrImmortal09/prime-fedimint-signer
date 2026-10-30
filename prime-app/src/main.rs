//! Fedimint Signer for Passport Prime.
//!
//! The app is a thin shell around `fps-signer`: transports bring request bytes in, the service
//! decides, the review screens show what it decided, and the answer leaves on the transport the
//! request came from. Transports in this build:
//!
//! - animated QR: the user scans a request with the system scanner (or opens the app from the
//!   Launcher's scanner) and the phone scans the answer from the screen;
//! - the simulator file bridge (`bridge.rs`), for development and automated tests.

mod bridge;
mod device;
mod qr;
mod theme;
mod view;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use fps_signer::{Outcome, Signer};
use gui_permissions::GuiPermissions;
use slint_keyos_platform::{
    app_ui2,
    gui_server_api::{
        navigation::{
            lockscreen::VerifyPinOptions,
            qrscanner::{MatchedQrResult, ScanQrOptions, ScanQrResult},
        },
        InputMessage,
    },
    navigation::{open_qr_scanner, verify_pin},
    slint::{ComponentHandle, ModelRc, SharedString, Timer},
};
use zeroize::Zeroizing;

app_ui2!("Fedimint Signer");

/// The transport a request arrived on, and so where its answer goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Qr,
    Bridge,
}

/// A result screen: a message, and optionally a QR code to show.
pub struct Screen {
    pub title: String,
    pub message: String,
    pub frames: ModelRc<SharedString>,
    /// The code holds ecash: anyone who scans it can spend it.
    pub secret: bool,
    pub has_next: bool,
}

pub struct App {
    pub signer: Signer<device::AppDataStorage, device::DevicePlatform>,
    /// Where the answer to the request waiting for approval goes.
    pub waiting: Option<Channel>,
    /// A result screen is showing.
    pub showing_result: bool,
    /// Result screens still to show after the current one.
    pub screens: VecDeque<Screen>,
    /// Polls the simulator file bridge while it lives; `None` on the device.
    pub bridge: Option<Timer>,
}

impl App {
    /// A review or a result is on screen, so a new request would interrupt the user.
    pub fn busy(&self) -> bool {
        self.waiting.is_some() || self.showing_result
    }
}

// Each screen change is logged, so tests driving the simulator can wait for the screen.

fn navigate_home(ui: &AppWindow) {
    ui.global::<Navigate>().invoke_main_page(NavigateOptions {
        replace: true,
        animate: Animate::None,
    });
    log::info!("home on screen");
}

/// Shows a problem on the home screen.
fn set_status(ui: &AppWindow, status: &str) {
    ui.global::<Callbacks>().set_status(status.into());
}

/// Shows what just happened on the home screen.
fn set_notice(ui: &AppWindow, notice: &str) {
    let callbacks = ui.global::<Callbacks>();
    callbacks.set_status(SharedString::new());
    callbacks.set_notice(notice.into());
}

/// Runs one message through the signer and routes the outcome.
pub fn handle_message(ui: &AppWindow, app: &Rc<RefCell<App>>, bytes: &[u8], channel: Channel) {
    let outcome = app.borrow_mut().signer.handle(bytes);
    match outcome {
        Outcome::Reply(reply) => deliver(ui, app, channel, reply, None, None),
        Outcome::NeedsApproval(approval) => {
            app.borrow_mut().waiting = Some(channel);
            set_notice(ui, "");
            view::show_review(ui, &approval);
            ui.global::<Navigate>().invoke_review_page(NavigateOptions {
                replace: false,
                animate: Animate::None,
            });
            log::info!("review on screen: {}", approval.review.title);
        }
        Outcome::Released { reply, show_token } => deliver(
            ui,
            app,
            channel,
            reply,
            show_token,
            Some("Released prepared ecash"),
        ),
    }
}

/// Sends an answer on its channel, showing whatever the user needs to see. `decided` names what
/// the user just decided, for requests that ended a review; other answers leave the screen alone.
fn deliver(
    ui: &AppWindow,
    app: &Rc<RefCell<App>>,
    channel: Channel,
    reply: Vec<u8>,
    show_token: Option<Zeroizing<String>>,
    decided: Option<&str>,
) {
    let mut screens = VecDeque::new();
    if let Some(token) = show_token {
        screens.push_back(Screen {
            title: "Scan to receive ecash".to_owned(),
            message: "Let the receiver scan this code. Anyone who scans it can spend it."
                .to_owned(),
            frames: qr::static_frame(&token),
            secret: true,
            has_next: false,
        });
    }
    match channel {
        Channel::Qr => screens.push_back(Screen {
            title: "Answer for your phone".to_owned(),
            message: "Scan this with the wallet app on your phone.".to_owned(),
            frames: qr::response_frames(&reply),
            secret: false,
            has_next: false,
        }),
        // The answer goes straight back, as it will over Bluetooth.
        Channel::Bridge => bridge::write_reply(&reply),
    }
    view::refresh_home(ui, &app.borrow().signer);
    if let Some(decided) = decided {
        set_notice(ui, decided);
    }
    let count = screens.len();
    for (index, screen) in screens.iter_mut().enumerate() {
        screen.has_next = index + 1 < count;
    }
    let first = screens.pop_front();
    {
        let mut app = app.borrow_mut();
        app.screens = screens;
        app.showing_result = first.is_some();
    }
    match first {
        Some(screen) => {
            view::show_screen(ui, &screen);
            ui.global::<Navigate>().invoke_result_page(NavigateOptions {
                replace: true,
                animate: Animate::None,
            });
            log::info!("result on screen: {}", screen.title);
        }
        None if decided.is_some() => navigate_home(ui),
        None => {}
    }
}

fn scan_request(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let options = ScanQrOptions {
        header_title: "Scan request".into(),
        message: "Show the code from the wallet app on your phone.".into(),
        ..ScanQrOptions::default()
    };
    let scan = match open_qr_scanner::<GuiPermissions>(options) {
        Ok(Some(scan)) => scan,
        Ok(None) => return,
        Err(error) => {
            set_status(ui, &format!("The scanner is unavailable: {error:?}"));
            return;
        }
    };
    match scan {
        ScanQrResult::Ur2 { ur_type, data, .. } => match qr::request_payload(&ur_type, data) {
            Ok(bytes) => handle_message(ui, app, &bytes, Channel::Qr),
            Err(message) => set_status(ui, &message),
        },
        ScanQrResult::Qr { .. } => set_status(ui, "That code is not a Fedimint signing request."),
        _ => {}
    }
}

fn approve(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let (request, require_pin) = {
        let app = app.borrow();
        match app.signer.pending() {
            Some(approval) => (approval.request, approval.require_pin),
            None => return,
        }
    };
    let pin_verified = if require_pin {
        let result = verify_pin::<GuiPermissions>(VerifyPinOptions {
            title: Some("Confirm with your PIN".into()),
            want_security_words: false,
        });
        match result {
            Ok(result) if result.success => true,
            Ok(_) => return,
            Err(error) => {
                set_status(ui, &format!("PIN check unavailable: {error:?}"));
                return;
            }
        }
    } else {
        false
    };
    let channel = app.borrow().waiting.unwrap_or(Channel::Qr);
    let title = app
        .borrow()
        .signer
        .pending()
        .map(|a| a.review.title.clone());
    let approved = app.borrow_mut().signer.approve(&request, pin_verified);
    match approved {
        Ok(approved) => {
            app.borrow_mut().waiting = None;
            let decided = format!("Approved: {}", title.unwrap_or_default());
            deliver(
                ui,
                app,
                channel,
                approved.reply,
                approved.show_token,
                Some(&decided),
            );
        }
        Err(error) => set_status(ui, &format!("Not approved: {error}")),
    }
}

fn reject(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let Some(request) = app.borrow().signer.pending().map(|a| a.request) else {
        return;
    };
    let channel = app.borrow().waiting.unwrap_or(Channel::Qr);
    let title = app
        .borrow()
        .signer
        .pending()
        .map(|a| a.review.title.clone());
    let rejected = app.borrow_mut().signer.reject(&request);
    app.borrow_mut().waiting = None;
    match rejected {
        Ok(reply) => {
            let decided = format!("Rejected: {}", title.unwrap_or_default());
            deliver(ui, app, channel, reply, None, Some(&decided));
        }
        Err(error) => set_status(ui, &format!("Could not reject: {error}")),
    }
}

fn next_screen(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let next = app.borrow_mut().screens.pop_front();
    match next {
        Some(screen) => {
            view::show_screen(ui, &screen);
            log::info!("result on screen: {}", screen.title);
        }
        None => {
            // Leaving the screen drops the token from the UI.
            ui.global::<Callbacks>()
                .set_result_frames(ModelRc::default());
            app.borrow_mut().showing_result = false;
            navigate_home(ui);
        }
    }
}

fn settings_changed(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let policy = view::policy_from_settings(ui, app.borrow().signer.policy());
    let saved = app.borrow_mut().signer.set_policy(policy);
    match saved {
        Ok(()) => view::load_settings(ui, app.borrow().signer.policy()),
        Err(error) => set_status(ui, &format!("Settings not saved: {error}")),
    }
}

/// The signing request in a code the Launcher's scanner matched to this app.
fn launcher_request(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let matched = MatchedQrResult::from_slice(bytes)
        .ok_or_else(|| "Could not read the code the Launcher scanned.".to_owned())?;
    match matched.scan_result {
        ScanQrResult::Ur2 { ur_type, data, .. } => qr::request_payload(&ur_type, data),
        _ => Err("That code is not a Fedimint signing request.".to_owned()),
    }
}

/// Reads the app seed and opens the signer over its stored state.
fn open(ui: &AppWindow) -> Option<Rc<RefCell<App>>> {
    let seed = match device::app_seed() {
        Ok(seed) => Zeroizing::new(seed),
        Err(error) => {
            log::error!("{error}");
            set_status(
                ui,
                "The app cannot read its key. Reopen it and allow access to its seed.",
            );
            return None;
        }
    };

    #[cfg(not(keyos))]
    if bridge::take_reset() {
        match device::AppDataStorage.wipe() {
            Ok(()) => log::warn!("simulator: forgot the stored signer state, as the bridge asked"),
            Err(error) => log::error!("simulator: {error}"),
        }
    }

    let signer = match Signer::open(*seed, device::AppDataStorage, device::DevicePlatform) {
        Ok(signer) => signer,
        Err(error) => {
            log::error!("cannot open the signer: {error}");
            set_status(ui, &format!("Cannot open the signer: {error}"));
            return None;
        }
    };
    drop(seed);

    let app = Rc::new(RefCell::new(App {
        signer,
        waiting: None,
        showing_result: false,
        screens: VecDeque::new(),
        bridge: None,
    }));
    view::refresh_home(ui, &app.borrow().signer);
    view::load_settings(ui, app.borrow().signer.policy());
    let bridge = bridge::start(ui.as_weak(), app.clone());
    app.borrow_mut().bridge = bridge;
    log::info!("signer open");
    Some(app)
}

/// How long after the window first becomes visible the signer opens; see `app_main`.
const OPEN_DELAY: Duration = Duration::from_millis(100);

/// The app once the signer is open; empty until then.
type Slot = Rc<OnceCell<Rc<RefCell<App>>>>;

/// A UI callback that runs `action` once the signer is open, and does nothing before.
fn when_open(
    ui: &AppWindow,
    slot: &Slot,
    action: fn(&AppWindow, &Rc<RefCell<App>>),
) -> impl Fn() + 'static {
    let (ui, slot) = (ui.as_weak(), slot.clone());
    move || {
        if let (Some(ui), Some(app)) = (ui.upgrade(), slot.get()) {
            action(&ui, app);
        }
    }
}

fn app_main(cx: AppContext, ui: AppWindow) {
    log_server::init_wait(env!("CARGO_CRATE_NAME")).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("Fedimint Signer starting");

    theme::init(&ui);
    ui.global::<Utils>()
        .on_qrcode(slint_keyos_platform::qrcode::render);

    let slot: Slot = Rc::default();
    let callbacks = ui.global::<Callbacks>();
    callbacks.on_scan_request(when_open(&ui, &slot, scan_request));
    callbacks.on_approve(when_open(&ui, &slot, approve));
    callbacks.on_reject(when_open(&ui, &slot, reject));
    callbacks.on_next(when_open(&ui, &slot, next_screen));
    callbacks.on_settings_changed(when_open(&ui, &slot, settings_changed));

    // The signer opens once the window is on screen. The first read of the app seed shows the
    // system's consent prompt, and KeyOS presents it only over a foreground window, which a window
    // becomes when its first frame arrives. Read before that (before `ui.run()`, or straight from
    // the `Visible` event, which comes before the first frame is drawn) and the call waits forever
    // on a dark screen. So the read waits for a short timer started on `Visible`: the event loop
    // draws and submits the first frame before any timer can fire (Foundation SDK migration
    // guide, gotcha 24).
    //
    // The Launcher's scanner can also hand over a request it matched to this app. One that
    // arrives before the signer is open waits for it.
    let launched: Rc<RefCell<Option<Vec<u8>>>> = Rc::default();
    let opening = Rc::new(Cell::new(false));
    cx.set_input_handler({
        let gui = cx.gui.clone();
        let ui = ui.as_weak();
        move |input| {
            let Some(window) = ui.upgrade() else { return };
            match input.msg {
                InputMessage::Visible if slot.get().is_none() && !opening.get() => {
                    opening.set(true);
                    let (ui, slot, launched, opening) =
                        (ui.clone(), slot.clone(), launched.clone(), opening.clone());
                    Timer::single_shot(OPEN_DELAY, move || {
                        let Some(ui) = ui.upgrade() else { return };
                        let Some(app) = open(&ui) else {
                            // Try again the next time the app comes to the foreground.
                            opening.set(false);
                            return;
                        };
                        let _ = slot.set(app.clone());
                        let request = launched.borrow_mut().take();
                        if let Some(request) = request {
                            handle_message(&ui, &app, &request, Channel::Qr);
                        }
                    });
                }
                InputMessage::NavigationFocused => {
                    let Ok(Some(bytes)) = gui.navigate_pending() else {
                        return;
                    };
                    match (launcher_request(&bytes), slot.get()) {
                        (Ok(request), Some(app)) => {
                            handle_message(&window, app, &request, Channel::Qr)
                        }
                        (Ok(request), None) => *launched.borrow_mut() = Some(request),
                        (Err(message), _) => set_status(&window, &message),
                    }
                }
                _ => {}
            }
        }
    });

    ui.run().expect("UI running");
}
