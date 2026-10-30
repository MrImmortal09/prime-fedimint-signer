# 07. Verification and testing

## Oracles

| Oracle | What it proves | How to run |
|---|---|---|
| devimint | A real federation with gateway and bitcoind on regtest | fedimint repo nix shell; from fedimint-sdk, `scripts/run-sdk-examples.sh v2 shell` gives a federation plus counterparty commands |
| fedimint-sdk examples | The reference behaviour of every flow | `cargo run --example lightning -- ./wallet <invite> receive 100000`, plus `ecash`, `onchain`, `walkthrough` |
| Public signet or mutinynet federation | Compatibility with a federation we did not configure | Invite code; check its LN module first |

## Golden vectors (Stage 1)

Generated on the host by a small program in `fedimint-signer-devtools` that uses the real crates, stored
as JSON fixtures, consumed by `fedimint-signer-core`:

- Consensus-encoded unsigned transactions covering: mint-only reissue, LNv2 payment with change, LN v1
  payment, peg-out, refund, split for release, reclaim.
- Expected transaction hashes for each.
- Note nonces and mint signatures with per-tier keys from a devimint config, plus deliberately broken
  signatures.
- Blind nonces computed from known view-secret paths, for the change check.
- Expected decode results as structured JSON.

Rule: the device-side result must equal the upstream result byte for byte. A mismatch blocks a release.

## Decoder robustness

- Property tests: encode with fedimint-core, decode with `fedimint-signer-core`, compare.
- Fuzz the decoder with `cargo fuzz` on the host; the device aborts on panic, so any panic path is a bug.
- Length and count limits on everything decoded: inputs, outputs, note counts, request size.

## Negative tests on Prime (Stage 6, but write them as you go)

| Case | Expected |
|---|---|
| Input nonce not derivable from my tree | Refuse with `ForeignInput` |
| Note with an invalid BLS signature | Refuse with `InvalidNote` |
| Change path beyond the gap limit | Refuse |
| Output of an unknown module | Refuse |
| Two external outputs | Refuse |
| LNv2 contract hash not equal to the invoice hash, or amount off | Refuse with `ContextMismatch` |
| Refund key outside the refund branch | Refuse |
| Release request for a path already released | Refuse |
| Sign request using a released path as input (not a reclaim) | Refuse |
| Replayed request id | Ignore, log |
| Second request while a review is open | Reject with `Busy` |
| Cancel during review | Clear state, no signature released |
| Policy limit exceeded | Refuse, show the limit |
| Federation config whose hash does not match the federation id | Refuse at import |

## Phone-side checks

- Verify every returned signature against the note nonces before assembling the transaction.
- Await the transaction outcome before marking a spend done; retry submission only after a failure is
  confirmed.
- Never log tokens: `Notes` in fedimint-sdk redacts `Debug` for this reason.
- Reconcile the released-path log from Prime's `Status` reply on every connect.

## Performance

- Time BLS pairing on hardware with the `gui-app-crypto-perf` pattern; set the maximum notes per request
  from the result. Expect a handful of verifications per second on the Cortex-A5.
- Keep requests under the size a single animated UR session carries comfortably; the file bridge and
  QuantumLink have no practical limit at our sizes; NFC prefers a few hundred bytes per tap.

## Progression

1. Simulator with the file bridge: all flows, all negative tests, CI.
2. Hardware over QR/UR: the same flows with real `GetAppSeed`, PIN gating and camera.
3. Hardware over QuantumLink on patched firmware: pairing, reconnect, cancel, disconnect mid-review.
4. Optional: NFC two-tap on the same firmware.

## Test matrix

| Flow | File bridge | QR/UR | QuantumLink | NFC (optional) |
|---|---|---|---|---|
| Pair and export view material | x | x | x | x |
| Import federation config | x | x | x | x |
| Receive Lightning | phone only | phone only | phone only | phone only |
| Receive ecash token | phone only | phone only | phone only | phone only |
| Receive on-chain | phone only | phone only | phone only | phone only |
| Pay Lightning | x | x | x | x |
| Withdraw on-chain | x | x | x | x |
| Send ecash (split + release) | x | x | x | x |
| Reclaim unredeemed | x | x | x | spot check |
| Refund failed payment | x | x | x | spot check |
| Recovery on a new phone | x | x | x | n/a |
| Every negative case | x | spot check | spot check | spot check |
