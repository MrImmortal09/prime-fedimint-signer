# 07. Upstream changes by repository

File paths are from the commits inspected on 2026-10-30. Each section lists what to change, why, and
how the change stays upstreamable.

## fedimint (fork from tag `v0.13.0-beta.1`)

**Why this tag:** fedimint-sdk requires every fedimint crate at `=0.13.0-beta.1`, and fedimint crates
require each other with `=`. A fork that keeps the version string can be substituted with
`[patch.crates-io]` in fedimint-sdk's `Cargo.toml` for every crate we touch.

| Crate and file | Change |
|---|---|
| `fedimint-client-module/src/transaction/builder.rs` | `ClientInput.keys: Vec<Keypair>` becomes `Vec<InputKey>` with `InputKey::Local(Keypair)` and `InputKey::Remote { key_id }`. Add `trait TransactionSigner { async fn sign(&self, txid, keys: &[RemoteKeyRef]) -> Result<Vec<Signature>> }`. `TransactionBuilder::build` becomes async, signs local keys itself and asks the signer for remote ones, keeps input order |
| `fedimint-client/src/client.rs` | `finalize_transaction` and `finalize_and_submit_transaction*` pass the client's signer into `build`; `ClientBuilder` gains `with_signer(Arc<dyn TransactionSigner>)` defaulting to a local signer, so existing users see no change |
| `modules/fedimint-mint-client/src/output.rs` | `NoteIssuanceRequest::new` gets a derivation strategy: upstream HKDF, or `Xpub { xpub, view_secret, tier, index }` where the nonce is the xpub child and the blinding key is HKDF over the view secret and path |
| `modules/fedimint-mint-client/src/lib.rs` | A note store entry that holds either a `SpendableNote` or a `WatchOnlyNote { note, path }`; the spend path emits `InputKey::Remote` for watch-only notes (today it emits `keys: vec![spendable_note.spend_key]`); `release_notes(paths)` and `reclaim_released(notes)` for the ecash flow; the refund key for LNv2 drawn from the refund branch |
| `modules/fedimint-mint-client/src/backup/recovery.rs` | Recovery recognises xpub-derived nonces and view-secret blinding keys |
| `fedimint-core` (later, optional) | Feature-gate `iroh`, `aws-lc-sys`, `tokio` full, `tokio-rustls`, `jsonrpsee-core` behind a `net` feature so a types-only build could run on the device. Not needed for the hackathon |

## fedimint-sdk (fork)

| File | Change |
|---|---|
| `rust/fedimint-sdk/Cargo.toml` | `[patch.crates-io]` entries pointing the touched fedimint crates at the fork branch |
| `rust/fedimint-sdk/src/sdk.rs` | `SdkBuilder::signer(Arc<dyn TransactionSigner>)`; `Sdk::export_view_material(backend)` |
| `rust/fedimint-sdk/src/ecash.rs` | `Ecash::release(amount)` returning the staged note paths and signatures, plus `reclaim`; keep `quote`/`send` for the hot-wallet mode |
| `rust/fedimint-sdk/src/ffi/` | A UniFFI callback interface `TransactionSigner` so Kotlin and Swift implement signing; records for view material and release results; `Notes` unchanged |
| `scripts/build-android-sdk.sh`, `nix/ffi.nix` | Build from the fork; the `pins-agree` CI check will flag the drift, which is expected until upstreaming |

## cdk (fork)

| Crate and file | Change |
|---|---|
| `crates/cashu/src/nuts/nut11/mod.rs` | Builders for the `SIG_ALL` message for swap (inputs, outputs) and melt (inputs, quote id, blank outputs) exactly per NUT-11; remove the wallet-side `SigAllNotSupportedHere` from `verify_p2pk` once the witness path exists |
| `crates/cdk/src/wallet/melt/saga/mod.rs` | Create the NUT-08 blank change outputs before signing, then call the signer, then submit; today it rejects `SIG_ALL` because outputs appear only at confirmation |
| `crates/cdk/src/wallet/` (swap, send, receive, mint) | `trait ProofSigner { async fn sign_sig_all(&self, msg, key_id) -> Signature }`; wallet-wide default `SpendingConditions` so every new output is P2PK `SIG_ALL` to the current batch key; batch rotation per receive; proof selection restricted to one batch per transaction with a consolidation swap when needed |
| `crates/cdk/src/wallet/` (restore) | Deterministic P2PK nonce and `r` from a view seed per keyset and counter; NUT-09 restore that rebuilds those blinded messages |
| `crates/cdk-ffi/` | Expose `ProofSigner` as a UniFFI callback interface, the view-seed setup and the default conditions setting |
| `cdk-kotlin` | Not edited; generate bindings locally with `just ffi-generate kotlin` from the fork until the cdk changes land |

## KeyOS (fork, Stage 5 only)

| Path | Change |
|---|---|
| `api/quantum-link/manifest.toml`, `api/quantum-link/src` | Add our request and response message types with a `permissionGroup` (for example `network-and-pairing.ecash-signer`) and `approval = "grantOnFirstUse"`; model on the Tether patch |
| `os/quantum-link` server | Route the new messages to the app, as the patch does for the WDK messages |
| App manifest | Declare the new messages in `permission_templates.toml` of `prime-ecash-signer` |
| SDK packaging | Work around the missing `api/gpio` by building the app from the KeyOS tree with Tether's `scripts/setup_keyos.sh`, `apply_keyos_integration.sh`, `build_prime_app.sh` and `build_prime_firmware.sh` as templates |
| Later: `os/nfc/src/atsama5d2.rs` | Applet registry by AID for NFC (`10_NFC_FUTURE.md`) |

When QuantumLink v2 lands in KeyOS, the patch is replaced by an RPC route key under our `AppId`, and the
phone side moves to the `quantum-link` crates.

## Our repositories

| Repo | Contents |
|---|---|
| `prime-ecash-signer` | Workspace: `protocol` (CBOR schema, versioned), `fedimint-signer-core`, `cashu-signer-core`, `policy`, the KeyOS app (`src/`, `ui/`, `app-config.toml`, `permission_templates.toml`, `resources/`), golden-vector tests. App id fixed once and never changed |
| `ecash-phone` | Kotlin Android app: fedimint-sdk and CDK bindings, input router, UR transport (hummingbird), file-bridge transport for the simulator, QuantumLink module ported from Tether, signer bridge implementing both callback interfaces |
| `ecash-devtools` | devimint and cdk-mintd launch scripts, host driver for the simulator file bridge, golden-vector generator, demo runbooks |

## Version pinning rules

- fedimint crates: one exact version everywhere, fork keeps `0.13.0-beta.1`.
- cdk: fork from the `v0.18.3` tag; all `cdk-*` crates at that version.
- KeyOS: match the firmware you flash to the SDK the app is built with; Tether's scripts refuse mismatched
  revisions, copy that check.
- `getrandom` stays on 0.2 in the Prime app (platform rule; 0.3 is not routed to the TRNG).
