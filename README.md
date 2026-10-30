# prime-fedimint-signer

A Fedimint hardware signer for Foundation's **Passport Prime**. A phone app is the networked
Fedimint wallet; Passport Prime holds the spend keys, decodes every spend from raw bytes, shows it
on its trusted display, and signs only after a tap. Receiving never needs the device.

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
| 2B | fedimint-sdk bindings, devimint and the simulator, end to end | Next |
| 3 | Phone app and QR/UR on hardware | Planned |
| 4 | Bluetooth through a KeyOS QuantumLink patch | Planned |
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
| `prime-app/` | The KeyOS app, built with the Foundation SDK |
| `wallet/` | `fps-wallet`: the phone side, a watch-only fedimint client with Prime as its signer |
| `patches/fedimint/` | The patch series that gives fedimint's client a signer boundary ([`docs/FEDIMINT_PATCHES.md`](docs/FEDIMINT_PATCHES.md)) |
| `devtools/` | Host-only: golden vectors and cross-checks against the real fedimint crates |
| `spikes/xous-crypto/` | Phase 0 spike: the signer's crypto and encoding dependencies, built for the device |
| `tools/sim-e2e/` | The end-to-end driver for the app in the simulator |
| `scripts/` | `sdk-shell.sh` runs a command in the Foundation SDK shell; `device-build.sh` builds crates and `app-device-build.sh` the app for Passport Prime; `sim-e2e.sh` runs the simulator check |

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

Device builds need Nix and the Foundation SDK (1.1.0, installed under `~/.foundation/sdk`). They
use the SDK's pinned nightly and the `armv7a-unknown-xous-elf` target:

```sh
scripts/device-build.sh -p fps-signer -p fps-signer-core -p fps-protocol -p fps-policy --release
```

### The phone side

The watch-only client builds against fedimint with the patch series applied. The first build takes
a while, because fedimint's test fixtures run a whole federation in the test process:

```sh
scripts/fedimint-checkout.sh
cargo test --manifest-path wallet/Cargo.toml
```

### The KeyOS app

Run the app in the KeyOS simulator, then build it for the device without signing:

```sh
scripts/sdk-shell.sh bash -c 'cd prime-app && foundation sim'
scripts/app-device-build.sh
```

In the simulator the app also serves a file bridge, `~/.fps-signer-sim`, which takes request
envelopes in `inbox/` and answers in `outbox/` (see [`docs/PROTOCOL.md`](docs/PROTOCOL.md#transports)).
`foundation build` produces the signed bundle once a publisher identity exists.

The end-to-end check plays the golden scenario through the simulator: the bridge, the screen, the
system PIN, and a restart. It needs no other simulator running:

```sh
scripts/sim-e2e.sh
```

## Target platform

- Passport Prime, KeyOS 1.4.x, Foundation SDK 1.1.0 (KeyOS API 1.4.0).
- Federations on fedimint 0.13 with the default module shape: mint v2, wallet v2 and LNv2.
