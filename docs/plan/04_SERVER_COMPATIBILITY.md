# 04. Server compatibility

**No change to the Fedimint federation software is needed.** Every change in this plan is in client
libraries and apps. Existing testnet and mainnet federations work as they are, subject to the module
requirements below.

## Why the federation needs no change

A federation only ever sees a normal signed transaction. For each note input it checks the mint's BLS
signature on the nonce, that the nonce was not spent before, and a Schnorr signature by the nonce's public
key over the transaction hash. It does not know or care how the client derived that key or which device
produced the signature. Our changes are all client-side: how keys are derived, where the spend key lives,
and a watch-only note store. Recovery is also client-side: the federation serves its history and the
client does the matching.

## What a federation must run

| Requirement | Why | How to check |
|---|---|---|
| Mint v2 module (`mintv2`) | Notes, reissue, ecash send | The default in fedimint 0.13 federations |
| LNv2 module (`lnv2`) | The Lightning flow as designed | The client config lists module kinds |
| Wallet v2 module (`walletv2`) | On-chain deposit and withdrawal | The default in fedimint 0.13 federations |
| Mint v1, LN v1, wallet v1 | Older federations; need their own decoders in Prime, planned later | Client config |
| At least one gateway | Lightning payments | Gateway list from the LNv2 module |

## Compatibility caveats

- Many public federations still run LN v1 only. Supporting them means one more output type in Prime's
  decoder. fedimint-sdk also documents that on the pinned fedimint release an LN v1 payment succeeds but
  the operation ends with an `Internal` error (fedimint/fedimint#8969). Plan the demo on an LNv2
  federation, devimint by default.
- fedimint-sdk pins fedimint `=0.13.0-beta.1`. The client negotiates API versions with older
  federations, but test against the specific federation version you will demo on.
- Corrected in Phase 0: fedimint 0.13 federations run mint v2, wallet v2 and LNv2 by default, with the
  v1 modules off unless enabled with `FM_ENABLE_MODULE_*`. The first version targets that shape and
  its `ECash` tokens. Federations that only run the v1 modules need the v1 decoders, planned later.
- Federation-side backup (encrypted client backup uploaded to guardians) is optional and stays
  phone-side; it must never contain spend keys, which it cannot under this design.

## Where to test

- devimint from the fedimint repo for a local federation with gateway and bitcoind. From the fedimint-sdk
  repo, `scripts/run-sdk-examples.sh v2 shell` stands one up and hands you the invite code and
  counterparty commands.
- Any public signet or mutinynet federation by invite code for a second opinion, after checking which LN
  module it runs.
