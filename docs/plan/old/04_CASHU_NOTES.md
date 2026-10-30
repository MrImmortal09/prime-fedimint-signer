# 04. Cashu integration notes

## Why Cashu changes the signer design

Fedimint transactions carry Schnorr signatures by the note keys, so a signer boundary is natural. Cashu
proofs are plain bearer secrets: presenting them to the mint spends them, no signature involved. The only
way to require Prime's signature is NUT-11 P2PK locking, and the only flag that binds the *destination* is
`SIG_ALL`:

- `SIG_INPUTS` (the default) signs only `Proof.secret`. A proof signed once can be swapped to any outputs
  the phone picks later. Prime's display would bind the amount but not where it goes.
- `SIG_ALL` signs all inputs and all outputs, and for melt also the quote id and the blank outputs. One
  signature on the first input covers the whole transaction. This is what the Cashu card protocol uses.

Therefore every proof the wallet holds is locked to a Prime key with `SIG_ALL`, and every spend is a
`SIG_ALL` swap or melt signed on Prime.

## What CDK supports today (v0.18.3, 2026-10-29)

| Piece | Status |
|---|---|
| Mint enforces P2PK `SIG_ALL` on swap and melt | Yes, with tests |
| Wallet spends `SIG_ALL` proofs | No. `verify_p2pk` returns `SigAllNotSupportedHere`; the melt saga rejects it because change outputs are created at confirmation |
| Wallet signs `SIG_INPUTS` proofs | Yes, with keys it holds (`p2pk_signing_keys`) |
| Wallet locks new outputs to a key by default | No wallet-wide setting; `send` can lock a token to a recipient key |
| Deterministic secrets (NUT-13) | Yes for plain secrets; not for P2PK secrets |
| Mint info flags | `nut11`, `nut12` and others readable from NUT-06 |
| Mobile bindings | `cdk-ffi` UniFFI for Kotlin, Swift, Python, Dart, Go; `cdk-kotlin` and `cdk-swift` packages |

## Problems to expect, and what we do

| Problem | Why it bites | What we do |
|---|---|---|
| CDK wallet cannot spend `SIG_ALL` proofs | The wallet refuses in `verify_p2pk` and in the melt saga, and the saga creates change outputs after the point where signing must happen | Add `SIG_ALL` message assembly for swap and melt, reorder the melt saga so blank outputs exist before signing, expose a signer callback. Upstream it |
| `SIG_INPUTS` is unsafe for a signer | Signature covers only the secret | Use `SIG_ALL` only; refuse mints that do not advertise NUT-11 or fail the `SIG_ALL` test |
| Outputs are blinded | Prime cannot tell where change goes from `B_` alone | The request carries each output's opening (secret, `r`); Prime recomputes the point and reads the lock |
| No "lock everything to Prime" mode | Mint, receive and swap paths generate plain secrets | Wallet-wide default output conditions, a CDK change |
| Linkability | Every proof carries the lock public key in its secret, so a mint can tie all proofs to one key. Fedimint's blind nonces avoid this | Rotate the lock key per receive batch. `SIG_ALL` requires all inputs of one transaction to share a key, so a spend combines proofs from one batch only, or consolidates first |
| Recovery | NUT-13 does not cover P2PK secrets | Derive nonce and `r` from the view secret per keyset and counter; restore with NUT-09 by rebuilding the blinded messages |
| Single custodian per mint | A mint can vanish, freeze funds, or be a lookalike URL | Show the mint URL on Prime every time; keep per-mint balances; cross-mint moves cost Lightning fees |
| Keyset rotation and input fees | Proofs belong to a keyset; inactive keysets must be swapped; fees are per proof | Push mint keys to Prime like the federation config, refresh on rotation, prefer few large proofs |
| Weak offline checks | DLEQ (NUT-12) is optional per mint and even then proves only authenticity, never spent state | Mark received tokens unverified until the swap confirms; check NUT-07 state before spends |
| Melt lifecycle | Quotes expire, proofs sit pending while the mint pays, overpaid fees return as change | A new quote is a new Prime approval; handle pending and failed states explicitly |
| Two signing domains on Prime | Fedimint signs a 32-byte hash; Cashu signs SHA-256 of a concatenated string with strict formatting (lowercase hex, decimal amounts, exact secret serialisation) | Separate verifier crates; test vectors from NUT-11 and NUT-00 |
| Alpha SDK | CDK calls itself alpha, releases often, and the mobile packages lag the core crate | Pin versions; build Kotlin bindings locally from our fork |
| Units | Keysets have units (sat, usd, eur, ...); Fedimint uses msat | Prime shows the unit; the phone never mixes units in one spend |
| `SIG_ALL` spec revision | The melt message was revised to include the quote id and blank outputs; older mints may implement the earlier form | Test each mint before demoing on it; start with cdk-mintd and a nutshell test mint |

## Mint requirements and how to check

| Requirement | Why | Check |
|---|---|---|
| NUT-11 P2PK with `SIG_ALL` enforced | The signer boundary | Mint info `nut11.supported`, then one locked test spend |
| NUT-07 state check | Pending and failed handling | Mint info |
| NUT-08 blank outputs | Melt change | Mint info |
| NUT-12 DLEQ | Offline authenticity of received tokens | Mint info; optional |
| NUT-09 restore | Recovery | Mint info |
| NUT-17 websockets (optional) | Live updates | Mint info |

Local oracle: `cdk-mintd` with `cdk-fake-wallet` as the Lightning backend. Public test mints exist
(nutshell-based, for example under `testnut.cashu.space`); confirm NUT-11 support in mint info before
using one in a demo.

## The `CashuSign` payload

| Field | Purpose |
|---|---|
| mint URL, unit, keyset id | Identity and display |
| inputs | Full proofs: amount, id, secret, `C` |
| outputs with openings | amount, keyset id, secret, `r` for each; Prime recomputes `B_` |
| kind | `melt { quote_id, invoice, fee_reserve }` or `swap { intent }` |
| lock key id | Which Prime batch key the inputs use |

## Linkability design detail

Each receive operation gets a fresh lock key `cashu/<mint>/<batch>`. All proofs from that receive share the
key. A spend may use proofs from exactly one batch in one `SIG_ALL` transaction. If an amount needs proofs
from several batches, the phone first consolidates them with a swap into a new batch (Prime signs it as a
self-transfer), then spends. The mint still links proofs within a batch, which is the price of P2PK; across
batches it sees unrelated keys.
