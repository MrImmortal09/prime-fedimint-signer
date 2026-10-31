# prime-fedimint-signer

A Fedimint hardware signer for Foundation's **Passport Prime**. A phone app is the networked
Fedimint wallet; Passport Prime holds the spend keys, decodes every spend from raw bytes, shows it
on its trusted display, and signs only after a deliberate approval slide. Receiving never needs the device.

The design, the decisions behind it and the execution plan are in [`docs/plan/`](docs/plan/00_INDEX.md).
Progress by phase is in [`docs/PHASES.md`](docs/PHASES.md).

## Status

Work proceeds in phases that follow the plan's stages. Each phase lands as implementation commits
followed by a separate test commit.

| Phase | What it delivers | State |
|---|---|---|
| 0 | Repository, plan, device toolchain spike, CI | Done |
| 1A | Wire protocol, signer core, policy, signer service, golden vectors | Done |
| 1B | KeyOS app on the simulator with the file bridge | Done |
| 2A | fedimint signer boundary and the watch-only client, on an in-process federation | Done |
| 2B | The watch-only wallet against the KeyOS app in the simulator, on-chain withdrawal | Done |
| 3 | Phone app with fedimint-sdk bindings, QR/UR on hardware | In progress |
| 4 | Bluetooth: a network stand-in for QuantumLink in the simulator, the KeyOS patch once there is a device | Stand-in done |
| 5 | NFC tap-to-approve (optional) | Planned |
| 6 | Hardening and upstreaming | Planned |

## Layout

| Path | Contents |
|---|---|
| `docs/plan/` | The planning documents, including the Passport Prime / KeyOS reference |
| `docs/PHASES.md` | What each phase delivered, measured results, and what is still open |
| `docs/KEY_SCHEME.md`, `docs/PROTOCOL.md`, `docs/SECURITY.md` | The key scheme, the wire protocol, and the security model |
| `crates/protocol/` | `fps-protocol`: CBOR messages between phone and device |
| `crates/signer-core/` | `fps-signer-core`: fedimint decoding, key scheme, analysis, signing, ecash release |
| `crates/policy/` | `fps-policy`: spending limits |
| `crates/signer/` | `fps-signer`: the service the KeyOS app embeds |
| `crates/transport/` | `fps-transport`: animated QR framing and the simulator file bridge, both ends |
| `crates/simdrive/` | `fps-simdrive`: the simulator's screen for end-to-end tests, and a hand that approves reviews on it |
| `prime-app/` | The KeyOS app, built with the Foundation SDK |
| `wallet/` | `fps-wallet`: the phone side, a watch-only fedimint client with Prime as its signer |
| `wallet/fps-mobile/` | `fps-mobile`: the phone wallet's core, exported with UniFFI; the generated Kotlin bindings are in `bindings/kotlin/` |
| `ios/` | Prime Wallet, the iOS app: SwiftUI on the core's Swift bindings, reaching Prime over animated QR codes |
| `cashu/` | The phone's Cashu wallet, `fps-cashu`, in its own cargo workspace, and `fps-core`, the iOS library with both wallets |
| `patches/fedimint/` | The patch series that gives fedimint's client a signer boundary ([`docs/FEDIMINT_PATCHES.md`](docs/FEDIMINT_PATCHES.md)) |
| `devtools/` | Host-only: golden vectors and cross-checks against the real fedimint crates |
| `spikes/xous-crypto/` | Phase 0 spike: the signer's crypto and encoding dependencies, built for the device |
| `tools/sim-e2e/` | The end-to-end driver for the app in the simulator |
| `scripts/` | `sdk-shell.sh` runs a command in the Foundation SDK shell; `device-build.sh` builds crates and `app-device-build.sh` the app for Passport Prime; `sim-e2e.sh` and `sim-wallet-e2e.sh` run the simulator checks; `fedimint-checkout.sh` prepares the patched fedimint; `ios-core.sh` and `android-core.sh` build the phone wallet's core, and `ios-core-cashu.sh` the iOS core with Cashu; `cashu-subgraph-check.sh` compares fps-mobile's dependencies in `wallet/` and `cashu/` |

## Building and testing

Host checks need only stable Rust:

```sh
cargo test --workspace
cargo run --release -p fps-spike-xous-crypto --bin spike-bench
```

