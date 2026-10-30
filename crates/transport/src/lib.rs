//! Transports for the Prime Fedimint signer protocol.
//!
//! `fps-protocol` defines the bytes; this crate moves them. The KeyOS app and the programs that talk
//! to it share this code, so both ends agree on framing by construction.
//!
//! - [`ur`]: animated QR codes. Each envelope travels as one multi-part Uniform Resource of type
//!   `fps-request` or `fps-response`.
//! - [`bridge`]: the simulator file bridge, a host directory shared with the KeyOS simulator.
//! - [`net`]: the simulator network link, a TCP stand-in for QuantumLink while there is no device.
//!   Simulator builds only.

pub mod bridge;
#[cfg(not(target_os = "xous"))]
pub mod net;
pub mod ur;
