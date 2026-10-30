# 10. NFC: future work and how to add it

NFC is out of the hackathon plan. This file records what was learned so tap-to-pay can be added later
without new research.

## The Cashu card, as the model

NUT-CCP, the Cashu Card Protocol, is a draft spec for a JavaCard that holds ecash and pays with one tap:

- The card makes a secp256k1 keypair at install and keeps the private key. Every proof it holds is locked
  to that key with `SIG_ALL`, so reading the card's memory yields nothing spendable.
- To pay, the reader builds the swap: inputs are the card's proofs, outputs are the reader's blinded
  outputs plus change outputs the card makes for itself. The card signs once and marks the slots spent in
  the same atomic write that releases the signature.
- The reader submits the swap when online. Accepting a payment while offline is the merchant's risk.
  Spent slots are cleared only after the mint confirms them spent.
- Status: draft, one reference applet on NXP JCOP4 cards, little outside review.

The card is a signer with a tiny key, not a bearer token copied over NFC. That is the role Prime already
plays in this design, with a trusted display and tap-to-approve on top.

## What Prime's NFC hardware does

- Chip: ST25R95. Reader for ISO 14443 A and B, ISO 15693 and FeliCa; card emulation for ISO 14443-A only;
  no peer-to-peer. ST lists it as not recommended for new designs, which does not matter for existing
  units. Foundation's own `st25r95` Rust driver implements both reader mode and card emulation.

## What KeyOS does with it today

- **Reader mode:** reads and writes NDEF on Type 2 tags (`nfc-forum-tags` implements Type 2 only). This is
  the Keycard path.
- **Card emulation:** `os/nfc/src/atsama5d2.rs` emulates a Type 4 tag (`emulate_t4t`), handles ISO 7816
  APDUs with chaining in 96-byte chunks, matches the FIDO AID `A0 00 00 06 47 2F 00 01`, and forwards
  CTAP2 (`NFCCTAP_MSG`, `GETRESPONSE`) and U2F to the FIDO service. This is how the Keys app lets a phone
  use Prime as a security key over NFC. The code carries a TODO: "add a RegisterApplet that fido server
  can call to register the FIDO U2F applet (giving the AID)".
- **Third-party apps:** `IsEnabled` and `IsActive` only. `ReadNdefRawMsg` and `WriteNdefRawMsg` have no
  permission group.

## The patch needed

1. An applet registry in the NFC server: an app registers an AID and receives the APDUs sent to it, the
   same way FIDO does today. The TODO in `atsama5d2.rs` describes exactly this.
2. A permission group for the registration and APDU messages so a third-party app may declare them.
3. Optionally, a permission group on the NDEF read and write messages for the gift-card mode below.

Everything else, activation, anti-collision, chaining and the session timeouts, already exists on the
shipping device.

## Phone side

The phone is always the reader, so no card-emulation entitlement is needed on either platform:

- Android: reader mode with `IsoDep`, select our AID, exchange APDUs.
- iOS: Core NFC ISO 7816 tag session with our AID listed in the app's `select-identifiers` entitlement.

## Protocol to adopt

Mirror the Cashu card commands over our existing request kinds, with Prime as the card:

| Card command | Our equivalent |
|---|---|
| Select AID, `GET_INFO` | Federation or mint id, balance summary, protocol version |
| `GET_CARD`, `GET_PIECES` | Note or proof inventory (public data only) |
| `AUTH` | Challenge signed by a Prime key, proves possession without spending |
| `SPEND_ALL_BEGIN`, `_OUTPUTS`, `_CHANGE`, `_SIGN`, `_AGAIN` | `FedimintSign` or `CashuSign` chunked into APDUs; signatures returned on `_SIGN`; last signature replayable on `_AGAIN` |
| `GET_CHANGE`, `LOAD_PROOF` | Change openings handed to the owner's phone at the next sync |
| `CLEAR_SPENT` | Only after the federation or mint confirms spent |

For Fedimint this is a genuinely new flow: the merchant's phone builds a transaction whose inputs are
Prime's notes and whose outputs are the merchant's blind nonces plus Prime's change (Prime derives its own
change blind nonces offline). Prime signs after a tap and commits the notes as spent before releasing
signatures. The payment is bound to the merchant's outputs, so sniffing the tap yields nothing, but the
federation remains the only double-spend check.

## Limits

- Hardware only: the simulator's NFC implementation is a stub.
- Short APDU chunks at NFC speed suit a handful of notes per tap, not hundreds. Use the binary `craw`
  token form for Cashu and consensus bytes for Fedimint.
- Range and antenna alignment are the usual NFC caveats.

## Cheaper side modes that need no card emulation

- **Ecash gift card:** Prime writes a token onto a plain NTAG sticker through the existing NDEF reader
  path; any phone reads it. Bearer semantics, same as a printed QR.
- **Prime to Prime:** one device emulates, the other reads. The reader side would need ISO-DEP support
  added to the reader stack, which today speaks Type 2 only.
