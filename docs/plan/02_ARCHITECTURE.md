# 02. Architecture

## Principle

Prime is a **signer with a trusted display**, not a wallet. The phone is the networked Fedimint wallet
and never holds a spend key. Every spend is decoded on Prime from raw bytes, shown on Prime's screen,
and signed only after a tap. Every receive is phone-only. This is the pattern every existing Prime
integration uses (`01_RESEARCH_FINDINGS.md`, section 3), applied to Fedimint.

## Components

```
Phone (Android first)                               Passport Prime (KeyOS app, Rust + Slint)
┌───────────────────────────────┐                   ┌────────────────────────────────────┐
│ UI, history, input router     │                   │ review screens, policy, PIN / tap  │
│ fedimint-sdk (RN or Android) ─┼─┐  signer bridge  │ fedimint-signer-core               │
│ view material, watch-only     │ │                 │ spend keys derived from GetAppSeed │
│ notes, operation log          │ │                 │ released-path log                  │
│ transports: UR, file, QL ─────┼─┘◄───────────────►│ transports: UR, file, QL, (NFC)    │
└───────────────┬───────────────┘   protocol (CBOR) └────────────────────────────────────┘
                │ websockets
          federation guardians, gateway
```

A shared Rust crate `protocol` holds the CBOR message schema and compiles into both sides.

## Key split

The full specification is `docs/KEY_SCHEME.md` (written in Phase 1). Summary, corrected in Phase 0:

- Per federation, Prime derives a spend root `(k, c)` and a view secret `v` from `GetAppSeed` and the
  federation id. Prime exports `(K = k·G, c, v)`.
- Mint v2 note spend keys are **non-hardened** children `K + t·G`, with `t` an HMAC of the chain code
  over the note's denomination and its public 16-byte tweak. Mint v2 publishes the tweak in every
  output, so there are no counters to track.
- Blinding keys and the tweak-grinding filter derive from the view secret. The phone derives every
  nonce and every blinding key, so it can mint, receive, recover and rescan without Prime. It cannot
  spend.
- LNv2 refund keys are spend-branch children keyed by the contract's ephemeral point, so a refund
  always needs Prime.
- Notes staged for an ecash send live on a **hardened outgoing branch** derived from a Prime-only
  secret. Only these keys are ever released (D11): one released non-hardened key plus the xpub would
  reveal the root.
- This replaces fedimint's HKDF per-note derivation, where blinding and spend keys share one parent
  secret. Notes created under the new scheme are recoverable only by a client that knows the scheme.
- Recovery: a new phone receives the xpub and view secret again and rescans the federation. A new Prime
  restored from the same seed and the same app id reproduces the same tree. Never change the app id.

## Input identification

| Input | What it is | Handling |
|---|---|---|
| `fedimint…` base32 | Fedimint ecash in the newer encoding (v1 base32 form or mintv2) | fedimint-sdk `Notes::parse` |
| bare base64 or base64url | Fedimint v1 ecash, which has no prefix | fedimint-sdk `Notes::parse`, tried last |
| `fed1…` | Fedimint invite code | join or preview |
| `lnbc…`, `lntb…`, `lnbcrt…`, `lightning:` | BOLT11 invoice | pay |
| `lnurl1…` | LNURL | pay or withdraw |
| `bc1…`, `tb1…`, `bcrt1…` | Bitcoin address | withdraw |
| `ur:…` of our own types | Signer protocol messages | protocol |
| `cashuA…`, `cashuB…`, `cashu:`, `creqA…` | Cashu token or request | recognised, shown as "not supported in this version" |

Prefixes are matched first. Fedimint v1 is tried last because it is the only unmarked format.

## Transports

All transports carry the same CBOR messages. Priority:

1. **QuantumLink (Bluetooth)** through a KeyOS patch modelled on `passport-tether-wdk`, until QuantumLink
   v2 ships a third-party channel. Pairing: static BLE QR, then animated identity QR; identity pinning;
   replay protection; one active review; typed reject and cancel.
2. **Animated UR** through the system scanner and the UR widgets. Works on a stock retail Prime. The
   phone uses a UR library (`hummingbird` on Kotlin, a bc-ur package on React Native). Register
   `[[qr-match-rules]]` for our UR types so the launcher opens the app.
3. **Simulator file bridge** for CI and daily development (pattern from `passport-agents-wallet`).
4. **NFC**, optional stretch, two-tap design: `09_NFC.md`.

