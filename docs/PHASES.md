# Phases

Each phase maps to a stage of [`plan/05_EXECUTION_PLAN.md`](plan/05_EXECUTION_PLAN.md). Stages 1
and 2 are each split in two because they are the largest.

| Phase | Plan stage | Scope | State |
|---|---|---|---|
| 0 | Stage 0 | Repository, plan, device toolchain spike, CI | Done |
| 1A | Stage 1 | Wire protocol, signer core, policy, signer service, golden vectors | Done |
| 1B | Stage 1 | KeyOS app on the simulator with the file bridge | Done |
| 2A | Stage 2 | fedimint signer boundary and the watch-only client, on an in-process federation | Done |
| 2B | Stage 2 | fedimint-sdk bindings, devimint and the simulator, end to end | Next |
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
- This proves compilation only. Phase 1B links the complete app.

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

## Phase 1A: protocol, signer core, policy, signer service

### Delivered

| Crate | What it does |
|---|---|
| `crates/protocol` (`fps-protocol`) | The CBOR wire protocol: envelope, requests, responses, rejection codes. Spec: `PROTOCOL.md` |
| `crates/signer-core` (`fps-signer-core`) | fedimint 0.13 decoders for the v2 module shape, the `fps-v1` key scheme, transaction analysis, signing, ecash release, BOLT11 parsing. Spec: `KEY_SCHEME.md`, `SECURITY.md` |
| `crates/policy` (`fps-policy`) | Outflow limits, gateway fee ceiling, PIN threshold |
| `crates/signer` (`fps-signer`) | The service the KeyOS app embeds: approvals, replay, the ledger of released notes, staged notes and funded contracts, durable persistence |
| `devtools` (`fps-devtools`) | Host-only workspace linking the real fedimint 0.13.0-beta.1 crates: the golden vector generator and live cross-checks |

### How it is verified

| Check | Result |
|---|---|
| Golden vectors produced by fedimint itself (integers, hash-to-curve, base32, federation ids, mint v2 inputs and outputs, note signatures, LNv2 contracts and inputs, wallet v2 peg-outs, ecash tokens) | Every decoder reads fedimint's bytes and every encoder reproduces them |
| Six full scenarios built with fedimint types (reissue, pay invoice, refund, withdraw, prepare send, reclaim) | Same transaction ids, same fee totals, valid signatures |
| Live cross-check: device signatures checked by fedimint's `Transaction::validate_signatures` | Accepted for reissue, Lightning payment, withdrawal, refund and staged send; a swapped signature is refused |
| Live cross-check: released tokens decoded by fedimint's `ECash` | Decode, carry the right amount, invite and keys; every note verifies with `verify_note` |
| 40 random fedimint-built transactions | Decode, re-encode and hash identically |
| Behaviour tests | analysis 23, encoding 8, federation 7, invoice 4, key scheme 5, release 5, protocol 8, policy 6, service 14 |
| Device build | All four production crates build for `armv7a-unknown-xous-elf` in release mode |

Run them with `cargo test --workspace`, and `cargo test --release --manifest-path devtools/Cargo.toml`
for the live cross-checks. `cargo run --release --manifest-path devtools/Cargo.toml --bin golden`
regenerates the vectors; CI checks they are unchanged.

### Findings

- **Refund contract substitution.** A refund input names an outpoint the device cannot look up, so a
  phone could pair it with a cheaper contract and have the federation keep the difference as fees. The
  service now records every contract it funds by outpoint and refuses a mismatching refund.
- **Fee overpayment.** fedimint 0.13 accepts transactions that overpay fees. The analysis computes the
  module fees from the federation's fee rules and refuses overpayment beyond 1,000 msat.
- **Recovery-safe outputs.** Change and staged notes must use tweaks that pass the wallet's recovery
  filter, or a later recovery would miss them.
- **macOS release builds.** Cargo's default release stripping corrupts host proc-macro dylibs built with
  the SDK toolchain on current macOS. The workspace sets `strip = false`; `foundation build` strips the
  device ELF itself. The KeyOS app in Phase 1B needs the same setting.
- **32-bit lengths.** Collection counts are range-checked rather than cast, since `usize` is 32 bits on
  the device.

### Still open

- Device timings for signing and BLS checks need hardware.
- The phone side of the key scheme lands with the fedimint client changes in Phase 2.
- Mint v1, wallet v1 and LN v1 decoders for older federations.

