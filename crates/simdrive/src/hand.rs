//! A person at the simulator, on a thread of its own.
//!
//! Tests where something else sends the requests, such as a wallet over the file bridge, cannot
//! tap in step with each request. The hand watches the app's log instead: it decides each review
//! as told (approving unless told otherwise), reads the QR code off each result screen and taps
//! Done, and keeps what it saw for the test to check.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{read_qr_png, shows_result, shows_review, Screen, Shown, APPROVE, DONE, REJECT};

/// How often the hand looks for a new screen.
const POLL: Duration = Duration::from_millis(100);

/// What the hand does with a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
}

/// A screen the hand dealt with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// A review, and what the hand decided.
    Review { title: String, decision: Decision },
    /// A result screen, and the text of the QR code on it if it showed one.
    Result { title: String, qr: Option<String> },
}

#[derive(Default)]
struct Shared {
    /// Decisions for the coming reviews, in order; the hand approves once they run out.
    decisions: Mutex<VecDeque<Decision>>,
    seen: Mutex<Vec<Seen>>,
    failure: Mutex<Option<String>>,
    stop: AtomicBool,
}

/// The state is plain data, so a panic elsewhere leaves nothing half-updated.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A person at the simulator. Dropping it stops it.
pub struct Hand {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Hand {
    /// Starts dealing with every review and result screen the app shows from now on.
    pub fn start(mut screen: Screen) -> Result<Self, String> {
        screen.clear_logs()?;
        let shared = Arc::new(Shared::default());
        let thread = std::thread::Builder::new()
            .name("simulator-hand".to_owned())
            .spawn({
                let shared = shared.clone();
                move || {
                    if let Err(error) = work(&mut screen, &shared) {
                        *lock(&shared.failure) = Some(error);
                    }
                }
            })
            .map_err(|error| format!("cannot start the hand: {error}"))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Queues what to do with the next review that has no decision yet.
    pub fn decide_next(&self, decision: Decision) {
        lock(&self.shared.decisions).push_back(decision);
    }

    /// Every screen dealt with so far, in order. A screen is recorded before the hand taps it
    /// away, so whoever waits for the app's answer sees it here once the answer arrives.
    pub fn seen(&self) -> Vec<Seen> {
        lock(&self.shared.seen).clone()
    }

    /// The titles of the reviews so far, in order.
    pub fn reviews(&self) -> Vec<String> {
        self.seen()
            .into_iter()
            .filter_map(|seen| match seen {
                Seen::Review { title, .. } => Some(title),
                Seen::Result { .. } => None,
            })
            .collect()
    }

    /// The QR codes read off result screens so far, in order.
    pub fn codes(&self) -> Vec<String> {
        self.seen()
            .into_iter()
            .filter_map(|seen| match seen {
                Seen::Result { qr, .. } => qr,
                Seen::Review { .. } => None,
            })
            .collect()
    }

    /// Why the hand stopped early, if it did. It stops at the first screen it cannot deal with.
    pub fn failure(&self) -> Option<String> {
        lock(&self.shared.failure).clone()
    }

    /// Stops the hand and returns what it saw, or why it stopped early.
    pub fn stop(mut self) -> Result<Vec<Seen>, String> {
        self.halt();
        match self.failure() {
            Some(error) => Err(error),
            None => Ok(self.seen()),
        }
    }

    fn halt(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                lock(&self.shared.failure).get_or_insert_with(|| "the hand panicked".to_owned());
            }
        }
    }
}

impl Drop for Hand {
    fn drop(&mut self) {
        self.halt();
    }
}

fn work(screen: &mut Screen, shared: &Shared) -> Result<(), String> {
    while !shared.stop.load(Ordering::SeqCst) {
        let Some(shown) = screen.shown()? else {
            std::thread::sleep(POLL);
            continue;
        };
        // The log line comes before the screen is drawn: wait for the screen, then tap.
        let (png, tap) = match shown {
            Shown::Review(title) => {
                let png = screen.wait_until(shows_review)?;
                screen.save(&format!("review-{}", slug(&title)), &png)?;
                let decision = lock(&shared.decisions)
                    .pop_front()
                    .unwrap_or(Decision::Approve);
                lock(&shared.seen).push(Seen::Review { title, decision });
                let tap = match decision {
                    Decision::Approve => APPROVE,
                    Decision::Reject => REJECT,
                };
                (png, tap)
            }
            Shown::Result(title) => {
                let png = screen.wait_until(shows_result)?;
                screen.save(&format!("result-{}", slug(&title)), &png)?;
                let qr = read_qr_png(&png).ok();
                lock(&shared.seen).push(Seen::Result { title, qr });
                (png, DONE)
            }
        };
        // Cleared before the tap: the app logs nothing new until the tap lands.
        screen.clear_logs()?;
        screen.tap(tap)?;
        screen
            .wait_for_new_screen(&png)
            .map_err(|error| format!("after the tap at {tap:?}: {error}"))?;
    }
    Ok(())
}

/// A screen title as part of a file name.
fn slug(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
