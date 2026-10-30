# 01. Research findings

Compiled 2026-10-30. Facts were checked against the repositories and documents in `11_SOURCES.md`.
File paths refer to the commits inspected that day; line numbers will drift.

## 1. Platform constraints that shape the design

- **No network on Prime, by design.** No IP stack, no Wi-Fi. Outbound paths are USB-C, NFC, camera and
  QuantumLink Bluetooth, all user-initiated. Foundation calls this "a security choice, not a limitation
  we plan to remove".
- **Third-party app surface (SDK 1.1.0, KeyOS 1.4.0 API):** per-app seed (`GetAppSeed`), TRNG, encrypted
  app storage, hardware AES/SHA/HMAC/Shamir, the system QR/UR scanner and file picker via `ShowModal`,
  PIN re-verification, haptics, LEDs, battery and settings status. Platform reference, section 10.
- **QuantumLink for third-party apps is status and exchange rates only.** The `api/quantum-link`
  manifest at HEAD marks every wallet-sync message (`PublishPsbt`, `SubscribeSignPsbt`,
  `SendAccountUpdate`, ...) `requiredSignature = "foundation"`. The `bt` crate is a stub. The SDK bundle
  also lacks `api/gpio`, so `api/bt` and `api/quantum-link` do not resolve from the SDK alone
  (`passport-tether-wdk/docs/QUANTUM_LINK.md`). Building with QuantumLink means building from a KeyOS
  source tree.
- **NFC for third-party apps is status only** (`IsEnabled`, `IsActive`). NDEF read and write carry no
  permission group. What the hardware and OS actually do is in `09_NFC.md`.
- **KeyOS 1.4.1** was tagged on 2026-10-21 alongside "sdk: release 1.1.0 for KeyOS 1.4.0". Commits since
  2026-10-13 touch power, USB, launcher, Slint and update tooling; nothing on QuantumLink or NFC.
- **Custom firmware is documented.** KeyOS `DEVELOPMENT.md` describes `just build-all && just flash` over
  SAM-BA mode (hold power 10 s, boot menu, SAM-BA option) and a brick-recovery procedure. Foundation
  offers complimentary dev units to OSS developers through the developer portal.
- **Hardware:** SAMA5D27 Cortex-A5 at 500 MHz, 128 MB RAM, 480 x 800 screen, ATECC608C secure element,
  ST25R95 NFC transceiver, separate nRF52805 Bluetooth chip.

## 2. Foundation repositories that matter

| Repo | Why it matters |
|---|---|
| `KeyOS` | OS, services, SDK, reference apps. Patch target for QuantumLink app messages and, later, an NFC applet registry |
| `quantum-link` | QuantumLink v2 crates: `rpc`, `api`, `router`, `keyos` (16-byte `AppId`, `ServiceId`, `PeerPermissions` listing the apps a peer may reach), `runtime`, `fsm`, `btp`, `wire`, `codec`. Per-app routing is designed in. Last commit 2026-10-19. Not integrated into KeyOS 1.4.x |
| `foundation-api` | Device-to-device API on Blockchain Commons GSTP: `api`, `btp`, `quantum-link-macros` |
| `passport-tether-wdk` | The template for a Prime signer with a phone: KeyOS patch `keyos-patches/0001-tether-wdk-app.patch`, firmware scripts, phone QuantumLink module `mobile/modules/quantumlink` (Expo module: Rust UniFFI + Kotlin BLE), docs `PROTOCOL.md`, `QUANTUM_LINK.md`, `ARCHITECTURE.md`, `SECURITY.md`. Pinned to KeyOS v1.3.0 |
| `passport-cake-signer` | Sideloadable Monero signer; documents what a developer-signed app may call on KeyOS 1.4 and keeps QuantumLink behind `cfg(foundation_internal_quantum_link)` |
| `passport-agents-wallet` | Spending-policy co-signer; USB CDC transport removed "in favour of QuantumLink v2, which is not in the published SDK yet"; simulator file bridge pattern |
| `hummingbird` | Java UR implementation, for animated UR on Android |
| `st25r95`, `rfal-sys`, `nfc-forum-tags`, `ndef-rs`, `iso14443-rs`, `ntag21x` | NFC stack pieces |
| `prime-ble-firmware`, `foundation_ble` | nRF52805 BLE firmware and an iOS BLE library |

## 3. How existing Prime integrations verify transactions

All of them, Envoy with the Bitcoin app, Tether WDK, Cake signer and the Agents wallet, follow one
pattern:

1. The phone is the networked coordinator. It fetches balances, builds the raw transaction (PSBT or
   EIP-1559 fields) and broadcasts. It never holds a seed.
2. Prime derives keys from its per-app seed, re-parses the raw transaction itself, ignores any labels from
   the phone, enforces policy (fixed derivation path, owned inputs only, one external recipient, bounded
   fees), shows a review computed from the actual bytes, and signs only after a physical tap.
3. The phone re-verifies the signed result against its request before broadcasting.
4. Transport is QuantumLink: GSTP authenticated encryption over BLE (Nordic UART service) with BTP
   framing. Pairing: static BLE QR, then animated XID QR; the phone pins Prime's XID. Request IDs are 16
   random bytes; one review active at a time; typed reject and cancel; a disconnect clears pending state.

