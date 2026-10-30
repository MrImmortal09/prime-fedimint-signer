# 01. Research findings

Compiled 2026-10-30. Facts were checked against the repositories and documents listed in
`11_SOURCES.md`. File paths refer to the commits inspected that day; line numbers will drift.

## 1. Platform constraints that shape the design

- **No network on Prime, by design.** No IP stack, no Wi-Fi. Outbound paths are USB-C, NFC, camera and
  QuantumLink Bluetooth, all user-initiated. Foundation's capabilities page: "a security choice, not a
  limitation we plan to remove".
- **Third-party app surface (SDK 1.1.0, KeyOS 1.4.0 API):** per-app seed (`GetAppSeed`), TRNG, encrypted
  app storage, hardware AES/SHA/HMAC/Shamir, the system QR/UR scanner and file picker via `ShowModal`,
  PIN re-verification, haptics, LEDs, battery and settings status. Platform reference, section 10.
- **QuantumLink for third-party apps is status and exchange rates only.** The `api/quantum-link`
  manifest at HEAD marks every wallet-sync message (`PublishPsbt`, `SubscribeSignPsbt`,
  `SendAccountUpdate`, ...) `requiredSignature = "foundation"`. The `bt` crate is a stub. The SDK bundle
  also lacks `api/gpio`, so `api/bt` and `api/quantum-link` do not even resolve from the SDK alone
  (documented in `passport-tether-wdk/docs/QUANTUM_LINK.md`). Building with QuantumLink means building
  from a KeyOS source tree.
- **NFC for third-party apps is status only** (`IsEnabled`, `IsActive`). NDEF read and write carry no
  permission group. What the hardware and OS actually do is in `10_NFC_FUTURE.md`.
- **KeyOS 1.4.1** was tagged on 2026-10-21 alongside "sdk: release 1.1.0 for KeyOS 1.4.0". The commits
  since 2026-10-13 touch power, USB, launcher, Slint and update tooling; nothing on QuantumLink.
- **Custom firmware is documented.** KeyOS `DEVELOPMENT.md` describes `just build-all && just flash`
  over SAM-BA mode (hold power 10 s, boot menu, SAM-BA option) and a brick-recovery procedure that
  shorts two board contacts. Foundation offers complimentary dev units to OSS developers via the
  developer portal.
- **Hardware:** SAMA5D27 Cortex-A5 at 500 MHz, 128 MB RAM, 480 x 800 screen, ATECC608C secure element,
  ST25R95 NFC transceiver, separate nRF52805 Bluetooth chip.

## 2. Foundation repositories that matter

| Repo | Why it matters |
|---|---|
| `KeyOS` | OS, services, SDK, reference apps. Patch target for QuantumLink app messages |
| `quantum-link` | QuantumLink v2 crates: `rpc` (request, notification, subscription, upload, download, duplex), `api` (feature modules: app_store, backup, bitcoin, firmware, fx, key, onboarding, scv, system, time), `router`, `keyos` (16-byte `AppId`, `ServiceId`, `RouteId`, `PeerPermissions` listing the apps a peer may reach), `runtime`, `fsm`, `btp`, `wire`, `codec`. Per-app routing is designed in. Last commit 2026-10-19. Not integrated into KeyOS 1.4.x |
| `foundation-api` | Device-to-device API on Blockchain Commons GSTP: `api`, `btp`, `quantum-link-macros`, `abstracted`, `api-demo` |
| `passport-tether-wdk` | The template for a Prime signer with a phone. KeyOS patch `keyos-patches/0001-tether-wdk-app.patch`, firmware scripts (`scripts/setup_keyos.sh`, `apply_keyos_integration.sh`, `build_prime_firmware.sh`), phone QuantumLink module `mobile/modules/quantumlink` (Rust UniFFI + Kotlin BLE + Expo), docs: `PROTOCOL.md`, `QUANTUM_LINK.md`, `ARCHITECTURE.md`, `SECURITY.md`, `VALIDATION.md`. Pinned to KeyOS v1.3.0 |
| `passport-cake-signer` | Sideloadable Monero signer. Documents what a developer-signed app may call on KeyOS 1.4: QR scanner, file picker and PIN prompt via `ShowModal`; QuantumLink "unavailable until KeyOS exposes a third-party-safe API"; Airlock file exchange |
| `passport-agents-wallet` | Spending-policy co-signer for an AI agent. USB CDC transport removed "in favour of QuantumLink v2, which is not in the published SDK yet"; simulator file bridge still works |
| `passport-proof-authority` | Dual-signed approval PoC with a WebSocket bridge to the simulator |
| `hummingbird` | Java UR implementation, usable for animated UR on an Android phone |
| `st25r95`, `rfal-sys`, `nfc-forum-tags`, `ndef-rs`, `iso14443-rs`, `ntag21x` | NFC stack pieces |
| `prime-ble-firmware`, `foundation_ble` | nRF52805 BLE firmware and an iOS BLE library |
| `ngwallet`, `envoy` | Foundation's BDK-based wallet library and the Flutter companion app |

