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

### Cashu phases

Cashu is a second backend beside Fedimint, built as a stack of pull requests that leave the
Fedimint code as it is. Each maps to a row of [`plan/12_CASHU.md`](plan/12_CASHU.md).

| Phase | Branch | Scope | State |
|---|---|---|---|
| C0 | `cashu/0-plan` | The plan, decision D14, compatibility pins for the Fedimint wire format, state and vectors | Done |
| C1 | `cashu/1-core` | The Cashu wire types, `crates/cashu-core`, golden vectors and cross-checks from the `cashu` crate | Done |
| C2 | `cashu/2-prime` | Protocol variants, the Cashu service and `Device`, the Prime app opening it | Done |
| C3 | `cashu/3-wallet` | The phone's `cashu/` workspace and `fps-cashu` with every flow | Planned |
| C4 | `cashu/4-simulator` | The Cashu wallet against the Prime app in the simulator | Planned |
| C5 | `cashu/5-ios` | The `fps_cashu` bindings and the Cashu screen in Prime Wallet | Planned |

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

## Cashu 1: the Cashu signer core

The first Cashu pull request, `cashu/1-core`: everything Prime needs to check and sign a Cashu
spend, as a library and wire types that nothing calls yet. The protocol variants, the Cashu service
and the app follow in Cashu 2. No Fedimint file changes beyond two lines in
`crates/protocol/src/lib.rs`.

### Delivered

| Path | What it is |
|---|---|
| `crates/protocol/src/cashu.rs` | The Cashu wire types: `CashuRequest`, `CashuResponse` and their bodies, reached through `Request::Cashu` and `Response::Cashu` once Cashu 2 adds them. Spec: `PROTOCOL.md`. Their tests are in `crates/cashu-core/tests/protocol.rs`, so fps-protocol gains only the module |
| `crates/cashu-core` (`fps-cashu-core`) | The `fps-cashu-v1` key scheme, Prime's half and the phone's; canonical mint URLs; the secret template; NUT-00 `hash_to_curve` and `B_`; the NUT-11 `SIG_ALL` message and its one-reading check; NUT-12 DLEQ; NUT-02 keyset ids, mint validation and updates; swap and melt analysis with their reviews; signing. Spec: [`KEY_SCHEME_CASHU.md`](KEY_SCHEME_CASHU.md) |
| `devtools-cashu/` (`fps-devtools-cashu`) | Host-only workspace linking the real `cashu` 0.18.3 crate: the golden vector generator and randomised cross-checks |
| `.github/workflows/ci.yml` | Job `cashu-vectors`: regenerates the vectors, diffs them, runs the cross-checks |

The crate depends on fps-protocol, fps-signer-core (only its review type, BOLT11 parsing and error
type), bitcoin and zeroize, all already in the device build. Its dependency graph has no
`getrandom`, `uuid`, `url`, `serde_json` or `lightning`, it adds no feature to `bitcoin`, and the
root `Cargo.lock` gains only its own entry. It answers with existing rejection codes only.

### How it is verified