### Request schema (protocol crate)

| Kind | Direction | Payload |
|---|---|---|
| `ImportFederationConfig` | phone → Prime | client config bytes, expected federation id |
| `ExportViewMaterial` | phone → Prime | reply: xpub, view secret, scheme version |
| `Sign` | phone → Prime | federation id, raw unsigned transaction, input key paths, change path hints, context: `Lightning { bolt11 }`, `PegOut`, `Reissue`, `Refund`, `Split`, `Reclaim` |
| `Release` | phone → Prime | note paths and mint signatures to release; reply: token, or "shown on screen" |
| `Status` | phone → Prime | reply: released paths, last signed request ids, policy state |
| `Signatures`, `Token`, `Reject`, `Cancel` | replies | request id echoed, typed reason on reject |

Rules: 16-byte random request id; one review at a time; a reply must match its id; cancel clears pending
state; disconnect clears all pending state; a version field on every message.

## What Prime verifies before signing

| Check | How |
|---|---|
| Inputs are mine | Re-derive each input's nonce from its denomination and tweak and compare |
| Inputs are genuine | Optionally verify each note's BLS signature against the stored per-denomination mint keys. Defence in depth only: the federation rejects a forged note, and the transaction is atomic |
| Change is mine | Re-derive the blinded nonce from the output's own denomination and tweak and compare; the tweak must pass the recovery filter |
| Payment is what it claims | LNv2 contract payment hash equals the invoice hash, amount equals invoice plus quoted fee; LN v1 contract checked the same way |
| Peg-out | Exactly one external output; address and amount shown; fee read from the output |
| Refund cannot leak | The contract's refund key sits in Prime's refund branch |
| Self-transfer | Every output re-derives to my tree: shown as split, reissue or reclaim, optional lighter approval |
| Nothing hidden | An output of an unknown module or a second external output is refused |
| Policy | Optional per-transaction and daily limits |
| Display | Federation name from the stored config, destination, amount, fee as inputs minus outputs, change |

## Receives need nothing from Prime

Incoming Lightning contracts are claimed with one-shot phone keys; incoming ecash is reissued with the
keys inside the token; on-chain deposits use phone-held tweak keys. All outputs go to Prime's tree.

## Prime-side code constraints

fedimint-core cannot be compiled for `armv7a-unknown-xous-elf`. Prime therefore uses one small standalone
crate with no network dependencies:

- `fedimint-signer-core`: consensus decoding of transaction parts, module input and output wrappers,
  mint v2 input and output, LNv2 outgoing contract and refund input, wallet v2 peg-out, amounts;
  transaction hash; Schnorr via `secp256k1`; BLS verification via `bls12_381` with the tbs
  hash-to-curve re-implemented (mint v2 uses the SHA-256 variant). The v1 module types come later.

It is validated against golden vectors produced on the host by the real upstream crates.

## Phone-side stack

Android first. Two viable stacks:

- **React Native with Expo (recommended):** fedimint-sdk ships `@fedimint/react-native`, Tether's
  QuantumLink module is an Expo module usable as-is, and JS UR and QR libraries exist. Most reuse.
- **Kotlin:** fedimint-sdk Android SDK, `hummingbird` for UR, a small re-wrap of the Kotlin and Rust
  inside Tether's module. Fallback if the React Native bindings prove flaky.

iOS follows with the fedimint-sdk Swift package and `foundation_ble`.

## Alternatives considered

| Option | Pros | Cons |
|---|---|---|
| **A. Signer (chosen)** | Real security boundary; matches Foundation's model; recovery without Prime; first of its kind for Fedimint | Needs client-library forks and a KeyOS patch for Bluetooth; two codebases |
| B. Bearer-note vault on Prime | No fedimint changes; works on a stock device today; offline BLS checks | The phone that minted the notes saw the keys; no double-spend check offline; cash in a safe, not a wallet |
| C. Seed vault only | Trivial | Low value; recovery still needs a rescan |
| D. Full client on Prime, phone as proxy | Prime owns everything | fedimint-client needs tokio, RocksDB, websockets; a port to a 500 MHz Cortex-A5 is months |
| E. 2-of-2 MuSig note keys | Both devices required | Interactive rounds, more complexity, no gain over Prime's PIN |
| F. A plus cached notes on Prime | Prime pays face to face with no phone | State desync with the phone; the receiver still needs the federation. Revisited in `09_NFC.md` as tap-to-pay |
