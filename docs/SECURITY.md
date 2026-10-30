# Security model

What the device protects, what it checks before signing, what it has to trust, and what it cannot
prevent. The checks are implemented in `crates/signer-core/src/analysis.rs` and
`crates/signer-core/src/release.rs`, and each one has a test in `crates/signer-core/tests/`.

## Roles

- **Passport Prime** holds the spend keys, decodes every request from raw bytes, shows what it does on
  its own screen, and signs or releases only after the user approves. It has no network.
- **The phone** runs the fedimint client: it talks to the federation, builds transactions, keeps the
  note database and history. It holds the view material, so it sees every note, but it cannot spend.
- **The federation** checks note signatures, double spends, transaction signatures and balance.

The design goal: **a compromised phone cannot spend the user's notes** in a way the user did not
approve on the device.

## What the device checks before signing

1. **Bytes.** The transaction decodes as fedimint 0.13 encodes it, re-encodes to exactly the same bytes,
   and stays within the size limits. The device signs the SHA-256 of the bytes it decoded.
2. **Every input is the device's own.** A mint v2 note must re-derive from its denomination and tweak
   to the device's key on the stated branch. An LNv2 refund or cancel must carry a contract whose
   refund key re-derives from its ephemeral point. Peg-in claims, incoming-contract claims and
   preimage claims are refused: those keys are the phone's or the gateway's.
3. **Every output is accounted for.** A mint output must re-derive, blinded nonce included, to the
   device's spend branch (change) or outgoing branch (staged for sending), and its tweak must pass the
   recovery filter. An LNv2 output must be an outgoing contract. A wallet output must be a peg-out to
   a standard script. Ecash issued to anyone else, incoming contracts, other modules and unknown
   variants are refused.
4. **The transaction matches the intent.** Paying an invoice means exactly one contract and only change
   besides; withdrawing means exactly one peg-out; preparing a send means staged notes totalling the
   stated amount; reissue, reclaim and refund produce only change. Notes already released in a token
   may only be spent by a reclaim.
5. **A Lightning payment pays the invoice.** The device parses the BOLT11 invoice and checks its
   signature, then checks it is for the federation's network, has an amount, and that the contract
   pays its payment hash, carries at least its amount, and refunds to the device. The gateway's fee is
   shown and capped by policy.
6. **Fees are what the federation charges.** Inputs must cover outputs plus the module fees computed
   from the federation's fee rules, and may overpay by at most 1,000 msat of rounding.
7. **Policy.** Per-request and daily outflow limits, the gateway fee ceiling, and a PIN above a
   threshold.

What the user sees on the device is computed from these checks, never from labels the phone supplies.

## Attacks found while building, and how they are stopped

- **Leaking the root through a token.** Spend-branch keys are derived publicly from the spend xpub, so
  one released spend key plus the xpub reveals the root. An ecash token is the note keys, and the phone
  holds the xpub. Notes are therefore moved to a hardened outgoing branch before they are sent, and
  only outgoing keys are ever released (D11; test `a_released_outgoing_key_reveals_nothing_about_the_spend_root`).
- **Burning funds through a substituted refund contract.** A refund input names only an outpoint,
  which the device cannot look up. A phone could pair it with a different, cheaper contract of ours,
  so the device would display a small refund while the federation sees the large one and keeps the
  difference as fees. The device remembers every contract it funds by outpoint and refuses a refund
  whose contract differs; unknown contracts are allowed with a warning (test
  `a_refund_is_verified_against_the_contract_the_device_funded`).
- **Burning funds through overpaid fees.** fedimint 0.13 accepts transactions whose inputs exceed
  outputs plus fees. The device computes the expected fees and refuses overpayment beyond rounding
  (test `unbalanced_and_overpaying_transactions_are_refused`).

## What the device has to trust

- **The federation config at import.** The federation id is the hash of the guardian endpoints only;
  module configs such as the mint keys are not bound to it (D12). The device checks the endpoints and
  shows the guardians and name; the rest is trusted from the paired phone. A fake mint key can only
  make the device accept a note the federation will reject.
- **The gateway** to route a Lightning payment honestly, as with every fedimint client. If it fails,
  the contract refunds to the device's key.
- **The guardians**, as every fedimint user does: they custody the funds.

## What the device cannot prevent

- **Interception of incoming funds.** Receives are signed by the phone with one-shot keys (D13). A
  compromised phone can redirect a payment that is still arriving, before it lands in the device's
  notes. Funds already in the device's notes stay safe.
- **Lies about the balance or history.** The phone keeps the note database. The device can confirm a
  note is the wallet's when it is spent, not report what the wallet holds.
- **A misleading invoice.** The device shows the invoice's amount, payee node and description. If the
  user approves an attacker's invoice, the payment goes through.
- **Released tokens held by the phone.** With `ReturnToPhone` delivery the phone holds the token until
  the receiver redeems it. `ShowOnDevice` keeps it off the phone.
- **Privacy against a compromised phone.** The view material reveals every note in the federation.

## Gateway authorisation

A Lightning payment needs a second signature from the refund key: the gateway's proof that whoever
funded the contract asked for this invoice. Prime gives it only for a `PayInvoice` transaction it
analysed, over a hash it computes itself from that invoice. The message is the SHA-256 of an encoded
invoice, so it cannot double as a transaction id that spends the contract, and the phone cannot pick
it.

## On the device

- **The review is the device's own.** The app shows what `fps-signer` computed from the request's
  bytes, and only that. A request that needs approval waits for a deliberate slide to approve or a tap on Reject.
- **The PIN.** Spends above the policy threshold, 100,000 sats by default, also need the device PIN,
  which the system's lock screen asks for. The app never sees the PIN.
- **Tokens shown on the device stay there.** A `ShowOnDevice` release answers the phone with the
  amount only; the token goes to the screen.
- **Simulator-only code.** The file bridge, the pinned test seed and the state reset exist only in
  hosted builds (`cfg(not(keyos))`). On the device the app reads its seed from KeyOS's
  `GetAppSeed`, which asks the user once, and has no other way in but the scanner. A seed pinned in
  the simulator sits in a plain file, so a simulator wallet must never hold real funds.

## Clock

Invoice expiry is only a warning, and the daily limit counts per UTC day only when the device has a
trustworthy clock. Without one the daily counter never resets, which can only make it stricter.

## Panics

KeyOS aborts the app on a panic. The signer core avoids panicking paths on untrusted input: decoders
return errors, lengths are bounded before allocating, and the remaining `expect` calls guard
invariants established earlier in the same function.
