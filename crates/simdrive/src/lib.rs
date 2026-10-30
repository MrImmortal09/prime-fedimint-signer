//! The KeyOS simulator's screen, through one long-lived `foundation-passport-drive mcp --sim`
//! process, for end-to-end tests of the Fedimint signer app.
//!
//! The simulator's debug channel leaks a kernel thread slot for every client connection
//! (Foundation SDK 1.1.0): about 28 separate `foundation-passport-drive --sim` calls exhaust the
//! 32-slot table and the hosted kernel panics. The MCP server keeps one connection for the whole
//! run, so tests talk to it over stdio (JSON-RPC, one message per line).
//!
//! Taps go to fixed points: the app's action buttons sit at the bottom of the 480 × 800 screen, so
//! the points only move when the app's layout does. Nothing waits a fixed time for a screen: the
//! app logs each screen it shows, the MCP server returns the simulator's log lines, and
//! [`Screen::wait_for_new_screen`] and [`Screen::wait_until`] wait until the new screen is drawn
//! and still.
//!
//! [`Hand`] is a person at the simulator on a thread of its own, for tests where something else,
//! such as a wallet, sends the requests.

mod hand;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

pub use hand::{Decision, Hand, Seen};

/// The review screen's Reject and Approve buttons, and the result screen's Done button.
pub const REJECT: (u32, u32) = (124, 742);
pub const APPROVE: (u32, u32) = (356, 742);
pub const DONE: (u32, u32) = (240, 742);

/// Points inside the bottom buttons, clear of their labels: the review screen's Reject (left) and
/// Approve (right), both inside a result screen's Done.
const LEFT_BUTTON: (u32, u32) = (60, 760);
const RIGHT_BUTTON: (u32, u32) = (420, 760);

/// What the app logs when it shows a review or a result screen, before the screen's title.
const REVIEW_LOG: &str = "review on screen: ";
const RESULT_LOG: &str = "result on screen: ";

/// The system PIN pad: digits 0 to 9, then Unlock.
const PIN_DIGITS: [(u32, u32); 10] = [
    (240, 756),
    (85, 535),
    (240, 535),
    (395, 535),
    (85, 608),
    (240, 608),
    (395, 608),
    (85, 682),
    (240, 682),
    (395, 682),
];
const PIN_UNLOCK: (u32, u32) = (395, 756);
/// Time between PIN pad taps.
const KEY_INTERVAL: Duration = Duration::from_millis(300);

/// The longest the driver waits for a screen to appear.
const SCREEN_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Screen {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    shots: PathBuf,
    taken: u32,
}

impl Screen {
    pub fn new(drive: PathBuf, shots: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&shots)
            .map_err(|error| format!("cannot create {}: {error}", shots.display()))?;
        let mut child = Command::new(&drive)
            .args(["mcp", "--sim"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot run {}: {error}", drive.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        let mut screen = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
            shots,
            taken: 0,
        };
        screen.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "fps-simdrive", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        screen.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        screen.tool("connect", json!({}))?;
        Ok(screen)
    }