## 3. How existing Prime integrations verify transactions

All of them, Envoy with the Bitcoin app, Tether WDK, Cake signer and the Agents wallet, follow one
pattern:

1. The phone is the networked coordinator. It fetches balances, builds the raw transaction (PSBT or
   EIP-1559 fields) and broadcasts. It never holds a seed.
2. Prime derives keys from its per-app seed, re-parses the raw transaction itself, ignores any labels from
   the phone, enforces policy (fixed derivation path, owned inputs only, one external recipient, bounded
   fees, value and count limits), shows a review computed from the actual bytes, and signs only after a
   physical tap.
3. The phone re-verifies the signed result against its request before broadcasting.
4. Transport is QuantumLink: GSTP authenticated encryption over BLE (Nordic UART service) with BTP
   framing. Pairing is a static BLE QR, then an animated XID QR; the phone pins Prime's XID and rejects
   any other sender. Request IDs are 16 random bytes; one review is active at a time; typed reject and
   cancel messages; a BLE disconnect or unpair clears pending state.

Tether's PoC needed twelve custom QuantumLink messages. "An app cannot add server-side message routing",
so the team patched both `api/quantum-link` and the `os/quantum-link` server and built firmware. The Cake
signer keeps its QuantumLink code behind `cfg(foundation_internal_quantum_link)` for the same reason.

## 4. Fedimint: wallets and client internals

Wallets: Fedi (AGPL since January 2026, React Native), Ecash App (Flutter, fedimint org), Harbor
(desktop), Vipr (PWA), Conduit (iOS and Android), Bitsacco (fedimint-clientd). None separates signing
from the networked client. No Fedimint hardware signer or device app exists.

Client facts (fedimint `main`, 2026-10-29):

- `fedimint-client-module/src/transaction/builder.rs`: `ClientInput { input, keys: Vec<Keypair>,
  amounts }`. `TransactionBuilder::build` draws a random nonce, computes `txid =
  Transaction::tx_hash_from_parts(inputs, outputs, nonce)` and signs synchronously, one Schnorr
  signature per key, into `TransactionSignature::NaiveMultisig(Vec<Signature>)`.
- `fedimint-core/src/transaction.rs`: `validate_signatures(pub_keys)` checks exactly one signature per
  input key. For a mint input that key is the note nonce, a secp256k1 public key.
- `modules/fedimint-mint-client/src/lib.rs`: `SpendableNote { signature: tbs::Signature, spend_key:
  Keypair }`. The spend path builds `ClientInput { input: MintInput::new_v0(amount, note), keys:
  vec![spendable_note.spend_key], .. }`. `OOBNotes` is a list of parts: notes, federation id prefix,
  optional invite. `Display` is base64url of the consensus encoding; `FromStr` accepts
  `fedimint`-prefixed base32, base64url and base64.
- `modules/fedimint-mint-client/src/output.rs`: per note, `spend_key = secret.child_key(0).to_secp_key()`
  and `blinding_key = secret.child_key(1).to_bls12_381_key()`, where `secret` comes from the module root
  by note index and amount tier. HKDF-style, hardened only, no public derivation.
- `modules/fedimint-mint-common/src/lib.rs`: `Note { nonce, signature }`; `Note::verify(pk)` is
  `tbs::verify(nonce.to_message(), signature, pk)`; `MintInputV0 { amount, note }`;
  `MintOutputV0 { amount, blind_nonce }`.
- `crypto/tbs/src/lib.rs`: `verify(msg, sig, pk) = pairing(msg, pk) == pairing(sig, G2::generator())`.
  Message hashing uses the tag `TBS_BLS12-381_`, SHA-3, and a ChaCha-seeded `G1Projective::random`.
  Dependencies: `bls12_381`, `group`, `sha3`, `rand_chacha`, and `fedimint-core` (for encoding traits).