The cross-checks against fedimint 0.13.0-beta.1 live in their own workspace, because the fedimint
crates only build on the host (they need a C and C++ toolchain, CMake and libclang):

```sh
cargo test --release --manifest-path devtools/Cargo.toml
cargo run --release --manifest-path devtools/Cargo.toml --bin golden   # regenerate vectors
```

`nix develop` opens a shell with Rust and the C and C++ toolchain, CMake, libclang and protoc that
these cross-checks and the phone side need (`flake.nix`). It is not the Foundation SDK shell below.

Device builds need Nix and the Foundation SDK (1.1.0, installed under `~/.foundation/sdk`). They
use the SDK's pinned nightly and the `armv7a-unknown-xous-elf` target:

```sh
scripts/device-build.sh -p fps-signer -p fps-signer-core -p fps-protocol -p fps-policy --release
```

### The phone side

The watch-only client builds against fedimint with the patch series applied. The tests run a whole
federation in the test process, with fedimint's fake chain. Run them from `wallet/`, whose cargo
config gives fedimint's fixtures their test timings; the first build takes a while:

```sh
scripts/fedimint-checkout.sh
cd wallet && cargo test
```

### The iOS app

Prime Wallet needs Xcode and an Apple development team (the project's `DEVELOPMENT_TEAM`). Build
the Rust core into `ios/FpsMobile.xcframework`, then open the project, or build and install on a
connected iPhone from the command line:

```sh
scripts/ios-core.sh
open ios/PrimeWallet.xcodeproj
```

```sh
cd ios
xcodebuild -project PrimeWallet.xcodeproj -scheme PrimeWallet -destination 'id=<device UDID>' \
  -allowProvisioningUpdates -derivedDataPath build build
xcrun devicectl device install app --device <device UDID> build/Build/Products/Debug-iphoneos/PrimeWallet.app
```

### Cashu

`cashu/` holds the phone's Cashu wallet, `fps-cashu`, in a cargo workspace of its own beside
`wallet/` ([`docs/plan/12_CASHU.md`](docs/plan/12_CASHU.md)). Its tests do not build fedimint, but
cargo loads fedimint's path dependencies there, so run `scripts/fedimint-checkout.sh` first. The
live test and the example use the public test mint https://testnut.cashu.space; the example
reaches the Prime app in the KeyOS simulator, where someone approves each review:

```sh
cd cashu
cargo test -p fps-cashu
cargo test -p fps-cashu --test live -- --ignored
cargo run -p fps-cashu --example cashu_cli -- 127.0.0.1
```

`scripts/ios-core-cashu.sh` builds the iOS core with Cashu: fps-mobile and fps-cashu in one library,
for iPhones and the iOS Simulator, into the same `ios/FpsMobile.xcframework`, plus the Swift
bindings `ios/PrimeWallet/Generated/fps_cashu.swift`. The app built with it has a Cashu button; built
with a core from `scripts/ios-core.sh`, it is Fedimint only.

### The KeyOS app

Run the app in the KeyOS simulator, then build it for the device without signing:

```sh
scripts/run-simulator.sh
scripts/app-device-build.sh
```

In the simulator the app also serves a file bridge, `~/.fps-signer-sim`, which takes request
envelopes in `inbox/` and answers in `outbox/`, and listens on TCP port 7865 for the phone app on the
same network, a stand-in for QuantumLink while there is no device (both in
[`docs/PROTOCOL.md`](docs/PROTOCOL.md#transports)). On the phone, Settings chooses between QR codes and
the simulator over Wi-Fi.
`foundation build` produces the signed bundle once a publisher identity exists.

The end-to-end check plays the golden scenario through the simulator: the bridge, the screen, the
system PIN, and a restart. The wallet check puts the app behind the watch-only wallet instead: pairing,
Lightning payments and a refund, a withdrawal, a declined spend, and ecash sends, with every review
decided by tapping the simulator screen, once over the file bridge and once over the network link.
Each needs no other simulator running, and runs from a foreground shell:

```sh
scripts/sim-e2e.sh
scripts/sim-wallet-e2e.sh
```

## Target platform

- Passport Prime, KeyOS 1.4.x, Foundation SDK 1.1.0 (KeyOS API 1.4.0).
- Federations on fedimint 0.13 with the default module shape: mint v2, wallet v2 and LNv2.
