# Phases

Each phase maps to a stage of [`plan/05_EXECUTION_PLAN.md`](plan/05_EXECUTION_PLAN.md). Stage 1 is
split into 1A (host-testable crates) and 1B (the KeyOS app) because it is the largest stage.

| Phase | Plan stage | Scope | State |
|---|---|---|---|
| 0 | Stage 0 | Repository, plan, device toolchain spike, CI | Done |
| 1A | Stage 1 | Wire protocol, signer core, policy, signer service, golden vectors | Next |
| 1B | Stage 1 | KeyOS app on the simulator with the file bridge | Planned |
| 2 | Stage 2 | fedimint client signer boundary, end to end on a local federation | Planned |
| 3 | Stage 3 | Phone app, QR/UR, hardware | Planned |
| 4 | Stage 4 | Bluetooth through a KeyOS QuantumLink patch | Planned |
| 5 | Stage 5 | NFC tap-to-approve (optional) | Planned |
| 6 | Stage 6 | Hardening and upstreaming | Planned |

## Phase 0: scaffold and toolchain spike

### Delivered

- This repository, with the planning documents under `docs/plan/`.
- `spikes/xous-crypto`: every primitive the signer core depends on, in one crate: BIP-340 Schnorr
  over secp256k1, BLS12-381 pairings, fedimint's two hash-to-curve variants, G1 blinding, CBOR, and
  BOLT11 parsing.
- `scripts/sdk-shell.sh` and `scripts/device-build.sh` to run the Foundation SDK toolchain
  non-interactively.
- CI for formatting, lints and host tests.

### Device build result

The spike's full dependency set compiles for `armv7a-unknown-xous-elf` with the SDK's nightly
(`rustc 1.96.0-nightly 2026-04-10`):

| Crate | Version | Notes |
|---|---|---|
| `bitcoin` / `secp256k1` / `secp256k1-sys` | 0.32 / 0.29 / 0.10 | C code built by the shell's `/usr/bin/cc` into `elf32-littlearm` objects |
| `bls12_381`, `group`, `pairing` | 0.8 / 0.13 / 0.23 | Pure Rust |
| `sha3`, `rand_chacha`, `rand_core` | 0.10 / 0.3 / 0.6 | Same versions fedimint 0.13 uses |
| `ciborium`, `serde`, `serde_bytes` | 0.2 / 1 / 0.11 | Wire format |
| `lightning-invoice` | 0.33 | BOLT11 parsing with signature check |

Two facts the later phases rely on:

- The target is a custom target spec in the SDK sysroot, so `RUSTFLAGS` must include
  `-Zunstable-options`, as `foundation build` does. `scripts/device-build.sh` sets the same flags.
- This proves compilation only. Linking a complete app is verified with `foundation build` in
  Phase 1B.

### Host cost baseline

Apple M4, release build, `cargo run --release -p fps-spike-xous-crypto --bin spike-bench`:

| Operation | Per op |
|---|---|
| Schnorr sign (BIP-340) | 72 µs |
| Hash to G1, SHA-256 variant (mint v2 nonce) | 152 µs |
| Hash to G1, SHA3 variant (mint v1 nonce) | 101 µs |
| Blind a message (G1 scalar multiplication) | 222 µs |
| BLS sign and verify (two pairings) | 2.1 ms |

Device timings need hardware and are not measured yet. A 500 MHz in-order Cortex-A5 doing 32-bit
big-number arithmetic is commonly 50 to 150 times slower than this host, which would put one BLS
note verification at roughly 0.1 to 0.3 seconds. That is why the signer core makes BLS checks on
spends optional and capped, and uses them where they matter: ecash releases and offline receipt.

### Findings that changed the plan

Reading fedimint 0.13.0-beta.1 during this phase produced four corrections, recorded in
[`plan/10_DECISIONS.md`](plan/10_DECISIONS.md):

- **D8 revised:** fedimint 0.13 federations run mint v2, wallet v2 and LNv2 by default. The first
  version targets that shape; the v1 modules come later.
- **D11 added:** a released non-hardened key plus the spend xpub reveals the root key, so ecash
  tokens only ever carry keys from a hardened outgoing branch.
- **D12 added:** the federation id binds only the guardian endpoints, so the module configs Prime
  imports are trust-on-first-use from the paired phone. Spending checks do not depend on them.
- **D13 added:** receives stay phone-only, which means a compromised phone can intercept funds that
  are still arriving.

### Still open from Stage 0

- Device timings: need hardware or a dev unit.
- Questions for Foundation listed at the end of `plan/05_EXECUTION_PLAN.md`.
- A local federation (devimint) is not needed until Phase 2.
