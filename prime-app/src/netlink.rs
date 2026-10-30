//! The simulator network link: a TCP stand-in for QuantumLink while the app runs in the hosted
//! simulator (`fps_transport::net` documents the framing). A phone on the same network sends a
//! request, the review shows on the simulator screen, and the answer goes back on the same
//! connection, as it will over Bluetooth. Device builds have no network link.

#[cfg(not(keyos))]
mod hosted {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use fps_transport::net::{DEFAULT_LISTEN_ADDR, ENV_ADDR, Listener};
    use slint_keyos_platform::slint::{Timer, TimerMode, Weak};

    use crate::{App, AppWindow, Channel};

    /// A connection waiting for its answer.
    pub type Pending = fps_transport::net::Connection;

    /// How often the queue of arrived requests is checked.
    const POLL: Duration = Duration::from_millis(250);

    /// Starts listening. The link runs while the returned timer lives.
    pub fn start(ui: Weak<AppWindow>, app: Rc<RefCell<App>>) -> Option<Timer> {
        let address = std::env::var(ENV_ADDR).unwrap_or_else(|_| DEFAULT_LISTEN_ADDR.to_owned());
        let listener = match Listener::bind(&address) {
            Ok(listener) => listener,
            Err(error) => {
                log::error!("simulator link disabled: cannot listen on {address}: {error}");
                return None;
            }
        };
        log::info!("simulator link listening on {}", listener.local_addr());
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, POLL, move || {
            let Some(ui) = ui.upgrade() else { return };
            // One request at a time: a review or a result on screen holds the queue.
            if app.borrow().busy() {
                return;
            }
            if let Some((connection, bytes)) = listener.take_request() {
                log::info!("simulator link: request from {}", connection.peer());
                app.borrow_mut().net_pending = Some(connection);
                crate::handle_message(&ui, &app, &bytes, Channel::Net);
            }
        });
        Some(timer)
    }

    /// Answers the connection whose request is being handled.
    pub fn write_reply(pending: Option<Pending>, reply: &[u8]) {
        match pending {
            Some(connection) => {
                if let Err(error) = connection.answer(reply) {
                    log::error!("simulator link: cannot send the answer: {error}");
                }
            }
            None => log::error!("simulator link: no connection waits for this answer"),
        }
    }
}

#[cfg(not(keyos))]
pub use hosted::{Pending, start, write_reply};

#[cfg(keyos)]
pub struct Pending;

#[cfg(keyos)]
pub fn start(
    _ui: slint_keyos_platform::slint::Weak<crate::AppWindow>,
    _app: std::rc::Rc<std::cell::RefCell<crate::App>>,
) -> Option<slint_keyos_platform::slint::Timer> {
    None
}

#[cfg(keyos)]
pub fn write_reply(_pending: Option<Pending>, _reply: &[u8]) {
    log::error!("the simulator network link does not exist on the device");
}