## Phase 1B: the KeyOS app

### Delivered

| Path | What it is |
|---|---|
| `prime-app/` | The KeyOS app `fedimint-signer`, on Foundation SDK 1.1.0 and Slint: a home screen listing the imported federations, and review, result and settings screens |
| `crates/transport/` (`fps-transport`) | Both ends of both transports: animated-QR framing and reassembly, and the simulator file bridge |
| `scripts/app-device-build.sh` | Compiles and links the app for Passport Prime without signing it |

How the app works:

- **Requests in.** The system QR scanner, opened from the app or from the Launcher, which hands
  `fps-request` codes to the app through a `[[qr-match-rules]]` entry. In the simulator, the file
  bridge as well.
- **Every decision is the service's.** The app passes the request bytes to `fps-signer` and shows the
  review it returns. Nothing on screen comes from labels the phone supplied.
- **Gates.** Approve or Reject on the review screen. Spends above 100,000 sats also need the device PIN,
  which the system's lock screen asks for.
- **Answers out.** An animated `fps-response` code for a scanned request, or the bridge. A
  `ShowOnDevice` release shows the token as one QR code for the receiver and never sends it to the
  phone.
- **One request at a time.** A review or a result on screen holds the bridge queue.
- **State.** One durable file in the app's encrypted `AppData`, written before each answer leaves.
- **Settings.** Three switches: the PIN above 100,000 sats, mint signature checks on spends, and
  releasing prepared notes without a second review.

### Device build

`scripts/app-device-build.sh` compiles and links the app for `armv7a-unknown-xous-elf` with the
flags `foundation build` uses. The result is a static PIE executable of 4.9 MB once stripped. The
only `getrandom` in the device dependency graph is the SDK's TRNG-backed fork. Signing needs a
publisher identity (`foundation cert gen`), so `foundation build`, sideloading and device timings
wait for hardware in Phase 3.

### How it is verified

`scripts/sim-e2e.sh` builds the app, starts the simulator and plays the golden scenario through the
file bridge and the screen (`tools/sim-e2e`). It reads the results the way a phone would, checks them
on its own, restarts the simulator, and checks again.

| Check | Result |
|---|---|
| The scenario on a fresh app | 19 checks: import, view material, nonces, reissue, a Lightning payment rejected and then approved, refund, a withdrawal signed only after the system PIN, prepared send, release shown on screen, release to the phone, reclaim, replay, malformed input, status |
| Signatures from the simulator | Every one verifies with secp256k1 against the transaction id fedimint computed |
| The token shown on screen | Read off a screenshot as a QR code: exactly the token the scenario's notes make, invite included |
| After a simulator restart | 4 checks: the federation, the stored answers and the released-notes ledger survived in `AppData` |
| Repeatability | Two consecutive runs pass, and the simulator leaves no processes behind |
| `fps-transport` | 23 tests: UR round trips in any order and case, size limits checked before reassembly, bridge ordering, atomic files, the private seed file, reset |
| Release with the federation's real mint signatures | `fps-signer` service test with signature checks on; a signature swapped between notes is refused |
| First-run consent prompt | Checked by hand in the simulator: the prompt shows over the home screen, and the signer opens after Allow |

`cargo test --workspace` runs the host tests. The simulator check needs the Foundation SDK, so it
runs locally rather than in CI. The golden generator now also records the mint signatures of the
outgoing notes the scenario stages and reclaims, so releases are tested with real signatures.

### Findings

- **The app seed can only be read once the window is on screen.** The first `GetAppSeed` asks the
  user for consent, and KeyOS shows that prompt only over a foreground window. A window becomes
  foreground when its first frame arrives. Reading the seed before `ui.run()`, from a zero-delay
  timer, or inside the `Visible` event all block before that frame, and the app hangs on a dark
  screen. The app opens the signer from a short timer started on `Visible`, which fires after the
  event loop has drawn the first frame. The SDK's gotcha 24 covers the first case; the others look
  like fixes but are not.
- **The getrandom patch had gone missing.** `prime-app/Cargo.toml` had lost the scaffold's patch to
  the SDK's TRNG-backed `getrandom`, so the build used the crates.io release, which has no Xous
  backend. It is restored.
