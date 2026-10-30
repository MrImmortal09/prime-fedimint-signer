//! The simulator screen, through one long-lived `foundation-passport-drive mcp --sim` process.
//!
//! The simulator's debug channel leaks a kernel thread slot for every client connection
//! (Foundation SDK 1.1.0): about 28 separate `foundation-passport-drive --sim` calls exhaust the
//! 32-slot table and the hosted kernel panics. The MCP server keeps one connection for the whole
//! run, so the driver talks to it over stdio (JSON-RPC, one message per line).
//!
//! Taps go to fixed points: the action buttons sit at the bottom of the 480 × 800 screen, so the
//! points only move when the app's layout does. The driver never waits a fixed time for a screen:
//! the app logs each screen it shows, and the MCP server returns the simulator's log lines.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

/// The review screen's Reject and Approve buttons, and the result screen's Done button.
pub const REJECT: (u32, u32) = (124, 742);
pub const APPROVE: (u32, u32) = (356, 742);
pub const DONE: (u32, u32) = (240, 742);

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
                "clientInfo": { "name": "fps-sim-e2e", "version": env!("CARGO_PKG_VERSION") }
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

    /// Waits for a log line containing `needle` since the last [`Screen::clear_logs`]. The app
    /// logs every screen it shows, so this is how the driver knows a screen is up.
    pub fn wait_for_log(&mut self, needle: &str) -> Result<(), String> {
        let deadline = Instant::now() + SCREEN_TIMEOUT;
        loop {
            let content = self.tool("get_logs", json!({ "filter": needle, "max_lines": 100 }))?;
            let seen = content
                .iter()
                .filter_map(|item| item["text"].as_str())
                .any(|text| text.contains(needle));
            if seen {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "no log line with {needle:?} within {SCREEN_TIMEOUT:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
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

/// The text of the QR code in a screenshot.
pub fn read_qr(path: &Path) -> Result<String, String> {
    let image = image::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?
        .to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(
        image.width() as usize,
        image.height() as usize,
        |x, y| image.get_pixel(x as u32, y as u32).0[0],
    );
    let grids = prepared.detect_grids();
    let grid = grids
        .first()
        .ok_or_else(|| format!("no QR code in {}", path.display()))?;
    grid.decode()
        .map(|(_, text)| text)
        .map_err(|error| format!("the QR code in {} does not decode: {error}", path.display()))
}
