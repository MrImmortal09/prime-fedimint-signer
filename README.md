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
| 1A | Wire protocol, signer core, policy, signer service, golden vectors | Next |
| 1B | KeyOS app on the simulator with the file bridge | Planned |
| 2 | fedimint client signer boundary, end to end on a local federation | Planned |
| 3 | Phone app and QR/UR on hardware | Planned |
| 4 | Bluetooth through a KeyOS QuantumLink patch | Planned |
| 5 | NFC tap-to-approve (optional) | Planned |
| 6 | Hardening and upstreaming | Planned |

## Layout

| Path | Contents |
|---|---|
| `docs/plan/` | The planning documents, including the Passport Prime / KeyOS reference |
| `docs/PHASES.md` | What each phase delivered, measured results, and what is still open |
| `spikes/xous-crypto/` | Phase 0 spike: the signer's crypto and encoding dependencies, built for the device |
| `scripts/` | `sdk-shell.sh` runs a command in the Foundation SDK shell; `device-build.sh` builds crates for Passport Prime |

## Building and testing

Host checks need only stable Rust:

```sh
cargo test --workspace
cargo run --release -p fps-spike-xous-crypto --bin spike-bench
```

Device builds need Nix and the Foundation SDK (1.1.0, installed under `~/.foundation/sdk`). They
use the SDK's pinned nightly and the `armv7a-unknown-xous-elf` target:

```sh
scripts/device-build.sh -p fps-spike-xous-crypto
```

## Target platform

- Passport Prime, KeyOS 1.4.x, Foundation SDK 1.1.0 (KeyOS API 1.4.0).
- Federations on fedimint 0.13 with the default module shape: mint v2, wallet v2 and LNv2.
