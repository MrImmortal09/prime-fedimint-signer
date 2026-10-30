# Wire protocol, version 1

The messages between the phone wallet and Passport Prime. The reference implementation is
`crates/protocol`. The same bytes travel over every transport: the simulator file bridge, animated UR
(types `fps-request` and `fps-response`), QuantumLink and NFC. [Transports](#transports) describes
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
| `Signed` | `txid`, signatures |
| `Released` | `amount_msat`, `notes`, `token` |
| `Status` | Released notes, recent request ids, today's outflow, the waiting request |
| `Pending` | The request is waiting for the user |
| `Cancelled` | |
| `Rejected` | `code` and `message` |

### Rejection codes

`Malformed`, `UnsupportedVersion`, `UnknownFederation`, `FederationIdMismatch`, `InvalidDescriptor`,
`InvalidTransaction`, `Unsupported`, `ForeignInput`, `InvalidNote`, `UnexpectedOutput`,
`IntentMismatch`, `InvoiceMismatch`, `AlreadyReleased`, `PolicyLimit`, `UserRejected`, `Busy`,
`NotFound`, `TooLarge`, `Internal`. Codes are stable; messages are for people and may change.

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
