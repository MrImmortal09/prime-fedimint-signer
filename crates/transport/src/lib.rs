//! Transports for the Prime Fedimint signer protocol.
//!
//! `fps-protocol` defines the bytes; this crate moves them. The KeyOS app and the programs that talk
//! to it share this code, so both ends agree on framing by construction.
//!
//! - [`ur`]: animated QR codes. Each envelope travels as one multi-part Uniform Resource of type
//!   `fps-request` or `fps-response`.
//! - [`bridge`]: the simulator file bridge, a host directory shared with the KeyOS simulator.

pub mod bridge;
pub mod ur;
