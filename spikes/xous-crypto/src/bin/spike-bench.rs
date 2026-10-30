//! Host cost baseline for the primitives the signer runs per request.
//!
//! Run with `cargo run --release -p fps-spike-xous-crypto --bin spike-bench`. Device numbers need
//! hardware; see docs/PHASES.md for how to read these against the Cortex-A5.

use std::time::Instant;

use bls12_381::Scalar;
use fps_spike_xous_crypto as spike;

fn time<F: FnMut(u64)>(label: &str, iterations: u64, mut f: F) {
    let start = Instant::now();
    for i in 0..iterations {
        f(i);
    }
    let per_op = start.elapsed() / iterations as u32;
    println!("{label:<38} {iterations:>5} runs  {per_op:>12.2?} per op");
}

fn main() {
    let digest = [7u8; 32];
    let aux = [9u8; 32];
    let mut secret = [1u8; 32];

    time("schnorr sign (BIP-340)", 200, |i| {
        secret[31] = (i % 250) as u8 + 1;
        assert!(spike::schnorr_sign(&secret, &digest, &aux).is_some());
    });
    time("hash to G1, SHA-256 (mint v2 nonce)", 200, |i| {
        let _ = spike::hash_to_g1_sha256(&i.to_le_bytes());
    });
    time("hash to G1, SHA3 (mint v1 nonce)", 200, |i| {
        let _ = spike::hash_to_g1_sha3(&i.to_le_bytes());
    });
    let msg = spike::hash_to_g1_sha256(b"blind");
    time("blind message (G1 scalar mult)", 200, |i| {
        let _ = spike::blind(&msg, &Scalar::from(i + 2));
    });
    time("BLS sign + verify (two pairings)", 20, |i| {
        assert!(spike::bls_sign_and_verify(i + 3, b"note nonce"));
    });
}
