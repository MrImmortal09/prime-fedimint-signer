# Wire protocol, version 1

The messages between the phone wallet and Passport Prime. The reference implementation is
`crates/protocol`. The same bytes travel over every transport: the simulator file bridge, the simulator
network link, animated UR (types `fps-request` and `fps-response`), QuantumLink and NFC. [Transports](#transports) describes
the first two.

## Envelope

Every message is one CBOR map:

| Key | Type | Meaning |
|---|---|---|
| `v` | unsigned | Protocol version, 1 |
| `id` | bytes(16) | Request id chosen by the phone; a response repeats it |
| `body` | Request or Response | The message |

Rules for both sides:

- At most 64 KiB per message, checked before parsing.
- Unknown fields and unknown variants are errors. Byte strings declared with a length must have it.
- Trailing bytes after the envelope are an error.
- A message with another version is answered with `Rejected(UnsupportedVersion)` under its id.
- A body that does not parse is answered with `Rejected(Malformed)` under its id when the header was
  readable, and under the all-zero id otherwise.
- fedimint data (`transaction`, `api_endpoints`, `contract`) is always the exact fedimint consensus
  encoding, so the device decodes the same bytes the federation sees.

Enums use serde's external tagging: a unit variant is a text string such as `"GetInfo"`; a variant
with data is a one-entry map from the variant name to its fields.

## Requests

| Request | Approval | Answer |
|---|---|---|
| `GetInfo` | never | `Info` |
| `ImportFederation(FederationDescriptor)` | yes, unless the identical descriptor is already stored | `FederationImported` |
| `ExportViewMaterial { federation }` | yes, every time; never replayed | `ViewMaterial` |
| `DeriveOutgoingNonces { federation, notes }` | never; at most 64 notes | `OutgoingNonces` |
| `SignTransaction(SignRequest)` | yes | `Signed` |
| `ReleaseNotes(ReleaseRequest)` | yes, unless every note was staged by an approved `PrepareSend`, the policy allows it, and no PIN is needed | `Released` |
| `GetResult { request }` | never | the stored response of `request`, `Pending`, or `Rejected(NotFound)` |
| `GetStatus` | never | `Status` |
| `Cancel { request }` | never | `Cancelled`, stored as the cancelled request's answer |
| `Cashu(CashuRequest)` | see [Cashu](#cashu) | `Cashu(CashuResponse)` |

Only one request waits for approval at a time; another one that needs approval gets
`Rejected(Busy)`. Final answers (signatures, releases, imports, user rejections, cancellations) are
stored with their request id. Sending the same id again returns the stored bytes, so a lost reply can
be fetched again without a second signature or a second release. The device keeps the last 32.

### FederationDescriptor

What the phone extracts from the federation's client config.

| Field | Meaning |
|---|---|
| `federation` | Federation id, 32 bytes |
| `api_endpoints` | Consensus encoding of `GlobalClientConfig::api_endpoints`. The device checks its SHA-256 equals `federation` |
| `name` | The `federation_name` meta field, if any |
| `modules` | Every module instance: `instance`, `kind`, `major`, `minor` |
| `mint` | `mintv2` config: `instance`, aggregate key per denomination (`keys`), `fee_base_msat`, `fee_ppm`, `amount_unit` (must be 0) |
| `lightning` | `lnv2` config: `instance`, `network`, `fee_base_msat`, `fee_ppm` |
| `wallet` | `walletv2` config: `instance`, `network`, `dust_limit_sat`, `fee_base_msat`, `fee_ppm` |

Only the endpoints are authenticated (decision D12).

### SignRequest

| Field | Meaning |
|---|---|
| `federation` | Federation id |
| `transaction` | `encode(inputs) || encode(outputs) || nonce`: exactly what fedimint hashes into the transaction id |
| `inputs` | One hint per input: `Note { tweak, branch }` for a mint v2 note, `Refund { contract }` for an LNv2 refund or cancel, carrying the consensus-encoded `OutgoingContract` |
| `intent` | `PayInvoice { invoice }`, `Withdraw`, `PrepareSend { amount_msat }`, `Reissue`, `Reclaim` or `Refund` |

The device re-encodes the decoded transaction and refuses any difference, then checks it as described
in `SECURITY.md`. The answer is `Signed { txid, signatures }`: one BIP-340 signature per input, in input
order, ready for fedimint's `TransactionSignature::NaiveMultisig`.

### ReleaseRequest

| Field | Meaning |
|---|---|
| `federation` | Federation id |
| `notes` | `{ denomination, tweak, signature }` per outgoing-branch note, `signature` being the unblinded mint signature |
| `include_invite` | Add an invite to the token. The device takes the guardian URL from its own endpoints |
| `delivery` | `ShowOnDevice` (the token is shown as a QR on the device only) or `ReturnToPhone` |

The answer is `Released { amount_msat, notes, token }`, with `token` set only for `ReturnToPhone`. The
token is a fedimint mint v2 `ECash` string (`fedimint` + base32). The device records the release
durably before the answer leaves it.

## Responses

| Response | Contents |
|---|---|
| `Info` | `protocol`, `key_scheme` (`fps-v1`), imported federations, `busy` |
| `FederationImported` | `federation`, `name`, guardian count, module kinds |
| `ViewMaterial` | `federation`, `key_scheme`, `spend_key` (33), `chain_code` (32), `view_secret` (32) |
| `OutgoingNonces` | One 33-byte nonce per requested note |
| `Signed` | `txid`, signatures, and for a Lightning payment `gateway_auth` |
| `Released` | `amount_msat`, `notes`, `token` |
| `Status` | Released notes, recent request ids, today's outflow, the waiting request |
| `Pending` | The request is waiting for the user |
| `Cancelled` | |
| `Rejected` | `code` and `message` |
| `Cashu` | The answer to a `Cashu` request; nothing else gets one |

### Gateway authorisation

An LNv2 gateway pays only if the request carries the contract refund key's BIP-340 signature over
the SHA-256 of fedimint's encoding of `LightningInvoice::Bolt11(invoice)`. Prime holds the refund
key, so when it signs a `PayInvoice` transaction it adds that signature as
`SignedTransaction::gateway_auth`. It signs only the hash of the invoice it checked against the
contract, never a hash the phone supplies.

### Rejection codes

`Malformed`, `UnsupportedVersion`, `UnknownFederation`, `FederationIdMismatch`, `InvalidDescriptor`,
`InvalidTransaction`, `Unsupported`, `ForeignInput`, `InvalidNote`, `UnexpectedOutput`,
`IntentMismatch`, `InvoiceMismatch`, `AlreadyReleased`, `PolicyLimit`, `UserRejected`, `Busy`,
`NotFound`, `TooLarge`, `Internal`. Codes are stable; messages are for people and may change.

## Cashu

The second backend. Its messages travel in the same envelopes, version 1, as the last variants of
the two enums: `Request::Cashu(CashuRequest)` and `Response::Cashu(CashuResponse)`, with the bodies
in `crates/protocol/src/cashu.rs`. Enums are tagged by name, so every Fedimint message keeps its
bytes. Prime checks, shows and signs Cashu spends as it does Fedimint ones: the phone builds every
operation and talks to the mint. The design is [`plan/12_CASHU.md`](plan/12_CASHU.md), the key
scheme `fps-cashu-v1` ([`KEY_SCHEME_CASHU.md`](KEY_SCHEME_CASHU.md)); `crates/cashu-core`
implements the checks and `crates/cashu-signer` the service.

```
{"v": 1, "id": h'…', "body": {"Cashu": "Info"}}
{"v": 1, "id": h'…', "body": {"Cashu": {"Sign": {"mint": h'…', "epoch": 0, "inputs": […], …}}}}
{"v": 1, "id": h'…', "body": {"Cashu": {"Signed": {"message_hash": h'…', "signature": h'…'}}}}
```

| Request | Approval | Answer |
|---|---|---|
| `Info` | never | `Info(CashuInfo)`: the scheme name and the imported mints |
| `AddMint(CashuMintDescriptor)` | yes, every time: "Cashu: add mint", or "Cashu: update mint keys" for a known mint | `MintAdded(CashuLockMaterial)`, never stored for replay |
| `Sign(CashuSignRequest)` | yes: "Cashu: send ecash", "Cashu: combine ecash", "Cashu: move to a new key" or "Cashu: pay Lightning invoice" | `Signed(CashuSigned)` |

How Prime serves them beside Fedimint:

- **One review at a time, across both backends.** `AddMint` and `Sign` get `Rejected(Busy)` while
  any review waits, Fedimint's or Cashu's, unless their id has a stored answer; `Info` is always
  answered. While a Cashu review waits, `ImportFederation`, `ExportViewMaterial`,
  `SignTransaction` and `ReleaseNotes` get exactly what they get while a Fedimint review waits:
  their stored answer, or `Busy` naming the waiting request. That holds for a release of prepared
  notes too, which an idle Prime would release without asking.
- **Stored answers.** Cashu keeps its own ring of the last 32 final answers (`Signed`, and the
  `UserRejected` and `Cancelled` refusals) in its own file. A Cashu request sent again under the
  same id gets its stored answer, never a second signature. `MintAdded` carries the view key and
  is never stored: the same `AddMint` again asks again, as `ExportViewMaterial` does.
- **`GetResult` and `Cancel`** look in the Fedimint state first and, only if it has nothing for
  that request, in Cashu's. So they work on Cashu ids too: the stored answer, `Pending` while the
  request waits, and `Cancel` abandons a waiting Cashu request and stores `Cancelled` as its
  answer. If neither backend has the request, the answer is the usual `Rejected(NotFound)`.
- **`GetInfo` and `GetStatus` describe Fedimint only.** While a Cashu review waits, `busy` stays
  false and `pending` empty; Cashu's `Info` lists the mints.
- **Policy.** The PIN threshold, the per-request limit and the daily limit apply to a Cashu spend
  as to a Fedimint one, over its outflow: the leaving outputs plus the mint's fee for a send, the
  fee alone for a combine or a move to a new key, and every input of a melt, since Prime cannot
  see what the quote pays. The daily count a Cashu spend is held to is Cashu's own plus Fedimint's,
  so the two together stay under the limit. A melt's fee reserve is held to the routing fee limit
  that bounds a Lightning gateway's fee.
- **A Prime without Cashu.** A Prime app older than the Cashu backend cannot decode
  `Request::Cashu` and answers `Rejected(Malformed)` under the request id ("unknown variant
  `Cashu`"), which the phone reports as "this Prime app has no Cashu". A Prime app built with this
  protocol but without the Cashu service answers `Rejected(Unsupported)`. A Prime whose Cashu state
  cannot be read answers `Info` with no mints and `AddMint` and `Sign` with `Rejected(Internal)`,
  saying why; Fedimint works as before.

An `AddMint` for a known mint only adds keysets and updates their `active` flags. A keyset already
pinned must come with the same keys and fee, one the descriptor no longer lists stays pinned as
inactive, and no key may be listed under two amounts, across all of them (at most 16): a melt counts
each input at the amount its DLEQ proof's key is pinned for. Anything else is `InvalidDescriptor`.

| Type | Fields |
|---|---|
| `CashuMintDescriptor` | `url` (canonical, as CDK's `MintUrl` renders it), `unit` (`sat`), `keysets` |
| `CashuKeyset` | `id` (8 bytes for version 00, 33 for version 01), `unit`, `input_fee_ppk`, `final_expiry`, `active`, `keys` |
| `CashuKey` | `amount`, `pubkey` (33) |
| `CashuSignRequest` | `mint` (`MintId`, the SHA-256 of the URL), `epoch` (of the inputs' lock key), `inputs`, `outputs`, `kind` |
| `CashuInput` | `amount`, `keyset`, `secret` (verbatim), `c` (33), `dleq` (required on every input of a melt) |
| `CashuDleq` | `e`, `s`, `r` (32 each), NUT-12 |
| `CashuOutput` | `amount` (0 for a melt's blanks), `keyset`, `b` (33), `opening` |
| `CashuOpening` | `secret`, `r` (32), `hint`: present for change, which Prime re-derives; absent for an output leaving the wallet |
| `CashuOutputHint` | `epoch`, `keyset`, `counter`: the change output's `fps-cashu-v1` position |
| `CashuSpendKind` | `Swap`, or `Melt { quote, invoice, amount, fee_reserve }` (sats) |
| `CashuInfo` | `scheme` (`fps-cashu-v1`), `mints` (`CashuMintSummary`: `mint`, `url`, `unit`, `keysets`) |
| `CashuLockMaterial` | `mint`, `scheme`, `lock_pubkey` (33), `view_key` (32) |
| `CashuSigned` | `message_hash` (32), `signature` (64): BIP-340 over the NUT-11 `SIG_ALL` message Prime built, for the first input's witness |

The same strictness applies: unknown fields and variants are errors, and byte strings declared
with a length must have it. Cashu answers use only the rejection codes above:

| Code | For a Cashu request |
|---|---|
| `Malformed` | The message does not decode; from a Prime app older than Cashu, every Cashu request |
| `Unsupported` | A unit other than `sat`; a Prime app without the Cashu service |
| `UnknownFederation` | The mint is not added on the device |
| `InvalidDescriptor` | A mint URL not in canonical form, a keyset id not derived from its keys, an update that changes a pinned keyset, a key under two amounts |
| `InvalidTransaction` | No inputs, a swap that does not balance, inputs that do not cover a melt, a quote id Prime refuses, a repeated input or blinded message, outputs whose message reads two ways |
| `ForeignInput` | An input not locked to the wallet's key for the request's epoch |
| `InvalidNote` | An input whose keyset is not pinned or has no key for its amount, whose `c` is not a point, or whose DLEQ proof fails or, on a melt, is missing |
| `UnexpectedOutput` | An output that does not open, in a keyset not pinned or not active, with a wrong change hint, or reusing a secret |
| `InvoiceMismatch` | A melt invoice that does not parse, has no amount, or does not match the quote |
| `PolicyLimit` | Over the per-request or daily limit, a fee reserve over the routing fee limit, too few blanks to return a melt's change |
| `TooLarge` | More than 64 inputs or outputs; a new mint beyond the 16 Prime holds, or a mint or update that would take it past 2,048 pinned keys over all its mints |
| `Busy` | Another review waits, on either backend |
| `UserRejected` | Rejected on the device |
| `NotFound` | `GetResult` or `Cancel` for a request neither backend has |
| `Internal` | The Cashu state cannot be read, so Cashu is off; a write failed; a key derivation failed |

## Transports

`crates/transport` (`fps-transport`) implements both ends of each transport, so the device app and
the programs that talk to it share the framing.

### Animated QR

- A message is one Uniform Resource (BCR-2020-005) of type `fps-request` (phone to device) or
  `fps-response` (device to phone). Its payload is the envelope itself.
- The sender shows the pure fountain parts in a loop. Each part is at most 300 characters and upper
  case, so the codes use the QR alphanumeric mode.
- The receiver collects parts in any order and case. It checks the message length each part claims
  against the 64 KiB limit before it reassembles anything.
- On Passport Prime the system scanner reassembles requests. A `[[qr-match-rules]]` entry routes
  `fps-request` codes scanned from the Launcher to the app.
- A token released with `ShowOnDevice` is shown as one plain QR code of the `fedimint…` string, which
  any fedimint wallet can scan.

### Simulator file bridge

For development and tests, the app in the hosted KeyOS simulator also serves a host directory,
`$FPS_SIM_DIR` or `~/.fps-signer-sim`:

| Path | Written by | Contents |
|---|---|---|
| `inbox/*.cbor` | requester | One request envelope per file, taken in name order |
| `outbox/<request id in hex>.cbor` | app | The response envelope; an unreadable request is answered as `000…0.cbor` |
| `app-seed.hex` | test | Pins the app seed, so scenarios built for a known seed replay. Without it the app reads its seed from KeyOS |
| `reset` | test | Present when the app starts: the app forgets its stored state |

Both sides write under a temporary name and rename the file into place. A request that needs approval
holds the queue until the user decides on the simulator's screen. Device builds have no bridge.

### Simulator network link

A stand-in for QuantumLink while there is no device. The app in the hosted simulator listens on TCP
(`$FPS_NET_ADDR`, default `0.0.0.0:7865`), and a phone on the same network connects to it, one
exchange per connection:

| Direction | Frame |
|---|---|
| phone to app | `u32` big-endian length, then the request envelope |
| app to phone | `u32` big-endian length, then the response envelope, once it is final |

A request that needs approval keeps its connection open until the user decides on the simulator's
screen; requests that arrive meanwhile wait in order. A frame whose length is over the 64 KiB limit is
refused before it is read. There is no encryption and no authentication: the link is for a network you
control, and device builds do not compile it. On the device the same envelopes will travel over
QuantumLink.
