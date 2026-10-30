# 08. Verification and testing

## Oracles

| Oracle | What it proves | How to run |
|---|---|---|
| devimint | A real federation with gateway and bitcoind on regtest | fedimint repo nix shell; `scripts/run-sdk-examples.sh v2 shell` in fedimint-sdk gives a federation plus counterparty commands |
| fedimint-sdk examples | The reference behaviour of every Fedimint flow | `cargo run --example lightning -- ./wallet <invite> receive 100000` and the `ecash`, `onchain`, `walkthrough` examples |
| cdk-mintd + cdk-fake-wallet | A real mint whose quotes fill automatically | `cdk-mintd` with the fake backend; `cdk-cli` for reference flows |
| Public nutshell test mint | Cross-implementation `SIG_ALL` behaviour | Check mint info for `nut11` first |
| Public signet or mutinynet federation | Compatibility with federations we did not configure | Invite code |

## Golden vectors (Stage 1)

Generated on the host by a small program in `ecash-devtools` that uses the real crates, stored as JSON
fixtures, consumed by both signer cores:

- Fedimint: consensus-encoded unsigned transactions covering mint-only reissue, LNv2 payment with change,
  LN v1 payment, peg-out, and refund; expected transaction hashes; note nonces and mint signatures with
  per-tier keys from a devimint config; expected decode results.
- Cashu: NUT-00 hash-to-curve vectors (from the NUT test vectors), P2PK secrets in canonical form, swap
  and melt `SIG_ALL` messages with expected SHA-256 digests and signatures, DLEQ vectors from NUT-12.

Rule: the device-side result must equal the upstream result byte for byte. A mismatch is a release blocker.

## Decoder robustness

- Property tests: encode with fedimint-core, decode with `fedimint-signer-core`, compare.
- Fuzz both decoders with `cargo fuzz` on the host; the device panics abort, so any panic path is a bug.
- Length and count limits on everything decoded (inputs, outputs, note counts, JSON size).

## Negative tests on Prime (Stage 6, but write them as you go)

| Case | Expected |
|---|---|
| Input nonce not derivable from my tree | Refuse with `ForeignInput` |
| Note with an invalid BLS signature | Refuse with `InvalidNote` |
| Change path beyond the gap limit | Refuse |
| Output of an unknown module | Refuse |
| Two external outputs | Refuse |
| LNv2 contract hash not equal to invoice hash, or amount off | Refuse with `ContextMismatch` |
| Refund key outside the refund branch | Refuse |
| Cashu output opening that does not reproduce `B_` | Refuse |
| Cashu input locked to a different key, or `SIG_INPUTS` | Refuse |
| Melt quote id in the request differs from the message | Refuse |
| Replayed request id | Ignore, log |
| Second request while a review is open | Reject with `Busy` |
| Cancel during review | Clear state, no signature released |
| Policy limit exceeded | Refuse, show the limit |

## Phone-side checks

- Verify every returned Fedimint signature against the note nonces before assembling the transaction.
- Verify the Cashu `SIG_ALL` signature against the lock key before submitting.
- Check proof state (NUT-07) before a Cashu spend; await the Fedimint transaction outcome before marking a
  spend done.
- Never log tokens: `Notes` in fedimint-sdk redacts `Debug` for this reason; do the same for Cashu tokens.

## Performance

- Time BLS pairing on hardware with the `gui-app-crypto-perf` pattern; set the maximum notes per request
  from the result. Expect a handful of verifications per second on the Cortex-A5.
- Keep requests under the size a single animated UR session carries comfortably; the file bridge and
  QuantumLink have no practical limit at our sizes.

## Progression

1. Simulator with the file bridge: all flows, all negative tests, CI.
2. Hardware over QR/UR: the same flows with real `GetAppSeed`, PIN gating and camera.
3. Hardware over QuantumLink on patched firmware: pairing, reconnect, cancel, disconnect mid-review.

## Test matrix

| Flow | Fedimint | Cashu | File bridge | QR/UR | QuantumLink |
|---|---|---|---|---|---|
| Pair and export view material | x | x | x | x | x |
| Import config / mint keys | x | x | x | x | x |
| Receive Lightning | x | x | phone only | phone only | phone only |
| Receive ecash token | x | x | phone only | phone only | phone only |
| Pay Lightning | x | x (melt) | x | x | x |
| Withdraw on-chain | x | n/a | x | x | x |
| Send ecash | x (release) | x (swap) | x | x | x |
| Reclaim unredeemed | x | n/a | x | x | x |
| Refund failed payment | x | mint returns proofs | x | x | x |
| Recovery on a new phone | x | x (NUT-09) | x | x | x |
| Every negative case | x | x | x | spot check | spot check |
