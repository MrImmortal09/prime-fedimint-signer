# 03. Transaction flows

Every flow assumes the design in `02_ARCHITECTURE.md`. "Prime" means the KeyOS app, "phone" the
companion wallet app.

## 1. One-time setup

1. **Pair.** The phone scans Prime's static Bluetooth QR, then its animated identity QR. Both sides pin
   each other's QuantumLink identity. On the QR-only transport the same identity exchange runs as two UR
   scans.
2. **Export view material.** Prime derives the spend root from its per-app seed and sends the phone the
   xpub and the view secret. The phone can now derive every note nonce and blinding key. It cannot
   derive any spend key.
3. **Import the federation.** The phone joins with an invite code and forwards the client config to
   Prime. Prime checks the config hash against the federation id, then stores the name, the mint's
   per-tier public keys, the module instance map and the fee schedule. From here Prime can judge a
   transaction with no help from the phone.

## 2. Receiving (phone only)

- **Lightning:** the phone creates the incoming contract, the gateway funds it, the phone claims it with a
  one-shot key into new notes. For each note it takes the next unused path under the xpub, builds the
  blind nonce from the view secret, sends it to the federation, unblinds the returned signature, and
  stores a watch-only note: nonce, mint signature, amount tier, path.
- **Ecash token:** the phone parses the token, checks the federation prefix matches a joined federation,
  verifies every note's mint signature offline against the stored per-tier keys, and shows the amount as
  "unverified". It then submits a reissue: inputs are the received notes signed with the keys inside the
  token, outputs are fresh notes on Prime's tree. The federation accepts it only if no note was spent
  before. That acceptance is the double-spend check, and the only one that exists in ecash.
- **On-chain:** the phone derives a deposit address with a phone-held tweak key and claims the deposit
  into Prime-tree notes once confirmed.

## 3. Paying a Lightning invoice

**Phone, build.**

1. The user pastes or scans a BOLT11 invoice. The phone asks a gateway for a quote and gets the fee and
   the gateway's claim key.
2. The transaction builder selects watch-only notes covering the invoice amount, the gateway fee and the
   mint input fees. Each input is a mint input with the note's amount and public note data, plus a remote
   key reference instead of a keypair.
3. It adds one LNv2 outgoing contract output: payment hash, amount, expiry, the gateway's claim key, a
   refund key from Prime's refund branch, and an ephemeral key for the preimage handoff.
4. It adds mint change outputs at the next unused paths and draws a random transaction nonce.
5. It consensus-encodes the unsigned transaction and sends Prime one `Sign` request: request id,
   federation id, raw bytes, the key path of every input, the path hint of every change output, and the
   invoice as context.

**Prime, decode, verify, show, sign.** Prime decodes the raw bytes with its own decoder and refuses the
request if any check in `02_ARCHITECTURE.md` fails. It shows the federation name, the invoice
destination and description, the amount, the fee as inputs minus outputs, and the change. After a tap,
and a PIN prompt above a configurable threshold, it computes the transaction hash from the bytes it
decoded, Schnorr-signs it once per input key in input order, and returns the signature list with the
request id. Spend keys exist only for the duration of the call.

**Phone, verify, submit, settle.**

1. The phone verifies every returned signature against the note nonces before anything else.
2. It assembles the final transaction and submits it. Each guardian checks the BLS note signatures, that
   no nonce was spent before, the Schnorr signatures, and that amounts balance, then consensus includes it.
3. The LNv2 state machine hands the invoice and contract to the gateway. The gateway pays over Lightning,
   learns the preimage, and claims the contract. The phone shows the payment as settled.
4. The phone fetches the blind signature shares for its change outputs, unblinds them with the view
   secret, and stores the new watch-only notes. Prime is not involved again.

## 4. On-chain withdrawal

Same as section 3 with a peg-out output instead of the contract. Prime shows the destination address,
the amount and the on-chain fee read from the output, and checks there is exactly one external output.

## 5. Sending ecash

An ecash token is literally the secret keys of the notes plus their mint signatures. Sending means
handing those keys over. Prime's job is therefore to **release** keys for an approved amount.

1. **Pick the amount.** The phone checks whether its notes can make the amount exactly in the mint's
   denominations. If yes, go to step 3.
2. **Split first if needed.** The phone builds a reissue: inputs are some notes, outputs are fresh notes
   in exactly the needed denominations on the `outgoing` branch, plus change on the normal branch. Prime
   reviews it as a self-transfer, since every output re-derives to its own tree, and signs. The phone
   submits, waits for consensus, and unblinds the new signatures. Nothing has left the wallet yet.
3. **Approve the release.** The phone sends `Release` with the note paths and mint signatures. Prime shows
   "send N sats as ecash" with the federation name, and after a tap derives those spend keys and marks the
   paths released, so it never signs with them again. One tap can cover steps 2 and 3: Prime remembers
   the pre-approved paths and emits the token when the phone delivers the signatures.
4. **Deliver the token**, two options:
   - Prime shows the QR itself. The receiver scans Prime's screen. The phone never sees the keys.
   - Prime hands the token to the phone, which sends it over chat, Nostr or any channel. The phone now
     holds exactly the approved amount and nothing more.
5. **The receiver reissues** with the federation. That is when the notes leave your control for good.
6. **Reclaim if unredeemed.** If the phone holds the token it can reclaim alone, because the inputs are
   signed with the released keys and the outputs go back to Prime's tree. If Prime displayed the QR, the
   phone asks Prime to sign the reclaim, which Prime treats as a self-transfer.

Token size is not a problem: a handful of notes fits a single QR on Prime's screen; larger tokens go out
as animated UR.

## 6. Failure cases

- **Prime rejects or the user cancels.** Nothing happened anywhere. The phone discards the draft and can
  reuse the change paths.
- **Bluetooth drops after signing.** The phone asks again with a new request id. A signed transaction
  that was never submitted spends nothing, because the federation never saw it.
- **Submit fails.** The notes are still unspent. The phone awaits the transaction outcome before retrying.
- **Gateway cannot pay.** The contract expires and the phone builds a refund, a spend from the refund key
  that Prime reviews as "refund of failed payment back to your notes".
- **Token nobody redeems.** Comes back through the reclaim path (section 5, step 6).
- **Lost phone.** Nothing can be spent. A new phone pairs, receives the view material again, and rebuilds
  the note set by rescanning the federation.
- **Lost Prime.** Restore the seed on a new Prime; the same app id reproduces the same tree.

## 7. Optional modes, both with the same caveat

- **Prime receives offline.** Prime scans a token with its camera, verifies the mint signatures, and
  holds it as a bearer note until the phone connects and reissues. Until then the sender can still
  double-spend it. Fine only between parties who trust each other for a short window.
- **Prime to Prime.** Two devices exchange ecash by QR with no phones present and settle later. Same
  caveat, memorable demo.
