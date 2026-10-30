# 02. Architecture

## Principle

Prime is a **signer with a trusted display**, not a wallet. The phone is the networked wallet for both
ecash systems and never holds a spend key. Every spend is decoded on Prime from raw bytes, shown on
Prime's screen, and signed only after a tap. Every receive is phone-only. This is the pattern every
existing Prime integration uses (`01_RESEARCH_FINDINGS.md`, section 3), applied to Fedimint and Cashu.

## Components

```
Phone (Android, Kotlin)                            Passport Prime (KeyOS app, Rust + Slint)
┌───────────────────────────────┐                  ┌────────────────────────────────────┐
│ UI, history, input router     │                  │ review screens, policy, PIN / tap  │
│ fedimint-sdk (Android)   ─────┼─┐                │ fedimint-signer-core               │
│ CDK (cdk-ffi Kotlin)     ─────┼─┤ signer bridge  │ cashu-signer-core                  │
│ view material, notes, proofs  │ │                │ spend keys derived from GetAppSeed │
│ transports: UR, file, QL ─────┼─┘◄──────────────►│ transports: UR, file, QL           │
└───────────────┬───────────────┘  protocol (CBOR) └────────────────────────────────────┘
                │ HTTPS / websockets
       federations and mints
```

A shared Rust crate `protocol` holds the CBOR message schema and compiles into both sides.

## Key split

### Fedimint

- Prime derives a spend root from `GetAppSeed`. Note spend keys are BIP32 **non-hardened** children at
  `m/<tier>/<index>`, where tier is the log2 of the denomination (denominations are powers of two in
  msat). Prime exports the xpub.
- Blinding keys derive from a separate **view secret** that Prime exports. The phone derives every nonce
  and every blinding key, so it can mint, receive, recover and rescan without Prime. It cannot spend.
- Dedicated branches: `refund/<i>` for LNv2 refund keys and `outgoing/<i>` for notes staged for an ecash
  release.
- This replaces fedimint's HKDF per-note derivation, where blinding and spend keys share one parent
  secret. Notes created under the new scheme are recoverable only by a client that knows the scheme.

### Cashu

- Every proof the wallet holds is locked with a NUT-11 P2PK secret to a Prime key, with
  `sigflag SIG_ALL`, the same construction the Cashu card uses. Lock keys rotate per receive batch to
  limit linkability (`04_CASHU_NOTES.md`). Prime holds the private keys at `cashu/<mint>/<batch>`; the
  phone holds the public keys.
- Proof nonces and blinding factors derive from the view secret per keyset and counter, so recovery works
  from Prime's seed through NUT-09 restore.

## Token and input identification

| Input | What it is | Backend |
|---|---|---|
| `cashuA…` | Cashu token V3, base64 JSON, may span several mints | CDK |
| `cashuB…` | Cashu token V4, base64url CBOR, one mint | CDK |
| `cashu:` URI | A Cashu token with a URI prefix | strip, re-route |
| `craw` + version byte | Binary Cashu token for QR or NFC | CDK |
| `creqA…` | Cashu payment request (NUT-18) | CDK |
| `fedimint…` base32 | Fedimint ecash in the newer encoding (v1 base32 form or mintv2) | fedimint-sdk |
| bare base64 or base64url | Fedimint v1 ecash, which has no prefix | fedimint-sdk `Notes::parse`, tried last |
| `fed1…` | Fedimint invite code | fedimint-sdk |
| `lnbc…`, `lntb…`, `lnbcrt…`, `lightning:` | BOLT11 invoice | either; the user picks the balance to pay from |
| `lnurl1…` | LNURL | either |
| `https://…` answering mint info | Cashu mint URL | CDK |
| `ur:…` of our own types | Signer protocol messages | protocol |

Prefixes are matched first. Fedimint v1 is tried last because it is the only unmarked format.

## Transports

All transports carry the same CBOR messages. Priority:

1. **QuantumLink (Bluetooth)** through a KeyOS patch modelled on `passport-tether-wdk`, until QuantumLink
   v2 ships a third-party channel. Pairing: static BLE QR, then animated identity QR; identity pinning;
   replay protection; one active review; typed reject and cancel.
2. **Animated UR** through the system scanner and the UR widgets. Works on a stock retail Prime. The
   phone uses `hummingbird`. Register `[[qr-match-rules]]` for our UR types so the launcher opens the app.
3. **Simulator file bridge** for CI and daily development (pattern from `passport-agents-wallet`).

### Request schema (protocol crate)

