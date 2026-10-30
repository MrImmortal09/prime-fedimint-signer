# 09. NFC: optional stretch stage

NFC is not a dependency of anything in the plan. This file answers "how easy would it be to add", records
what the hardware and OS can do, and specifies the design so it can be added in Stage 5 or later.

## How easy is it

**Easier than Bluetooth on the OS side, harder on testing.** The verdict in one table:

| Aspect | Assessment |
|---|---|
| Card emulation on Prime | Already exists. The NFC server emulates a Type 4 tag with ISO 7816 APDUs, chaining and AID selection for FIDO. We register a second AID |
| OS patch size | Small and contained: an applet registry in one service, two or three messages, one permission group. The code carries a TODO for exactly this |
| Prime app work | An APDU handler that reassembles our existing CBOR requests and serves replies; everything behind it is reused |
| Phone work | Android reader mode with our AID, standard NFC permission. iOS reads ISO 7816 tags with the AID in the entitlement. No card-emulation restrictions apply because the phone is always the reader |
| Session timing | The real design problem. A human review takes longer than the server's session timeouts, so the flow is two taps, or hold-and-poll with the timeouts patched |
| Testing | Hardware only. The simulator's NFC implementation is a stub |
| Effort | About three working days once the QR flow works and the firmware build pipeline from Stage 4 exists; one or two more if that pipeline is not there yet |

So yes, it is easy relative to the rest of the project, with the same precondition as Bluetooth: a dev
unit running patched firmware.

## What the hardware does

- Chip: ST25R95. Reader for ISO 14443 A and B, ISO 15693 and FeliCa; card emulation for ISO 14443-A
  only; no peer-to-peer. ST lists it as not recommended for new designs, which does not matter for
  existing units. Foundation's own `st25r95` Rust driver implements both reader mode and card emulation.

## What KeyOS does with it today

- **Reader mode:** reads and writes NDEF on Type 2 tags (`nfc-forum-tags` implements Type 2 only). This is
  the Keycard path.
- **Card emulation:** `os/nfc/src/atsama5d2.rs` emulates a Type 4 tag (`emulate_t4t`), handles ISO 7816
  APDUs with chaining in 96-byte chunks, matches the FIDO AID `A0 00 00 06 47 2F 00 01`, and forwards
  CTAP2 (`NFCCTAP_MSG`, `NFCCTAP_GETRESPONSE`) and U2F to the FIDO service. This is how the Keys app lets
  a phone use Prime as a security key over NFC. The code carries a TODO: "add a RegisterApplet that fido
  server can call to register the FIDO U2F applet (giving the AID)".
- **Timeouts in that file:** `EMULATE_T4T_TIMEOUT` 10 s for activation plus transaction,
  `APDU_TRANSACTION_TIMEOUT` 5 s of inactivity, and cooldowns after successes and errors.
- **Third-party apps:** `IsEnabled` and `IsActive` only. `ReadNdefRawMsg` and `WriteNdefRawMsg` have no
  permission group.

## The patch

1. `api/nfc/manifest.toml`: `RegisterApplet { aid }`, `ApduReceived` (server to app, with session id) and
   `ApduReply` (app to server), all with a permission group such as `device-connectivity.nfc-applet`.
2. `os/nfc/src/atsama5d2.rs` `handle_apdu_cmd`: on SELECT, match registered AIDs next to the FIDO AID and
   forward subsequent APDUs of that session to the registered app; reuse the existing chaining code for
   replies.
3. Timeouts: make the inactivity timeout per applet, or long enough for hold-and-poll; keep FIDO's
   values for FIDO.
4. App manifest: declare the three messages in `permission_templates.toml`.

Nothing else changes: activation, anti-collision, chaining, cooldowns and the Keycard reader path stay as
they are. Dispatch is by AID, so FIDO and Keycards are unaffected.

## The two-tap design

Prime's review needs a human, and a phone held against the device for the whole review is awkward, so
the default flow is two taps. Hold-and-poll works as an optimisation when the user is quick.

| Step | Phone (reader) | Prime (card) |
|---|---|---|
| Tap 1 | SELECT our AID; `PUT_REQUEST` with the CBOR `Sign` or `Release` request in chained APDUs | Reassembles, validates, stores as pending, replies `ACCEPTED { request_id }`, ends the session |
| Review | Shows "approve on your Prime" | Shows the review screen; the user approves or rejects; the result is stored against the request id |
| Tap 2 | SELECT our AID; `GET_RESULT { request_id }` | Replies `Signatures`, `Token`, `Reject` or `PENDING`, in chained APDUs; a result is replayable until acknowledged |
| Ack | `ACK { request_id }` | Clears the stored result |

Request ids, one-review-at-a-time, cancel and reject semantics are the same as on every other transport.
A request of a dozen inputs is around one to two kilobytes, which is a second or two of APDU exchange.

## Tap-to-pay at a merchant (the Fedimint twist)

The same applet supports paying someone else's phone with no wallet phone of your own present, which is
option F from `02_ARCHITECTURE.md` done safely:

1. The merchant's phone reads Prime's note inventory (public data: amounts, nonces, mint signatures).
2. It builds a transaction whose inputs are Prime's notes and whose outputs are the merchant's blind
   nonces plus Prime's change. Prime derives its own change blind nonces offline, since it holds the view
   secret too, and keeps the openings.
3. Prime reviews and signs after a tap, and marks the notes spent before releasing signatures.
4. The merchant submits when online. The payment is bound to the merchant's outputs, so sniffing the tap
   yields nothing. The federation remains the only double-spend check, so accepting offline is the
   merchant's risk, exactly as with the Cashu card.
5. Prime hands the change openings to the owner's phone at the next sync; spent notes are purged only
   after the federation confirms them.

This is modelled on the Cashu Card Protocol (NUT-CCP): info, read, spend-begin, outputs, change, sign,
sign-again. Prime adds a screen and a tap to approve, which no card can offer.

## Limits

- Hardware only; the simulator's NFC implementation is a stub.
- Short APDU chunks at NFC speed suit a handful of notes per tap, not hundreds. Use consensus bytes, not
  text encodings, over NFC.
- Range and antenna alignment are the usual NFC caveats.

## Cheaper side modes that need no card emulation

- **Ecash gift card:** Prime writes a token onto a plain NTAG sticker through the existing NDEF reader
  path; any phone reads it. Bearer semantics, same as a printed QR. Needs only a permission group on
  `WriteNdefRawMsg`.
- **Prime to Prime:** one device emulates, the other reads. The reader side would need ISO-DEP support
  added to the reader stack, which today speaks Type 2 only. More work than the applet registry.