- `modules/fedimint-lnv2-common/src/contracts.rs`: `OutgoingContract { payment_image, amount,
  expiration, claim_pk, refund_pk, ephemeral_pk }`.
- `modules/fedimint-wallet-common/src/lib.rs`: `PegOut { recipient, amount, fees }`.
- `fedimint-core/src/base32.rs`: `FEDIMINT_PREFIX = "fedimint"`. `fedimint-core/src/invite_code.rs`:
  bech32 HRP `fed1`.
- **fedimint-core does not build for Prime.** On every non-wasm target it depends on `iroh`,
  `iroh-relay`, `aws-lc-sys` (bindgen), `tokio` with `full`, `tokio-rustls`, `rustls-webpki` and
  `jsonrpsee-core`. The wasm path swaps some of these but still needs `iroh`.

fedimint-sdk (`rust/fedimint-sdk`, 0.1.0-beta.2, repo updated 2026-10-28): one API over
fedimint-client with `Ecash`, `Lightning`, `Onchain` and `Meta` facades plus recovery; pinned to
fedimint `=0.13.0-beta.1`; UniFFI bindings for Kotlin, Swift, JS and React Native; RocksDB on native
targets; `Notes::parse` handles v1 `OOBNotes` and mintv2 `ECash`; `Ecash::quote` refuses a send that
would need the wallet to reissue change. Known issue: on LN v1-only federations a successful send ends
with an `Internal` error (fedimint/fedimint#8969).

## 5. Cashu: SDK state and the card protocol

- CDK v0.18.3 (2026-10-29), self-described alpha, "the api will change". Crates include `cashu`
  (protocol types), `cdk` (wallet and mint), `cdk-ffi` (UniFFI for Kotlin, Swift, Python, Dart, Go),
  `cdk-mintd`, `cdk-fake-wallet` (test Lightning backend), `cdk-cli`, `cdk-sqlite`, `cdk-redb`. Mobile
  packages live in `cdk-kotlin` and `cdk-swift`.
- The mint side enforces P2PK `SIG_ALL` on swap and melt (tests under
  `crates/cdk/src/mint/swap/tests/p2pk_sigall_spending_conditions_tests.rs` and the melt equivalent).
- The wallet side refuses `SIG_ALL`: `crates/cashu/src/nuts/nut11/mod.rs` `verify_p2pk` returns
  `SigAllNotSupportedHere`; `crates/cdk/src/wallet/melt/saga/mod.rs` rejects `SIG_ALL` because "its
  signature must commit to the melt request's change outputs, which are only created at confirmation".
- `crates/cashu/src/nuts/nut06.rs`: mint info advertises `nut11` and `nut12` support flags.
- NUT-11: `SIG_INPUTS` signs only `Proof.secret`, so a signature authorises spending that proof to any
  outputs. `SIG_ALL` signs all inputs and outputs, and for melt also the quote id and the NUT-08 blank
  outputs. All inputs of a `SIG_ALL` transaction must share the same `Secret.data` and tags, and only
  the first input carries the witness.
- NUT-00 token forms: `cashuA` (V3, base64 JSON, may hold several mints), `cashuB` (V4, base64url CBOR,
  one mint), `cashu:` URI wrapper, and a binary form `utf8("craw") || version || cbor`. NUT-18 payment
  requests are `creqA` + base64url CBOR.
- NUT-13 deterministic secrets: HMAC-SHA256 derivation for keyset ids starting with `01`, legacy BIP32
  derivation for `00`. It does not cover P2PK secrets.
- NUT-CCP, the Cashu Card Protocol (draft): a JavaCard makes a key at install, every proof on it is
  locked to that key with `SIG_ALL`, the reader builds the swap, the card signs once and marks slots
  spent in the same atomic write, and the reader submits when online. Change openings stay on the card.
  Reference applet on NXP JCOP4 cards, little outside review, point-of-sale reference app "Numo".

## 6. Nothing like this exists on Prime

Foundation's app showcase lists 25 apps and none mentions ecash, Cashu, Fedimint or a mint. The org's
85 repositories contain nothing ecash-related. GitHub repository search for KeyOS or Passport Prime with
cashu, fedimint or ecash returns zero results. The closest shapes are Spark Signer and Tether Wallet,
where the phone runs the protocol and Prime signs.
