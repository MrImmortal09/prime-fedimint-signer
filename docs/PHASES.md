# Phases

Each phase maps to a stage of [`plan/05_EXECUTION_PLAN.md`](plan/05_EXECUTION_PLAN.md). Stages 1
and 2 are each split in two because they are the largest.

| Phase | Plan stage | Scope | State |
|---|---|---|---|
| 0 | Stage 0 | Repository, plan, device toolchain spike, CI | Done |
| 1A | Stage 1 | Wire protocol, signer core, policy, signer service, golden vectors | Done |
| 1B | Stage 1 | KeyOS app on the simulator with the file bridge | Done |
| 2A | Stage 2 | fedimint signer boundary and the watch-only client, on an in-process federation | Done |
| 2B | Stage 2 | The watch-only wallet against the KeyOS app in the simulator, on-chain withdrawal | Done |
| 3 | Stage 3 | Phone app with fedimint-sdk bindings, QR/UR, hardware | In progress: the core and the iOS app are done, hardware is pending |
| 4 | Stage 4 | Bluetooth: a network stand-in for QuantumLink in the simulator now, the KeyOS patch once there is a device | Stand-in done |
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
- **Gates.** Slide to approve or tap Reject on the review screen. Spends above 100,000 sats also need the device PIN,
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

### How it is verified

`cargo test --manifest-path wallet/Cargo.toml` runs fedimint's test fixtures: a federation with the
mint v2, LNv2 and dummy modules in the test process, a gateway that checks payer authorisations as a
real one does, and Prime as the real `fps-signer` service behind the same link the simulator bridge
implements.

| Check | Result |
|---|---|
| Receiving | Funds and ecash from a stranger arrive with no request reaching Prime; the wallet refuses to hand its notes out itself |
| Lightning payment | Prime reviews the invoice and signs; the gateway accepts Prime's authorisation and pays |
| Cancelled payment | The refund goes through Prime and only fees are lost |
| Ecash send | Prime stages the notes, releases them without a second review, and a stranger redeems the token; a token shown only on Prime comes back with a reclaim |
| Declined spend | A rejected send or payment leaves the balance and Prime's ledger as they were |
| Recovery | A new phone with the same view material recovers the exact balance from the federation |
| Local keys | A client that holds its own keys works as before the patches |
| Gateway authorisation on the device | The message equals fedimint's own invoice hash (golden vector), the signature verifies against the contract's refund key, and only payments carry one |

CI gains a job that clones fedimint, applies the series and runs these tests.

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

- fedimint-sdk with UniFFI bindings (Phase 3) and devimint with a real gateway (Phase 6). The
  simulator over the bridge landed in Phase 2B.
- On-chain withdrawal from the watch-only wallet. Prime signs withdrawals (Phases 1A and 1B); the
  phone side is tested in Phase 2B, against fedimint's fake chain.
- Re-authorising a gateway after an app restart interrupts a payment.
- Database migrations for the changed encodings, before anything goes upstream (Phase 6).

## Phase 2B: Prime in the loop

The plan's Stage 2 ends with the simulated Prime signing every spend of a fedimint wallet. This phase
does that with the real KeyOS app: the watch-only wallet of Phase 2A reaches the app in the
simulator over the file bridge, and every review is decided by an approval slide or a Reject tap on the simulator's screen.

### Delivered

| Path | What it is |
|---|---|
| `wallet/fps-wallet/tests/simulator.rs` | The wallet against the app in the simulator: pairing, two Lightning payments and a refund, a withdrawal, a declined spend, and three ecash sends, one of them taken back |
| `crates/simdrive/` (`fps-simdrive`) | The simulator screen driver, moved out of `tools/sim-e2e`, plus `Hand`: a person at the simulator on a thread of its own, who decides each review as told, reads the QR code off each result screen and taps Done |
| `scripts/sim-wallet-e2e.sh` | Starts the app in the simulator with a known seed and a clean start, runs the wallet test, stops the simulator and restores the developer's seed. `scripts/simulator.sh` holds the start and stop code both simulator scripts share |
| `wallet/fps-wallet/tests/` | Wallet v2 joins the in-process federation, with a withdrawal test: a stranger pegs in, the wallet withdraws, Prime signs, and the address receives exactly the value |
| `wallet/.cargo/config.toml` | Gives fedimint's fixtures their test timings |

