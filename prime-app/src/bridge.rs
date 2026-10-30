//! The simulator file bridge: a host directory the hosted simulator app shares with test drivers
//! and laptop wallets (`fps_transport::bridge` documents the layout). Requests from it go through
//! the same handling as scanned ones, and their reviews show on the simulator screen. Device builds
//! have no bridge.

#[cfg(not(keyos))]
mod hosted {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use fps_transport::bridge::Bridge;
    use slint_keyos_platform::slint::{Timer, TimerMode, Weak};

    use crate::{App, AppWindow, Channel};

    /// How often the inbox is checked.
    const POLL: Duration = Duration::from_millis(250);

    /// The app seed a test pinned in the bridge directory, if any.
    pub fn pinned_app_seed() -> Result<Option<[u8; 32]>, String> {
        Bridge::from_env().pinned_app_seed()
    }

    /// Whether a test asked for a clean start.
    pub fn take_reset() -> bool {
        Bridge::from_env().take_reset()
    }

    /// Starts polling the inbox. The bridge runs while the returned timer lives.
    pub fn start(ui: Weak<AppWindow>, app: Rc<RefCell<App>>) -> Option<Timer> {
        let bridge = Bridge::from_env();
        if let Err(error) = bridge.create() {
            log::error!(
                "simulator bridge disabled: cannot create {}: {error}",
                bridge.root().display()
            );
            return None;
        }
        log::info!("simulator bridge watching {}", bridge.inbox().display());
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, POLL, move || {
            let Some(ui) = ui.upgrade() else { return };
            // One request at a time: a review or a result on screen holds the queue.
            if app.borrow().busy() {
                return;
            }
            match bridge.take_request() {
                Ok(Some(bytes)) => crate::handle_message(&ui, &app, &bytes, Channel::Bridge),
                Ok(None) => {}
                Err(error) => log::warn!("simulator bridge: cannot take a request: {error}"),
            }
        });
        Some(timer)
    }

    pub fn write_reply(reply: &[u8]) {
        if let Err(error) = Bridge::from_env().write_response(reply) {
            log::error!("simulator bridge: cannot write the answer: {error}");
        }
    }
}

#[cfg(not(keyos))]
pub use hosted::{pinned_app_seed, start, take_reset, write_reply};

#[cfg(keyos)]
pub fn start(
    _ui: slint_keyos_platform::slint::Weak<crate::AppWindow>,
    _app: std::rc::Rc<std::cell::RefCell<crate::App>>,
) -> Option<slint_keyos_platform::slint::Timer> {
    None
}

#[cfg(keyos)]
pub fn write_reply(_reply: &[u8]) {
    log::error!("the simulator bridge does not exist on the device");
}
