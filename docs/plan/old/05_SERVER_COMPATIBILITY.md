# 05. Server compatibility

**No change to the Fedimint federation or the Cashu mint software is needed.** Every change in this plan
is in client libraries and apps. Existing testnet and mainnet federations and mints work as they are,
subject to the capability requirements below.

## Why the Fedimint side needs no change

A federation only ever sees a normal signed transaction. For each note input it checks the mint's BLS
signature on the nonce, that the nonce was not spent before, and a Schnorr signature by the nonce's public
key over the transaction hash. It does not know or care how the client derived that key or which device
produced the signature. Our changes are all client-side: how keys are derived, where the spend key lives,
and a watch-only note store. Recovery is also client-side: the federation serves its history and the
client does the matching.

## Why the Cashu side needs no change

A mint sees only blinded outputs, so it cannot tell that new proofs are locked to Prime's key. At spend time
it enforces the P2PK lock and `SIG_ALL` exactly as NUT-11 specifies today, and the CDK mint already has
tests for this on swap and melt. The wallet-side gap is library code we edit, not protocol.

## What the server side must already support

| Backend | Requirement | How to check |
|---|---|---|
| Fedimint | Mint module v1 | Present in every federation |
| Fedimint | LNv2 for the Lightning flow, or LN v1 with its output type added to Prime's decoder | The client config lists module kinds |
| Fedimint | Wallet module for on-chain | Client config |
| Cashu | NUT-11 P2PK with `SIG_ALL` enforced | Mint info `nut11`, then one locked test spend |
| Cashu | NUT-07 state check, NUT-08 melt change | Mint info |
| Cashu | NUT-12 DLEQ (optional, for offline token checks) | Mint info |
| Cashu | NUT-09 restore (for recovery) | Mint info |

## Compatibility caveats

- Many public federations still run LN v1 only. Supporting them means one more output type in Prime's
  decoder. fedimint-sdk also documents that on the pinned fedimint release an LN v1 payment succeeds but
  the operation ends with an `Internal` error (fedimint/fedimint#8969).
- The NUT-11 `SIG_ALL` message for melt was revised to include the quote id and blank outputs. An older
  mint may implement the earlier form. Test each mint you plan to demo on.
- A mint that supports P2PK but not `SIG_ALL` would accept per-input signatures instead. Prime could
  produce those, but they do not bind the destination, so the wallet refuses such mints rather than
  degrade silently.
- fedimint-sdk pins fedimint `=0.13.0-beta.1`. The client negotiates API versions with older federations,
  but test against the specific federation versions you will demo on.

## Where to test

- Fedimint: devimint from the fedimint repo for a local federation with gateway and bitcoind; any public
  signet or mutinynet federation by invite code for a second opinion.
- Cashu: `cdk-mintd` with `cdk-fake-wallet` locally; a public nutshell test mint for cross-implementation
  checks of `SIG_ALL`.
