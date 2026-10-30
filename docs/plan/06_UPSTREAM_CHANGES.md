# 06. Upstream changes by repository

File paths are from the commits inspected on 2026-10-30. Each section lists what to change, why, and
how the change stays upstreamable.

## fedimint (fork from tag `v0.13.0-beta.1`)

**Why this tag:** fedimint-sdk requires every fedimint crate at `=0.13.0-beta.1`, and fedimint crates
require each other with `=`. A fork that keeps the version string can be substituted with
`[patch.crates-io]` in fedimint-sdk's `Cargo.toml` for every crate we touch.

| Crate and file | Change |
|---|---|
| `fedimint-client-module/src/transaction/builder.rs` | `ClientInput.keys: Vec<Keypair>` becomes `Vec<InputKey>` with `InputKey::Local(Keypair)` and `InputKey::Remote { key_id }`. Add `trait TransactionSigner { async fn sign(&self, txid, keys: &[RemoteKeyRef], context) -> Result<Vec<Signature>> }`. `TransactionBuilder::build` becomes async, signs local keys itself and asks the signer for remote ones, keeps input order |
| `fedimint-client/src/client.rs` | `finalize_transaction` and `finalize_and_submit_transaction*` pass the client's signer into `build`; `ClientBuilder` gains `with_signer(Arc<dyn TransactionSigner>)` defaulting to a local signer, so existing users see no change |
| `modules/fedimint-mint-client/src/output.rs` | `NoteIssuanceRequest::new` gets a derivation strategy: upstream HKDF, or `Xpub { xpub, view_secret, tier, index }` where the nonce is the xpub child and the blinding key is HKDF over the view secret and path |
| `modules/fedimint-mint-client/src/lib.rs` | A note store entry that holds either a `SpendableNote` or a `WatchOnlyNote { note, path }`; the spend path emits `InputKey::Remote` for watch-only notes (today it emits `keys: vec![spendable_note.spend_key]`); `release_notes(paths)` and `reclaim_released(notes)` for the ecash flow; the existing timeout-reclaim (`await_spend_oob_refund`) routed through the signer when the keys are not local |
| `modules/fedimint-lnv2-client` | Refund key for outgoing contracts drawn from the signer's refund branch instead of the client root |
| `modules/fedimint-mint-client/src/backup/recovery.rs` | Recovery recognises xpub-derived nonces and view-secret blinding keys |
| `fedimint-core` (later, optional) | Feature-gate `iroh`, `aws-lc-sys`, `tokio` full, `tokio-rustls`, `jsonrpsee-core` behind a `net` feature so a types-only build could run on the device. Not needed for the hackathon |

## fedimint-sdk (fork)

| File | Change |
|---|---|
| `rust/fedimint-sdk/Cargo.toml` | `[patch.crates-io]` entries pointing the touched fedimint crates at the fork branch |
| `rust/fedimint-sdk/src/sdk.rs` | `SdkBuilder::signer(Arc<dyn TransactionSigner>)`; `Sdk::export_view_material()` |
| `rust/fedimint-sdk/src/ecash.rs` | `Ecash::release(amount)` returning staged note paths and signatures, `Ecash::reclaim`; keep `quote` and `send` for the hot-wallet mode |
| `rust/fedimint-sdk/src/lightning.rs`, `onchain.rs` | No API change; the signer is invoked inside the client when a spend is built |
| `rust/fedimint-sdk/src/ffi/` | A UniFFI callback interface `TransactionSigner` so JS, Kotlin and Swift implement signing; records for view material and release results |
| `js/react-native/`, `scripts/build-android-sdk.sh`, `nix/ffi.nix` | Regenerate bindings from the fork; the `pins-agree` CI check will flag the drift, which is expected until upstreaming |

## KeyOS (fork, Stages 4 and 5 only)

| Path | Change |
|---|---|
| `api/quantum-link/manifest.toml`, `api/quantum-link/src` | Add our request and response message types with a `permissionGroup` (for example `network-and-pairing.fedimint-signer`) and `approval = "grantOnFirstUse"`; model on the Tether patch |
| `os/quantum-link` server | Route the new messages to the app, as the Tether patch does for the WDK messages |
| App manifest | Declare the new messages in `permission_templates.toml` of `prime-fedimint-signer` |
| SDK packaging | Work around the missing `api/gpio` by building the app from the KeyOS tree with Tether's `scripts/setup_keyos.sh`, `apply_keyos_integration.sh`, `build_prime_app.sh` and `build_prime_firmware.sh` as templates |
| `api/nfc/manifest.toml`, `os/nfc/src/{lib.rs,atsama5d2.rs}` (Stage 5) | `RegisterApplet { aid }` and APDU forward and reply messages with a permission group; dispatch in `handle_apdu_cmd` by registered AID next to the FIDO AID; longer session timeouts for the two-tap flow (`09_NFC.md`) |

When QuantumLink v2 lands in KeyOS, the Bluetooth patch is replaced by an RPC route key under our
`AppId`, and the phone side moves to the `quantum-link` crates.

## Our repositories

| Repo | Contents |
|---|---|
| `prime-fedimint-signer` | Workspace: `protocol` (CBOR schema, versioned), `fedimint-signer-core`, `policy`, the KeyOS app (`src/`, `ui/`, `app-config.toml`, `permission_templates.toml`, `resources/`), golden-vector tests. App id fixed once and never changed |
| `fedimint-phone` | The wallet app: fedimint-sdk bindings, input router, UR transport, file-bridge transport for the simulator, QuantumLink module from Tether, NFC reader transport (optional), signer bridge implementing the callback interface |
| `fedimint-signer-devtools` | devimint launch scripts, host driver for the simulator file bridge, golden-vector generator, demo runbooks |

## Version pinning rules

- fedimint crates: one exact version everywhere; the fork keeps `0.13.0-beta.1`.
- KeyOS: match the firmware you flash to the SDK the app is built with; Tether's scripts refuse mismatched
  revisions, copy that check.
- `getrandom` stays on 0.2 in the Prime app (platform rule; 0.3 is not routed to the TRNG).
