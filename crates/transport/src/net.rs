//! The simulator network link, a stand-in for QuantumLink.
//!
//! On the device, QuantumLink carries request and response envelopes over Bluetooth and the answer
//! comes back on the same connection. The hosted simulator has no radio: KeyOS's Bluetooth server
//! does nothing in hosted mode. So the simulator app listens on a TCP port instead, and a phone on
//! the same network connects to it. The bytes are the same envelopes, one exchange per connection:
//!
//! | Direction | Frame |
//! |---|---|
//! | phone to app | `u32` big-endian length, then the request envelope |
//! | app to phone | `u32` big-endian length, then the response envelope, once it is final |
//!
//! A request that needs approval keeps its connection open until the person at the simulator
//! decides. Requests that arrive while one is on screen wait in a queue, in order.
//!
//! There is no encryption and no authentication: the link exists only in simulator builds, for a
//! network you control. The device app never compiles it, and the device build of this crate
//! leaves the module out.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use fps_protocol::MAX_MESSAGE_SIZE;

/// Environment variable naming the address the simulator app listens on.
pub const ENV_ADDR: &str = "FPS_NET_ADDR";
/// The port when none is given, on either end.
pub const DEFAULT_PORT: u16 = 7865;
/// Where the simulator app listens when [`ENV_ADDR`] is unset: every interface, so a phone on
/// the network can reach it.
pub const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:7865";

/// How long the app waits for a connection to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the phone waits for the connection to open, and for its request to be taken.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Writes one frame: the length, then the bytes.
pub fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "a {} byte message is over the limit of {MAX_MESSAGE_SIZE}",
                bytes.len()
            ),
        ));
    }
    let len = u32::try_from(bytes.len()).expect("under the message limit");
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}

/// Reads one frame. A length over the protocol's message limit is refused before any of the
/// message is read.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a {len} byte message is over the limit of {MAX_MESSAGE_SIZE}"),
        ));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

// The app's end.

/// A connection whose request the app has taken. Its answer goes back on it.
#[derive(Debug)]
pub struct Connection {
    stream: TcpStream,
    peer: SocketAddr,
}

impl Connection {
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Sends the answer and closes the connection.
    pub fn answer(mut self, response: &[u8]) -> io::Result<()> {
        write_frame(&mut self.stream, response)?;
        self.stream.shutdown(Shutdown::Both)
    }
}

type Queue = Arc<Mutex<VecDeque<(Connection, Vec<u8>)>>>;

/// The app's end: accepts connections on a thread of its own, reads each one's request, and
/// keeps it until the app takes it. The accepting thread lives as long as the process.
#[derive(Debug)]
pub struct Listener {
    local: SocketAddr,
    queue: Queue,
}

impl Listener {
    /// Binds `addr` and starts accepting.
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let local = listener.local_addr()?;
        let queue: Queue = Arc::default();
        let accepting = queue.clone();
        thread::Builder::new()
            .name("fps-net-accept".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let queue = accepting.clone();
                    let _ = thread::Builder::new()
                        .name("fps-net-read".to_owned())
                        .spawn(move || receive(stream, &queue));
                }
            })?;
        Ok(Self { local, queue })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// The oldest request waiting, with the connection to answer on, if any.
    pub fn take_request(&self) -> Option<(Connection, Vec<u8>)> {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    /// How many requests wait to be taken.
    pub fn waiting(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

/// Reads one connection's request and queues it; a connection that sends nothing readable in
/// time is dropped.
fn receive(mut stream: TcpStream, queue: &Queue) {
    let Ok(peer) = stream.peer_addr() else { return };
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    match read_frame(&mut stream) {
        Ok(bytes) => {
            let _ = stream.set_read_timeout(None);
            queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back((Connection { stream, peer }, bytes));
        }
        Err(_) => {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

// The phone's end.

/// Resolves `address`, a host or `host:port`, adding the default port when there is none.
pub fn resolve(address: &str) -> io::Result<SocketAddr> {
    let address = address.trim();
    let with_port = if address.contains(':') {
        address.to_owned()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    };
    with_port.to_socket_addrs()?.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{address} names no address"),
        )
    })
}

/// Opens a connection to the simulator app at `address`.
pub fn connect(address: &str) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&resolve(address)?, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    Ok(stream)
}

/// Sends `request` on `stream` and waits up to `timeout` for the answer, which includes the time
/// someone takes to approve on the simulator's screen. A timeout is an error of kind
/// `WouldBlock` or `TimedOut`.
pub fn exchange_on(
    mut stream: TcpStream,
    request: &[u8],
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    write_frame(&mut stream, request)?;
    stream.set_read_timeout(Some(timeout))?;
    let answer = read_frame(&mut stream)?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(answer)
}

/// One exchange: connects to `address`, sends `request` and waits up to `timeout` for the answer.
pub fn exchange(address: &str, request: &[u8], timeout: Duration) -> io::Result<Vec<u8>> {
    exchange_on(connect(address)?, request, timeout)
}