| Kind | Direction | Payload |
|---|---|---|
| `ImportFederationConfig` | phone → Prime | client config bytes, expected federation id |
| `ImportMintKeys` | phone → Prime | mint URL, keyset ids, per-amount public keys, unit, mint info NUT flags |
| `ExportViewMaterial` | phone → Prime | backend. Reply: xpub and view secret (Fedimint) or lock public keys and view seed (Cashu) |
| `FedimintSign` | phone → Prime | federation id, raw unsigned transaction, input key paths, change path hints, context (BOLT11 invoice, peg-out, reissue, refund) |
| `FedimintRelease` | phone → Prime | note paths and mint signatures to release. Reply: token, or "shown on screen" |
| `CashuSign` | phone → Prime | mint URL, unit, inputs (proofs), outputs with openings (secret, r), melt quote plus invoice, or swap intent |
| `Signatures`, `Token`, `Reject`, `Cancel` | replies | request id echoed, typed reason on reject |

Rules: 16-byte random request id; one review at a time; a reply must match its id; cancel clears pending
state; disconnect clears all pending state; a version field on every message.

## What Prime verifies

### Fedimint spend

| Check | How |
|---|---|
| Inputs are mine | Re-derive each input's nonce at the given path and compare |
| Inputs are genuine | Verify each note's BLS signature against the stored per-tier mint keys |
| Change is mine | Re-derive the blind nonce at each hinted path and compare; reject paths beyond the gap limit |
| Payment is what it claims | LNv2 contract payment hash equals the invoice hash, amount equals invoice plus quoted fee; LN v1 contract checked the same way |
| Peg-out | Exactly one external output; address and amount shown; fee read from the output |
| Refund cannot leak | The contract's refund key sits in Prime's refund branch |
| Nothing hidden | An output of an unknown module or a second external output is refused |
| Policy | Optional per-transaction and daily limits |
| Display | Federation name from the stored config, destination, amount, fee as inputs minus outputs, change |

### Cashu spend

| Check | How |
|---|---|
| Inputs are mine | Each input secret is P2PK to a Prime key with `SIG_ALL`; all inputs share one key |
| Outputs | Recompute `B_ = hash_to_curve(secret) + r·G` from each opening; classify as change (locked to me) or leaving |
| Melt | Quote id, invoice hash, amount and fee reserve consistent; the invoice is shown |
| Mint identity | Mint URL shown prominently; keys match the imported keyset |
| Message | Rebuild the NUT-11 `SIG_ALL` message exactly and sign SHA-256 of it |

## Receives need nothing from Prime

Fedimint: incoming Lightning contracts are claimed with one-shot phone keys; incoming ecash is reissued
with the keys inside the token; on-chain deposits use phone-held tweak keys. All outputs go to Prime's
tree. Cashu: incoming proofs are swapped at the mint into proofs locked to Prime's current batch key.

## Prime-side code constraints

fedimint-core cannot be compiled for `armv7a-unknown-xous-elf` (see `01_RESEARCH_FINDINGS.md`). Prime
therefore uses two small standalone crates with no network dependencies:

- `fedimint-signer-core`: consensus decoding of transaction parts, module input and output wrappers,
  mint input and output, LNv2 outgoing contract, LN v1 contract output, peg-out, amounts; transaction
  hash; Schnorr via `secp256k1` (the Bitcoin app proves the C build works on the device); BLS
  verification via `bls12_381` with the tbs hash-to-curve re-implemented.
- `cashu-signer-core`: NUT-00 hash-to-curve, blinded point recomputation, P2PK secret parsing, NUT-11
  `SIG_ALL` message assembly for swap and melt, DLEQ verification, JSON via `serde_json`.

Both are validated against golden vectors produced on the host by the real upstream crates.

## Phone-side stack

Kotlin on Android first: fedimint-sdk ships an Android SDK, CDK ships Kotlin bindings, and the only
proven QuantumLink phone module is Android (Tether's Expo module, Rust UniFFI plus Kotlin BLE). iOS
follows with the fedimint-sdk Swift package, `cdk-swift` and `foundation_ble`. Flutter is possible
through CDK's Dart bindings plus ecash-app's own Fedimint bridge, but it doubles the glue.

## Alternatives considered

| Option | Pros | Cons |
|---|---|---|
| **A. Signer (chosen)** | Real security boundary; matches Foundation's model; recovery without Prime; first of its kind for either ecash system | Needs client-library forks and a KeyOS patch for Bluetooth; two codebases |
| B. Bearer-note vault on Prime | No fedimint changes; works on a stock device today; offline BLS checks possible | The phone that minted the notes saw the keys; no double-spend check offline; cash in a safe, not a wallet |
| C. Seed vault only | Trivial | Low value; recovery still needs a full rescan |
| D. Full client on Prime, phone as proxy | Prime owns everything | fedimint-client needs tokio, RocksDB and websockets; a port to a 500 MHz Cortex-A5 is months, not days |
| E. 2-of-2 MuSig note keys | Both devices required | Interactive rounds, more complexity, no gain over Prime's PIN |
| F. A plus cached notes on Prime | Prime pays face to face with no phone | State desync with the phone; the receiver still needs the federation |