The plan also put fedimint-sdk bindings and devimint in Stage 2. The bindings move to Phase 3, where
the phone app needs them. devimint moves to Phase 6: the in-process federation already runs
fedimint's real consensus and modules, so only the gateway and the chain are stand-ins.

### How it is verified

`scripts/sim-wallet-e2e.sh` runs the wallet test against the app in the simulator. Every screen the
hand dealt with is checked, in order:

| Step | Screens on the simulator | Checked on the wallet side |
|---|---|---|
| Pairing | Add federation, Connect wallet app: approved | The view material matches; funding needs no review |
| Lightning payment | Pay Lightning invoice: approved | The gateway verifies the authorisation and pays |
| Cancelled payment | Pay Lightning invoice, Refund failed payment: approved | The refund comes back |
| Withdrawal | Withdraw on chain: approved | The address receives exactly the value |
| Declined spend | Prepare ecash to send: rejected | Declined; the balance is unchanged |
| Ecash to the phone | Prepare ecash to send: approved | Released without a second review; a stranger redeems the token |
| Ecash on Prime | Prepare ecash to send: approved; the token screen | The token read off the screen redeems for the stranger; the phone never sees it |
| Ecash taken back | Prepare ecash to send: approved; the token screen; Reclaim unredeemed ecash: approved | The wallet gets the notes back |

The run takes about half a minute once the app is built. In CI, `cd wallet && cargo test` adds the
withdrawal test and skips the simulator test, which needs the Foundation SDK.

### Findings

- **The app needed no change.** Transactions built by the patched fedimint client for a live
  federation pass the same analysis and reviews as the golden scenario of Phase 1B.
- **fedimint's fixtures expect test timings.** fedimint shortens its intervals when its tests run
  under nextest. Under plain `cargo test` the guardians polled the fake chain once a minute, and the
  withdrawal test took three minutes. With the switch on, it takes eight seconds, and the whole
  wallet suite about thirteen.
- **A screen's log line comes before the screen.** The hand waits for the button colours of the
  screen it expects, a copper approval slider above a full-width charcoal Reject button, or a full-width copper Done button, and for two identical
  screenshots, before it acts. After the approval swipe or button tap it waits for the screen to change, so it never acts on
  a stale screenshot.

### Still open

- fedimint-sdk with UniFFI bindings, for the phone app (Phase 3).
- devimint with a real gateway and bitcoind (Phase 6).
- The hand cannot type the PIN, so the wallet test stays under the PIN threshold. Phase 1B's scenario
  covers an approval with the PIN.
- Re-authorising a gateway after an app restart, and the database migrations (Phase 6).

## Phase 4: the network link, a stand-in for QuantumLink

The plan's Stage 4 puts the phone and Prime on Bluetooth through a KeyOS QuantumLink patch, which
needs a dev unit and custom firmware. There is no device yet, and the hosted simulator has no radio:
KeyOS's Bluetooth server does nothing in hosted mode. This phase builds the wireless path with a
stand-in link, so every flow runs the way it will over Bluetooth and only the bottom transport
differs: the simulator app listens on TCP, and the phone on the same network connects to it.

### Delivered

| Path | What it is |
|---|---|
| `crates/transport/src/net.rs` | Both ends of the link: length-framed envelopes over TCP, one exchange per connection. The app's `Listener` accepts on a thread and queues each request with its connection; the phone's end connects, sends and waits for the answer |
| `prime-app/src/netlink.rs` | The app's end in the simulator: listens on `$FPS_NET_ADDR` (default `0.0.0.0:7865`), feeds requests through the same handling as scanned ones, and answers on the request's connection. Device builds compile none of it, and the device build of `fps-transport` leaves the module out |
| `wallet/fps-wallet` | `NetworkLink`, the phone's end for the watch-only client |
| `wallet/fps-mobile` | `Wallet::set_simulator_address`: the app switches between its own link (QR codes) and the simulator over the network at any time, without reopening the wallet |
| `ios/` | Settings: a Connection choice, QR codes or the simulator over Wi-Fi, and the Mac's address |