Tether's PoC needed twelve custom QuantumLink messages. "An app cannot add server-side message routing",
so they patched both `api/quantum-link` and the `os/quantum-link` server and built firmware.

## 4. Fedimint: wallets and client internals

Wallets: Fedi (AGPL since January 2026, React Native), Ecash App (Flutter, fedimint org), Harbor
(desktop), Vipr (PWA), Conduit (iOS and Android), Bitsacco (fedimint-clientd). None separates signing
from the networked client. No Fedimint hardware signer or device app exists.

Client facts (fedimint `main`, 2026-10-29):

- `fedimint-client-module/src/transaction/builder.rs`: `ClientInput { input, keys: Vec<Keypair>,
  amounts }`. `TransactionBuilder::build` draws a random nonce, computes
  `txid = Transaction::tx_hash_from_parts(inputs, outputs, nonce)` and signs synchronously, one Schnorr
  signature per key, into `TransactionSignature::NaiveMultisig(Vec<Signature>)`.
- `fedimint-core/src/transaction.rs`: `validate_signatures(pub_keys)` checks exactly one signature per
  input key. For a mint input that key is the note nonce, a secp256k1 public key.
- `modules/fedimint-mint-client/src/lib.rs`: `SpendableNote { signature: tbs::Signature, spend_key:
  Keypair }`. The spend path builds `ClientInput { input: MintInput::new_v0(amount, note), keys:
  vec![spendable_note.spend_key], .. }`. `OOBNotes` is a list of parts: notes, federation id prefix,
  optional invite. `Display` is base64url of the consensus encoding; `FromStr` accepts
  `fedimint`-prefixed base32, base64url and base64. `spend_notes`, `try_cancel_spend_notes` and
  `await_spend_oob_refund` implement the send, cancel and timeout-reclaim behaviour.
- `modules/fedimint-mint-client/src/output.rs`: per note, `spend_key = secret.child_key(0).to_secp_key()`
  and `blinding_key = secret.child_key(1).to_bls12_381_key()`, where `secret` comes from the module root
  by note index and amount tier. HKDF-style, hardened only, no public derivation.
- `modules/fedimint-mint-common/src/lib.rs`: `Note { nonce, signature }`; `Note::verify(pk)` is
  `tbs::verify(nonce.to_message(), signature, pk)`; `MintInputV0 { amount, note }`;
  `MintOutputV0 { amount, blind_nonce }`.
- `crypto/tbs/src/lib.rs`: `verify(msg, sig, pk) = pairing(msg, pk) == pairing(sig, G2::generator())`.
  Message hashing uses the tag `TBS_BLS12-381_`, SHA-3, and a ChaCha-seeded `G1Projective::random`.
  Dependencies: `bls12_381`, `group`, `sha3`, `rand_chacha`, and `fedimint-core` for encoding traits.
- `modules/fedimint-lnv2-common/src/contracts.rs`: `OutgoingContract { payment_image, amount,
  expiration, claim_pk, refund_pk, ephemeral_pk }`.
- `modules/fedimint-wallet-common/src/lib.rs`: `PegOut { recipient, amount, fees }`.
- `fedimint-core/src/base32.rs`: `FEDIMINT_PREFIX = "fedimint"`. `fedimint-core/src/invite_code.rs`:
  bech32 HRP `fed1`.
- **fedimint-core does not build for Prime.** On every non-wasm target it depends on `iroh`,
  `iroh-relay`, `aws-lc-sys` (bindgen), `tokio` with `full`, `tokio-rustls`, `rustls-webpki` and
  `jsonrpsee-core`.

fedimint-sdk (`rust/fedimint-sdk`, 0.1.0-beta.2, repo updated 2026-10-28): one API over
fedimint-client with `Ecash`, `Lightning`, `Onchain` and `Meta` facades plus recovery; pinned to
fedimint `=0.13.0-beta.1`; UniFFI bindings for Kotlin, Swift, JS and React Native, with a
`@fedimint/react-native` package; RocksDB on native targets; `Notes::parse` handles v1 `OOBNotes` and
mintv2 `ECash`; `Ecash::quote` refuses a send that would need the wallet to reissue change. Known issue:
on LN v1-only federations a successful send ends with an `Internal` error (fedimint/fedimint#8969).

## 5. Cashu, for the record

Cashu was in the first plan and was dropped on the same day. Reasons, in short: Cashu proofs are bearer
secrets with no signature, so a signer only matters if every proof is locked to a Prime key with
`SIG_ALL`; the Cashu Development Kit's wallet refuses to spend `SIG_ALL` proofs today; the lock makes
proofs linkable by the mint; recovery needs a custom scheme; each mint must be tested for `SIG_ALL`
support. That is roughly the Fedimint effort again for a weaker security and privacy story. The full
analysis is `old/04_CASHU_NOTES.md`.

## 6. Nothing like this exists on Prime

Foundation's app showcase lists 25 apps and none mentions ecash or Fedimint. The org's 85 repositories
contain nothing ecash-related. GitHub repository search for KeyOS or Passport Prime with fedimint, cashu
or ecash returns zero results. The closest shapes are Spark Signer and Tether Wallet, where the phone runs
the protocol and Prime signs.