| Check | Result |
|---|---|
| Golden vectors from `cashu` 0.18.3 | 14 hash-to-curve and 8 blinding cases with the NUT-00 vectors, 7 keyset ids of both versions with NUT-02's own, 19 mint URLs, 13 DLEQ cases with NUT-12's, and 3 mints of `fps-cashu-v1` derivations with 48 outputs whose secrets `cashu` parses and writes back unchanged: all reproduced byte for byte |
| Six full spends: send, combine, move to a new key, send with change three epochs ahead, two melts | `cashu`'s `sig_all_msg_to_sign` equals Prime's message, `verify_spending_conditions` accepts Prime's signature, and the signature is reproduced byte for byte |
| Refused vectors | The two known collision transcripts. Seven spends: a collision ground from the wallet's own change, a tampered opening, a foreign lock key, change with a stranger's secret under the wallet's own `r`, a melt input without a DLEQ proof, change four epochs above its inputs, a melt with too few blanks. Three mints: a non-canonical URL, a wrong keyset id, and a keyset listing another's keys under other amounts. Each is refused with its code, by the rule its stored reason names |
| Randomised cross-checks against `cashu` | 500 hash-to-curve and blinding cases, 300 keyset ids, 40 mints `cashu` generated (pinned the same by an identical update, and a copy of one keyset's keys under rotated amounts refused, alone or as an update), 200 template secrets, 300 swap and melt messages, 100 DLEQ proofs, 2,000 URLs; 60 random swaps (send, combine, move) and 40 random melts signed by Prime and accepted by `verify_spending_conditions`, and refused once an amount or the quote changes |
| Behaviour tests | analysis 24, vectors 11, mint and URL 11, protocol 8, key scheme 8, message 6, and an ignored measurement of the refusal rates below. The message tests include the NUT-11 vectors and the one-reading check against plain recursion on 6,000 transcripts whose keys are points or only look like keys; the protocol tests add an unknown field to every map of every body; the key scheme tests check the frozen derivations against their formulas and show no Cashu key or tag meets an `fps-v1` one for the seeds of its frozen vectors |
| Fedimint untouched | `cargo test --workspace` passes 225 tests on main, the 157 from before among them, and the pins of Cashu 0 hold |
| Device build | `scripts/device-build.sh -p fps-cashu-core --release` builds for `armv7a-unknown-xous-elf` with the SDK's nightly, without warnings |

Host timings, Apple M4, release build at opt-level `s`:

| Operation | Host |
|---|---|
| Analyse and sign a swap, 1 input and 2 outputs | 0.28 ms |
| … 4 inputs and 16 outputs | 0.66 ms |
| … 64 inputs and 64 outputs | 2.0 ms, or 7.2 ms with a DLEQ proof on every input |
| Analyse and sign a melt, 3 inputs with DLEQ proofs and 2 blanks | 0.49 ms |
| Verify one DLEQ proof | 0.08 ms |
| One-reading check of 64 outputs | 0.03 ms, or 0.21 ms when their hex alone reads two ways and the keys are parsed |
| Validate a mint of 3 keysets, 128 keys | 0.32 ms |

With the 50 to 150 times factor from Phase 0, a typical request takes under 0.1 s on the device
and the largest, 64 inputs with DLEQ proofs and 64 outputs, about 0.35 to 1.1 s.

`cargo test --workspace` runs the core's tests. `cargo run --release --manifest-path
devtools-cashu/Cargo.toml --bin golden` regenerates the vectors, which CI checks are unchanged, and
`cargo test --release --manifest-path devtools-cashu/Cargo.toml` runs the cross-checks. `cargo test
--release -p fps-cashu-core --test message -- --ignored --nocapture` measures the refusal rates.

### Findings

- **A key update could relabel the mint's own keys.** Neither keyset id version ties a key to its
  amount: a version 01 id of the mint's real keys listed under other amounts is valid. A melt's
  message does not commit to input amounts, and Prime counts each input at the amount its DLEQ
  proof's key is pinned for, so a compromised phone could pin such a keyset with a routine "update
  mint keys" and then have a real proof of 2^20 sats counted as 2^11, under the PIN threshold, which
  the `cashu` crate's own checks accept. Prime now refuses a key under two amounts anywhere in what
  it pins for a mint; an update cannot change a pinned keyset's keys or fee; and a keyset the mint
  stops listing stays pinned as inactive, so no key's amount is ever forgotten and listed anew. A
  keyset first pinned while the phone is compromised is still trusted on first use. One cost: a
  mint that changes the fee of a version 00 keyset under the same id can no longer be updated, as
  NUT-02 asks wallets to refuse a keyset whose id collides with one they hold.
- **Two readings by form are common, by points less so.** Measured with the core's own check on
  100,000 honest transcripts each, the outputs splitting a random total into powers of two in
  increasing order: 6.1% of totals below 2^10 sats, 26% below 2^20 and 33% below 2^30 read two
  ways by the form of their hex alone. Counting only readings whose keys are points, the only ones
  a mint signs, 0.6%, 9.5% and 13.4% do (0.2%, 8.6% and 12.4% in decreasing order), and those are
  refused. Above 2^10, most come from a 1,024-sat output that is not last (four in five of the
  refusals below 2^20): `1024` also reads as `1` and a key beginning `024`, which is a point half
  the time, whenever the key's last three hex characters are digits. A melt's blanks never collide.
  The phone must run the same check before it asks Prime, and move to the next counters or
  re-randomise a leaving output when it fails; Cashu 3 does.
- **Nutshell reads more forms of `B_`.** CDK re-encodes every `B_` compressed before it builds the
  message, but Nutshell 0.21.0 hashes the string the wallet posted and accepts any key
  libsecp256k1 parses, uncompressed (`04`) and hybrid (`06`, `07`) included. So the check counts
  130-character readings too, where the plan first counted only `02` and `03` keys. Such a window
  is a point only if its last 64 digits are the y coordinate of its first 64, which no phone can
  arrange by grinding blinding factors, so these readings refuse nothing honest in practice.
- **Restores must look four epochs ahead.** Prime signs change up to three epochs above its inputs,
  so a restore that stopped at the first empty epoch could miss change at epoch e + 3 when e + 1 and
  e + 2 were never used. A restore now stops after four empty epochs in a row.
- **A secret used twice burns a proof.** The mint tracks spent secrets, so an output whose secret is
  an input's, or another output's, can never be spent. A compromised phone could have Prime approve
  a combine that destroys the wallet's proofs; the analysis refuses repeated secrets.
- **A quote id could hide a blank.** The phone supplies the quote id, and one that begins with an
  amount and a key puts an extra blank in a melt's message. Quote ids must be printable ASCII of at
  most 128 characters that do not begin like an output. A melt already counts every input as
  leaving, so this closes a mislabelling rather than a loss.
- **Prime cannot see the phone's counters.** A compromised phone can put change at a counter a
  restore never reaches, or at the position of a proof the wallet already holds. That strands
  funds rather than moving them; the Cashu service can keep the highest counter it has signed per
  keyset and epoch.

### Still open

- Cashu 2: the protocol variants, the Cashu service with `fps-cashu.cbor`, `Device`, and the Prime
  app opening it, with the device and simulator checks. The service passes the mint it has pinned
  to `validate_mint` and `review_add_mint`, and keeps what `validate_mint` returns.
- The phone side of the one-reading check, in Cashu 3.
- Device timings need hardware.

## Cashu 2: Cashu on Prime

The second Cashu pull request, `cashu/2-prime`: the Prime app serves Cashu requests beside the
Fedimint ones. Fedimint changes by two appended protocol variants and two additive edits in
`crates/signer`; with Cashu idle, the app answers and stores exactly what it did before.

### Delivered

| Path | What it is |
|---|---|
| `crates/protocol` | `Request::Cashu(CashuRequest)` and `Response::Cashu(CashuResponse)`, appended as the last variants. `PROTOCOL_VERSION` stays 1. Spec: `PROTOCOL.md` |
| `crates/signer` | `ApprovalKind::Cashu`, and the `dispatch` arm that answers `Request::Cashu` with `Rejected(Unsupported)`: a signer on its own has no Cashu |
| `crates/cashu-signer` (`fps-cashu-signer`) | The Cashu service, with its own state in `fps-cashu.cbor`, and `Device`, which the app opens in place of `Signer` |
| `prime-app` | Opens `Device` (the import, the field type and the open call in `main.rs`); `CashuStorage`, a durable `AppData` file `fps-cashu.cbor`; the simulator reset removes it, and its `.tmp` and `.new`, in the same block as the signer state |
| `.github/workflows/ci.yml` | Job `curve-features`: fails if `fps-cashu-signer` switches on a feature of `bitcoin`, `secp256k1` or their sub-crates that `fps-signer` alone does not, or brings in `getrandom` |

How it works:

- **`Device` derefs to `Signer`.** Only `handle`, `pending`, `approve` and `reject` are its own, so
  every other call the app makes (`federations`, `forget_federation`, `policy`, `set_policy`,
  `state`) reaches the signer unchanged, and `view.rs` and the Slint pages are untouched.
- **Routing.** `Request::Cashu` goes to the Cashu service, everything else to `Signer::handle`,
  except that while a Cashu review waits the four requests that need approval get the signer's
  own answer for a waiting review, byte for byte: their stored answer, or `Busy`. `GetResult` and
  `Cancel` go to the signer first and to the Cashu service only when the signer has nothing.
- **The Cashu service.** `Info` is always answered. `AddMint` validates the descriptor against
  what is pinned for that mint, shows "Cashu: add mint" or "Cashu: update mint keys", and on
  approval pins what `validate_mint` returned and answers the lock material, never stored. `Sign`
  analyses the spend against the pinned mint, applies the policy, shows the spend's review, and on
  approval signs with fresh randomness and stores the answer and the outflow before it leaves.
- **State.** `fps-cashu.cbor` holds a version of its own, every pinned mint as one descriptor of
  everything pinned for it, a ring of the last 32 final answers, and Cashu's own count of the day's
  outflow. `Device::open` opens the signer only: the file is read on the first request that needs
  it (a Cashu request, or a `GetResult` the signer has nothing for), so opening the app costs
  Fedimint nothing whatever Cashu holds. Decoding is strict, and every stored mint is validated
  again when the file is read and must come out as stored. A file that cannot be read, decoded or
  validated, or has another version, turns Cashu off until the app opens again and is never
  rewritten; `Info` then lists no mints, `AddMint` and `Sign` get `Rejected(Internal)` saying why,
  and Fedimint works as before.
- **Limits.** At most 16 mints, and 2,048 keys over all of them (`MAX_PINNED_KEYS`, sixteen mints
  of 128 keys); an `AddMint` past either, an update included, gets `TooLarge`, and a file past
  either does not load. The file is then at most 137,229 bytes, where `fps-cashu-core`'s per-mint
  limits (16 keysets of 64 keys) alone allowed about 950 KB, rewritten whole at every decision.
- **Policy.** Read through `Signer::policy()` at each decision and evaluated by fps-policy's own
  code: the same PIN threshold, per-request limit and daily limit. The daily count is Cashu's own
  plus Fedimint's for the same day, read from the signer's state, so a Cashu spend cannot take the
  two together over the limit; Cashu counts a spend only once it is approved. A melt passes its
  invoice amount and fee reserve as the policy's routing check, so the routing fee limit bounds a
  mint's fee reserve as it bounds a gateway's fee.
- **Randomness.** The app passes a second `DevicePlatform` to `Device::open`, for the Cashu
  service, so `device.rs` needs no change to the platform and each service keeps its own.

This closes the note under Cashu 1: the service passes the mint it has pinned to `validate_mint`
and `review_add_mint`, and keeps what `validate_mint` returns.

### How it is verified

| Check | Result |
|---|---|
| Service, through `Device` | 22 tests: info; adding a mint, asked every time; an update that only adds and keeps an unlisted keyset pinned as inactive, from which a proof is still spent; four refused updates (a changed fee under a version 00 id, a key relabelled to another amount, unit `usd`, a URL not in canonical form); a send and a melt signed after approval, checked with BIP-340 against `P_e` over the message Prime built; fresh randomness for each signature; rejection, replay and `GetResult`; the ring of 32 answers, which drops the oldest and reads back full; one Cashu review at a time, while `Info` and stored answers still come back; an unknown mint; the PIN threshold; per-request and daily limits, counted on approval only and reset the next day; the daily limit over Fedimint's frozen count of 552,067,136 msat plus Cashu's; a fee reserve over the routing fee limit; cancel; restart; failed writes keeping an approval or a cancel waiting and ending a rejection; the limits of 16 mints and 2,048 keys, an update counted once; the file read on first use, never when the device opens; ten kinds of unreadable Cashu file turning Cashu off without being rewritten while a federation is imported; `Device::open` failing exactly as `Signer::open` |
| Differential | The Fedimint golden scenario of `compat_state.rs`, then status, every stored answer, replays, nonces, malformed, foreign-version and oversized messages, `Busy` for each of the four requests and nonces, results, cancels and malformed bytes answered while a review waits, a PIN, a release without review, a failed write, and forgetting and importing again: 96 events (answers, approvals asked for, results of approving, rejecting and changing the policy) and 19 writes of `fps-state.cbor`, identical through a bare `Signer`, a `Device` without a Cashu file and a `Device` with a populated one, whose file is never written. The populated file's answers sit under ids the Fedimint traffic uses too, so a Fedimint answer taken from Cashu's ring, or a `GetResult` that looked in Cashu first, would show. The scenario part still writes the frozen `fps-state-v1.cbor` |
| Cross-backend | 5 tests: a waiting Cashu review gets `ImportFederation`, `ExportViewMaterial`, `SignTransaction` and `ReleaseNotes` (prepared notes included) the bytes a bare signer gives with its own review waiting, and stored answers, `GetInfo`, `GetStatus`, nonces and `GetResult` the bytes an idle one gives; a waiting Fedimint review makes `AddMint` and `Sign` `Busy` with the signer's own bytes while `Info` and stored Cashu answers still come back; `GetResult` and `Cancel` find either backend's requests, and give the signer's bytes when neither has one; an id both backends hold replays each backend's own answer, and `GetResult` and `Cancel` find Fedimint's first; approving and rejecting reach the service whose review waits |
| Compatibility | 3 tests: all 53 v1 messages of `compat/v1-messages.json` answered identically by a `Device` and a bare `Signer`, fresh and populated; an old Prime's decoder, a copy of the v1 `Request` enum, refuses every Cashu request with `Malformed` under its id, an answer the phone reads; a bare `Signer` answers `Unsupported` and stores nothing |
| Isolation | 3 tests: Cashu traffic over the frozen `fps-state.cbor` never writes it; Fedimint traffic over a populated `fps-cashu.cbor` never writes it; a device over two empty files knows neither backend. The simulator reset that empties them is the app's, checked on the simulator only (below) |
| Protocol | 5 tests: every Cashu body round trips in an envelope, the variants are tagged by name, two envelopes are pinned byte for byte, decoding stays strict, and openings and the view key stay out of Debug output |
| Mutations | 14 deliberate faults in `Device` and the service, each caught by at least one test and then reverted: Fedimint answered from Cashu's ring, `GetResult` or `Cancel` looking in Cashu first, nonces `Busy` while a Fedimint review waits, the ring never dropping or dropping the newest, the answers limit skipped on load, stored answers withheld while a Cashu review waits, constant signing randomness, a failed reject keeping its review, the file read at open, either key limit dropped, and an update counted twice |
| Fedimint untouched | `cargo test --workspace` passes 266 tests on main, the 225 from before among them; the pins of Cashu 0 (`compat_v1.rs`, `compat_state.rs`, `pins.rs`) hold; the root and app `Cargo.lock` only gain entries; `wallet/` resolves `--locked` with its lockfile unchanged |
| `curve-features` | Passes; adding bitcoin's `secp-lowmemory` to `fps-cashu-signer` makes it fail, naming the three features it adds, and so does a `cargo tree` that fails: the step sets `pipefail` and checks both listings are real |
| Device build | `scripts/device-build.sh -p fps-cashu-signer --release` builds without warnings, and `scripts/app-device-build.sh` links the app: stripped, 5,212,936 bytes, 302,880 more than before (4,910,056), with the text segment 300,020 bytes larger |
| Simulator | `scripts/sim-e2e.sh`: 19 checks and 4 after a restart. `scripts/sim-wallet-e2e.sh`: 13 screens over the bridge (40.1 s) and 13 over the network link (27.6 s). Both resets log that the Cashu state was removed too |
| Cashu in the simulator | A scratch driver, not committed (Cashu 4 adds the script): `Info`, then "Cashu: add mint" and "Cashu: send ecash" approved on the simulator's screen, the lock material and the signature checked; after a restart the mint and the stored answer are still there; after a reset both are gone. Rerun with the lazy read: the first two parts pass again; the reset part was cut short by another simulator started beside it, and passed in the earlier run |

Host timings, Apple M4, release build:

| Operation | Host |
|---|---|
| Open `Device` over the frozen `fps-state.cbor`, with any Cashu file | 6.2 ms, as the bare `Signer` (6.1 ms): the Cashu file is not read |
| The first Cashu request after that, which reads and checks a file of 1 mint (3 keysets, 128 keys) | 0.36 ms more |
| … of 16 such mints, 2,048 keys | 5.6 ms more |
| … of the largest file the limits allow, 16 mints of 16 keysets of 8 keys | 6.0 ms more |
| A send, 3 inputs and 2 outputs: handle, approve, sign and store | 0.41 ms |
| A melt, 3 inputs and 2 blanks, the invoice parsed twice (analysis, then the routing check) | 0.63 ms |
| The extra decode `Device` makes before `Signer::handle`, for the largest golden request (1,522 bytes) | 1.6 µs |

`fps-cashu.cbor` is 13,953 bytes with one such mint and 32 stored answers, 124,045 bytes with 16
mints, and at most 137,229 bytes. With the 50 to 150 times factor from Phase 0, opening the app
costs no more than before whatever Cashu holds, and the first Cashu request after it takes 0.3 to
0.9 s longer with the most keys, and well under 0.1 s with one mint.

`cargo test --release -p fps-cashu-signer --test service -- --ignored --nocapture` measures them.

### Findings

- **The routing fee limit refuses large melts at a CDK mint with default settings.** CDK's mintd
  reserves 2% of the invoice for fees, at least 2 sats; the default limit is 100 sats plus 1.5%,
  so with both at their defaults a melt above 20,000 sats is refused with `PolicyLimit`. Nutshell
  reserves 1%, which fits. The limit is the user's routing fee setting, which the settings screen
  cannot edit yet.
- **`GetResult` answers with the stored envelope, addressed to the request it is about.** The
  simulator bridge files an answer under the id inside it, so a phone that fetches a result over
  the bridge must look for the original request's id, for Cashu as for Fedimint. Found by the
  scratch simulator driver, which first waited under the `GetResult`'s own id.
- **Replays are per backend.** A Cashu request is replayed only from Cashu's answers and a
  Fedimint request only from Fedimint's, so a populated Cashu file cannot change a Fedimint answer.
  Only `GetResult` and `Cancel` look in both, Fedimint first. A phone reusing an id across backends
  gets each backend's own answer. The first tests kept the two sets of ids apart and could not see
  the lookup order; the differential and a cross-backend test now share ids.
- **A failed write ends a Cashu rejection, as it ends a Fedimint one.** The app stops waiting for a
  review once the user rejects it, whatever the result, so a Cashu review kept after a failed
  reject (as first written) could still be approved, and its answer would have gone to a QR screen
  rather than the link the phone waits on. A failed approval keeps the review in both services. A
  failed cancel keeps a Cashu review waiting and answers `Internal`, where the signer drops its
  own; the app keeps waiting after a cancel, so the user can still decide.
- **Reading the Cashu file at open delayed Fedimint, and the file had no real bound.** The
  per-mint limits allowed 16 mints of 16 keysets of 64 keys: a file of about 950 KB, rewritten at
  every Cashu decision, and 45 to 53 ms on the host to check, so 2 to 8 s before "signer open" on
  every start. The file is now read on first use and holds at most 2,048 keys.
- **The app grows by 296 KiB,** about three times the 95 to 120 KiB the research estimated for a
  bare verifier: the service, the analysis messages and reviews, and the CBOR code for the Cashu
  types.
- **`crates/signer` logs nothing, and neither does the Cashu service.** The app's existing
  `review on screen: <title>` line shows the Cashu titles; the only new log line is the reset's.

### Still open

- Fedimint's own checks do not see Cashu's outflow: Cashu holds the combined day under the daily
  limit, but a Fedimint spend after Cashu spends can take the combined day over it. Today the
  daily limit cannot be set from the settings screen, and without a clock the device's count never
  resets.
- No way to forget a mint, on screen or by request, so the limits of 16 mints and 2,048 keys are
  for good. The home screen shows no mints, and a Cashu file that cannot be read shows only in
  Cashu answers (`Device::cashu_off`, which reads the file, is there for the app to show it).
- The routing fee limit and CDK's default fee reserve (above).
- The service could keep the highest counter it signed per keyset and epoch (Cashu 1).
- No committed check runs the simulator reset that removes `fps-cashu.cbor`: no workspace test
  reaches `CashuStorage::wipe`, and the Fedimint simulator scripts send no Cashu. Cashu 4's
  `sim-cashu-e2e.sh` should commit the scratch driver's check: add a mint, reset, and `Info` lists
  none.
- The phone (Cashu 3), the Cashu simulator script (Cashu 4), and device timings on hardware.