How it works:

1. The phone connects and sends the request envelope. The app's listener reads it and queues it.
2. The app takes one request at a time, between reviews, exactly as it takes bridge requests. A
   review shows on the simulator screen; the connection stays open while the person decides.
3. The answer envelope goes back on the same connection, which then closes. A request that arrives
   while a review is on screen waits in order.

The link carries the same bytes as every other transport, and the app's channel handling for it is
the one Bluetooth will use: the answer returns on the link instead of being shown as a code.

### How it is verified

`scripts/sim-wallet-e2e.sh` runs the wallet scenario twice, each time on a fresh app: over the file
bridge, then over the network link. Both runs deal with the same 13 screens, from pairing through
Lightning, a withdrawal, a declined spend and ecash sends, with every review decided by tapping the
simulator screen.

| Run | Screens | Time once built |
|---|---|---|
| File bridge | 13 | 34 s |
| Network link | 13 | 27 s |

`fps-transport` gains seven tests for the link: framing, the message limit on both ends, an answer
that arrives after a wait, requests queued in order while one is on screen, the phone's timeout, a
closed port, and a bare host getting the default port.

### Findings

- **The simulator stops when `foundation sim` loses its stdin.** Started from a shell that closes
  stdin, the simulator asks for shutdown about four seconds in and every request times out. The
  simulator scripts must run from a foreground shell; starting it by hand needs stdin kept open,
  for example with a pipe from `sleep`.
- **Tests must not guess closed ports.** A transport test flipped one bit of its own port to find a
  closed one and hit another test's live listener, whose scenario then took the stray request. It
  connects to port 1 now, which is reserved.
- **No security on the stand-in.** There is no encryption or authentication, so it is for a network
  you control. QuantumLink provides both on hardware, where this link does not exist.

### Still open

- QuantumLink itself: the KeyOS patch, custom firmware and a dev unit, as the plan's Stage 4 says.
- While a request waits on the simulator, the phone's busy note names the operation, not that
  someone must approve on Prime.
- The Android app has the switch in its Kotlin bindings and no screen for it yet.

## Wallet UI refinement

- Shared design tokens for iOS and KeyOS, with native system typography and device light/dark themes.
- Prime reviews show device-decoded details and warnings above a deliberate slide-to-approve control; rejection remains a separate button. Canceling the system PIN dialog or an authorization failure re-arms the slider.
- Full-width result QR codes preserve scanner module size. Scan-next navigation, federation forgetting, QR density selection, and the animated iPhone scanner remain supported.
- The simulator hand dispatches on approval/rejection decisions and detects review/result screens using samples inside both ends of the bottom button. RGB-only accent checks reject an incorrect blue theme.
- iPhone screens use existing wallet data, identify unsupported features, and distinguish authorization, submitted payments, and redeemed ecash.

### Verification

- 144 host tests, including seven screen-detection regressions with standard-SDK captures.
- Before integrating the network transport additions, the golden simulator scenario passed 19 checks and four restart checks.
- `scripts/sim-wallet-e2e.sh`: passed over both the file bridge and network link, with 13 screens each, including approval swipes, rejection, withdrawal, and ecash result QR decoding.
- Simulator runs use the corrected RGBA capture tool documented in `patches/keyos/README.md`. The RGB-only detector rejects swapped-blue output from the unpatched tool.
- Swift syntax and Slint compilation/layout checks pass. These do not replace an iOS build or physical camera tests.

### Verification still needed

- An iOS build and device checks with Xcode, including accessibility, Dynamic Type, and landscape.
- The physical iPhone-to-Prime camera loop and hardware color parity.