    fn send(&mut self, message: Value) -> Result<(), String> {
        writeln!(self.stdin, "{message}")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("passport-drive went away: {error}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("passport-drive went away: {error}"))?;
            if read == 0 {
                return Err("passport-drive exited".to_owned());
            }
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply["id"] != json!(id) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(format!("passport-drive {method}: {error}"));
            }
            return Ok(reply["result"].clone());
        }
    }

    /// Calls a passport-drive tool and returns its content items.
    fn tool(&mut self, name: &str, arguments: Value) -> Result<Vec<Value>, String> {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        let content = result["content"].as_array().cloned().unwrap_or_default();
        if result["isError"] == json!(true) {
            let text: Vec<&str> = content
                .iter()
                .filter_map(|item| item["text"].as_str())
                .collect();
            return Err(format!("passport-drive {name}: {}", text.join(" ")));
        }
        Ok(content)
    }

    pub fn tap(&mut self, (x, y): (u32, u32)) -> Result<(), String> {
        self.tool("tap", json!({ "x": x, "y": y })).map(|_| ())
    }

    /// Forgets the log lines collected so far, so [`Screen::wait_for_log`] sees only new ones.
    pub fn clear_logs(&mut self) -> Result<(), String> {
        self.tool("clear_logs", json!({})).map(|_| ())
    }

    /// The simulator's log lines containing `filter` since the last [`Screen::clear_logs`],
    /// oldest first.
    pub fn log_lines(&mut self, filter: &str) -> Result<Vec<String>, String> {
        let content = self.tool("get_logs", json!({ "filter": filter, "max_lines": 100 }))?;
        Ok(content
            .iter()
            .filter_map(|item| item["text"].as_str())
            .flat_map(str::lines)
            .filter(|line| line.contains(filter))
            .map(str::to_owned)
            .collect())
    }

    /// Waits for a log line containing `needle` since the last [`Screen::clear_logs`]. The app
    /// logs every screen it shows, so this is how the driver knows a screen is up.
    pub fn wait_for_log(&mut self, needle: &str) -> Result<(), String> {
        let deadline = Instant::now() + SCREEN_TIMEOUT;
        while self.log_lines(needle)?.is_empty() {
            if Instant::now() >= deadline {
                return Err(format!(
                    "no log line with {needle:?} within {SCREEN_TIMEOUT:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    /// The first review or result screen the app logged since the last [`Screen::clear_logs`].
    /// The app takes no new request while one is up, so there is at most one until it is dealt
    /// with.
    pub fn shown(&mut self) -> Result<Option<Shown>, String> {
        Ok(self.log_lines("on screen: ")?.iter().find_map(|line| {
            if let Some((_, title)) = line.split_once(REVIEW_LOG) {
                return Some(Shown::Review(title.trim().to_owned()));
            }
            line.split_once(RESULT_LOG)
                .map(|(_, title)| Shown::Result(title.trim().to_owned()))
        }))
    }

    /// The current screen as PNG bytes.
    pub fn capture(&mut self) -> Result<Vec<u8>, String> {
        let content = self.tool("screenshot", json!({}))?;
        let data = content
            .iter()
            .find(|item| item["type"] == "image")
            .and_then(|item| item["data"].as_str())
            .ok_or("passport-drive screenshot: no image")?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| format!("passport-drive screenshot: {error}"))
    }

    /// Waits until the screen shows something other than `before`, then stops changing. A
    /// screen is logged when it is requested, before it is drawn, and system modals slide in and
    /// out, so the log line alone is not enough to tap or read.
    pub fn wait_for_new_screen(&mut self, before: &[u8]) -> Result<(), String> {
        let deadline = Instant::now() + SCREEN_TIMEOUT;
        let mut previous = self.capture()?;
        while previous == before {
            if Instant::now() >= deadline {
                return Err(format!(
                    "the screen did not change within {SCREEN_TIMEOUT:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
            previous = self.capture()?;
        }
        loop {
            std::thread::sleep(Duration::from_millis(150));
            let current = self.capture()?;
            if current == previous {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("the screen kept changing for {SCREEN_TIMEOUT:?}"));
            }
            previous = current;
        }
    }

    /// Waits until `ready` holds for two screenshots in a row that are also identical, so the
    /// screen is drawn and still, and returns the last one.
    pub fn wait_until(&mut self, ready: impl Fn(&[u8]) -> bool) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + SCREEN_TIMEOUT;
        let mut previous: Option<Vec<u8>> = None;
        loop {
            let current = self.capture()?;
            if ready(&current) && previous.as_ref() == Some(&current) {
                return Ok(current);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the screen did not reach the expected state within {SCREEN_TIMEOUT:?}"
                ));
            }
            previous = Some(current);
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// Types `pin` on the system PIN pad and unlocks. The pad drops taps that come faster than a
    /// person types.
    pub fn enter_pin(&mut self, pin: &str) -> Result<(), String> {
        for digit in pin.chars() {
            let index = digit
                .to_digit(10)
                .ok_or_else(|| format!("the PIN {pin:?} is not all digits"))?;
            self.tap(PIN_DIGITS[index as usize])?;
            std::thread::sleep(KEY_INTERVAL);
        }
        self.tap(PIN_UNLOCK)
    }

    /// Saves a numbered screenshot and returns its path.
    pub fn shot(&mut self, name: &str) -> Result<PathBuf, String> {
        let png = self.capture()?;
        self.save(name, &png)
    }

    /// Saves `png`, a screenshot taken earlier, as the next numbered one and returns its path.
    pub fn save(&mut self, name: &str, png: &[u8]) -> Result<PathBuf, String> {
        self.taken += 1;
        let path = self.shots.join(format!("{:02}-{name}.png", self.taken));
        std::fs::write(&path, png)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(path)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.tool("disconnect", json!({}));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The text of the QR code in a saved screenshot.
pub fn read_qr(path: &Path) -> Result<String, String> {
    let png =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    read_qr_png(&png).map_err(|error| format!("{}: {error}", path.display()))
}

/// The text of the QR code in a screenshot's PNG bytes.
pub fn read_qr_png(png: &[u8]) -> Result<String, String> {
    let image = image::load_from_memory(png)
        .map_err(|error| format!("not a screenshot: {error}"))?
        .to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(
        image.width() as usize,
        image.height() as usize,
        |x, y| image.get_pixel(x as u32, y as u32).0[0],
    );
    let grids = prepared.detect_grids();
    let grid = grids.first().ok_or("no QR code on the screen")?;
    grid.decode()
        .map(|(_, text)| text)
        .map_err(|error| format!("the QR code does not decode: {error}"))
}

/// A review or result screen the app logged, with its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    Review(String),
    Result(String),
}

/// The review screen is drawn: a grey Reject button beside a yellow Approve button.
pub fn shows_review(png: &[u8]) -> bool {
    matches!(button_colours(png), Some((left, right)) if is_grey(left) && is_accent(right))
}

/// A result screen is drawn: one yellow Done button across the screen.
pub fn shows_result(png: &[u8]) -> bool {
    matches!(button_colours(png), Some((left, right)) if is_accent(left) && is_accent(right))
}

/// The colours inside the left and the right bottom button.
fn button_colours(png: &[u8]) -> Option<([u8; 3], [u8; 3])> {
    let image = image::load_from_memory(png).ok()?.to_rgb8();
    let colour = |(x, y): (u32, u32)| image.get_pixel_checked(x, y).map(|pixel| pixel.0);
    Some((colour(LEFT_BUTTON)?, colour(RIGHT_BUTTON)?))
}

/// The yellow of the app's main buttons: Approve, Done.
fn is_accent([red, green, blue]: [u8; 3]) -> bool {
    red > 150 && green > 120 && blue < 110
}

/// The grey of its other buttons: Reject, and Scan request on the home screen.
fn is_grey([red, green, blue]: [u8; 3]) -> bool {
    (60..=160).contains(&red) && red.abs_diff(green) < 12 && red.abs_diff(blue) < 12
}