- **The simulator's debug channel leaks a kernel thread slot per connection.** About 28 separate
  `foundation-passport-drive --sim` calls fill the 32-slot thread table, and the hosted kernel
  panics. One `foundation-passport-drive mcp --sim` session survives any number of calls, so the test
  driver keeps one open. Worth reporting to Foundation.
- **The system PIN pad drops fast taps, and a closing system modal swallows taps meant for the app.**
  The test driver types at human speed and waits for each screen to be drawn and still.
- **UR frame count.** The fountain encoder picks its own fragment length, so the number of pure parts
  comes from the encoder, not from dividing the message length.
- **`GetRandom` is not needed.** The SDK's `getrandom` talks to the TRNG server directly, so the app
  no longer asks for that permission.

### Still open

- Signing, sideloading and on-device timings.
- The 256 KiB device stack. The simulator cannot show an overflow (SDK gotcha 25); `foundation build`
  reports large stack frames once the app can be signed.
- Scanning through a camera. The simulator's camera is the host webcam, so the QR path has unit tests
  only until hardware.
- The settings screen switches the PIN threshold on and off but does not edit amounts.

## Phase 2A: the fedimint signer boundary

### Delivered

| Path | What it is |
|---|---|
| `patches/fedimint/` | Three patches to fedimint v0.13.0-beta.1: the client signs through an external signer; mint v2 notes whose keys are held elsewhere, plus notes issued to and claimed back from external keys; LNv2 refund keys held elsewhere. Design and limitations: [`FEDIMINT_PATCHES.md`](FEDIMINT_PATCHES.md) |
| `scripts/fedimint-checkout.sh` | Clones fedimint at the tag into `third_party/fedimint` and applies the series; `--export` writes changes back |
| `wallet/` (`fps-wallet`) | The phone side. `DeviceKeys` pairs a federation with Prime and then serves as the client's note keys, refund keys and transaction signer. `device` reaches Prime over any transport, the simulator file bridge included. `send` stages, releases and reclaims ecash through Prime's outgoing branch |
| `crates/` | Prime now also authorises the LNv2 gateway for each payment it signs (`SignedTransaction::gateway_auth`) |

How a watch-only wallet works:

1. **Pairing.** The phone builds the federation's descriptor from its client config. Prime approves
   the federation, then the export of its view material.
2. **Receiving.** The phone derives every nonce and blinding key from the view material, so incoming
   ecash, Lightning and deposits land in Prime's tree without Prime.
3. **Spending.** The patched client builds the transaction, and `DeviceKeys` turns it into a sign
   request. The intent comes from the operation: the invoice of a Lightning payment, a withdrawal, a
   prepared send, a reclaim, or a refund. Prime analyses the bytes, the person approves, and the
   client checks the signatures before submitting.
4. **Ecash.** Prime stages notes on its outgoing branch, then releases them as a token, on its
   screen or to the phone. An unredeemed token comes back with a reclaim.

### Findings

- **LNv2 needs the refund key twice.** Beyond refunds, the gateway pays only with the refund key's
  signature over the invoice hash. Prime now returns it with the funding signatures, computed from
  the invoice it checked.
- **Receives must stay off the device.** fedimint's mint module consolidates and rebalances notes in
  any transaction it balances, receives included. A watch-only wallet doing that mixes a token's keys
  with Prime's, which Prime rightly refuses. The patch has such a wallet touch its notes only when a
  transaction needs funding.
- **The signer needs the operation before signing.** fedimint wrote the operation meta after
  building the transaction; the patch computes it first, so the invoice reaches Prime.
- **A refund cannot fail just because Prime is away.** A state machine claiming a refund now waits
  and retries instead of panicking.
- **fedimint's test fixtures need fedimint's lockfile.** A fresh dependency resolution picked an
  `electrum-client` that fedimint's LDK fork does not build with, so `wallet/Cargo.lock` starts from
  fedimint's.

### Still open

- fedimint-sdk with UniFFI bindings, devimint with a real gateway, and the simulator over the bridge
  (Phase 2B).
- On-chain withdrawal from the watch-only wallet. Prime signs withdrawals (Phases 1A and 1B); the
  phone side is tested once devimint runs (Phase 2B).
- Re-authorising a gateway after an app restart interrupts a payment.
- Database migrations for the changed encodings, before anything goes upstream (Phase 6).
