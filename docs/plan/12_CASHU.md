# 12. Cashu as a second backend

Planned 2026-10-31, after the Fedimint phases 0 to 4. Prime signs Cashu spends the way it signs
Fedimint spends: the phone builds every operation and talks to the mint, and Prime checks, shows
and signs. This replaces the Cashu parts of `old/` (D2 in `10_DECISIONS.md` explains why Cashu was
dropped; D14 records why it is back and what changed). It is built as a stack of pull requests,
`cashu/0-plan` to `cashu/5-ios`, listed at the end.

## Rule zero: Fedimint is not disturbed

Everything Cashu is added beside the Fedimint code, never through it:

| Invariant | How the Cashu work keeps it |
|---|---|
| `PROTOCOL_VERSION` stays 1; existing requests, responses, `RejectCode` and every nested type keep their exact CBOR | Two wrapper variants are appended, `Request::Cashu` and `Response::Cashu`, in a new `cashu.rs`. Enums are tagged by name, so existing bytes cannot change (pinned by `crates/protocol/tests/compat_v1.rs`). An older Prime answers a Cashu request with `Rejected(Malformed)`, which the phone reports as "this Prime app has no Cashu" |
| `DeviceInfo`, `StatusInfo`, `ViewMaterial` and `KEY_SCHEME = "fps-v1"` are frozen | Cashu has its own `Info` answer and scheme name, `fps-cashu-v1`, carried only in Cashu answers. `busy` and `pending` in the Fedimint answers keep describing Fedimint only |
| `fps-state.cbor` (`State`, `STATE_VERSION = 1`, strict) never holds anything new | Cashu keeps `fps-cashu.cbor`, with its own version, and only reads the policy through `Signer::policy()`. A broken Cashu file disables Cashu, is not rewritten, and leaves the Fedimint signer open (pinned by `crates/signer/tests/compat_state.rs`) |
| The app id, `GetAppSeed`, the `fps-v1` derivation and its frozen vectors | Cashu keys use their own tag prefix, `fps-cashu-v1/`, from the same app seed, with their own vectors file. The vector files' hashes are pinned (`crates/signer-core/tests/pins.rs`) |
| The golden vectors and the devtools cross-checks | Cashu vectors come from a separate host workspace, `devtools-cashu/`, into `crates/cashu-core/tests/vectors/` |
| `crates/signer` behaviour | Two additive edits: `ApprovalKind::Cashu` (never matched, never serialised) and the arm `Request::Cashu(_) => Unsupported` that the exhaustive `match` in `dispatch` needs. A new crate's `Device` wraps `Signer` (through `Deref` for every call the app already makes) and routes Cashu requests; with Cashu idle its answers are byte-identical to the bare `Signer`'s, with an empty or a populated Cashu file (a differential test) |
| The crates `wallet/` uses by path: protocol, signer, signer-core, transport, simdrive | They gain no normal dependencies, and no Cashu crate adds a feature to `bitcoin` or `secp256k1` (on the device, `rand-std` alone would change the Fedimint signing path). A guard CI job runs `cargo metadata --locked` in the root workspace and in `wallet/` |
| `wallet/`: members, `Cargo.lock`, the fedimint fixtures, the CI `wallet` job | Untouched. CDK cannot share `wallet/Cargo.lock` without bumping four crates fedimint compiles (parking_lot, lock_api, parking_lot_core, unicode-normalization), and `cdk-sqlite` cannot resolve there at all (a `links = "sqlite3"` clash with fedimint-ldk-node). The phone's Cashu code lives in its own workspace, `cashu/` |
| fps-mobile, its UniFFI surface, `fps_mobile.swift`, the Kotlin bindings, `scripts/ios-core.sh` | Not edited. Cashu gets its own UniFFI namespace, `fps_cashu`. A new `scripts/ios-core-cashu.sh` builds an umbrella library holding both namespaces and checks that the regenerated `fps_mobile.swift` is byte-identical. All Cashu Swift sits inside `#if canImport(fps_cashuFFI)`, so a core from `ios-core.sh` still builds today's app |
| The patch series in `patches/fedimint` | No new patches |
| The simulator scripts, simdrive, review titles and log lines | Cashu reviews use the same review page, with titles that start with "Cashu", and come up only for Cashu requests. No Cashu log line contains `signer open` or `on screen` (the scripts and simdrive wait on those). Cashu has its own simulator script |
| The iOS files touched by the collaborator branch and the uncommitted edits | Cashu UI lives in new files. The one shared edit is `PrimeWalletApp.swift`, which neither touches |

What does change for Fedimint is the iOS app built with Cashu: the umbrella compiles the same
fedimint sources with four patch-level bumps (parking_lot 0.12.5, lock_api 0.4.14, parking_lot_core
0.9.12, unicode-normalization 0.1.26) and additive features on six crates (bitcoin `base64` and
`rand`, ring `std`, uuid `serde` and `v7`, regex-syntax `unicode`, dnssec-prover `validation`,
hex_lit). A gate diffs fps-mobile's dependency subgraph between `wallet/` and the umbrella against
exactly this allowlist, the fedimint fixtures run once on a copy of `wallet/` with the four bumps
applied, and pull request 5 needs a Fedimint smoke test on the phone (receive, pay, send through
Prime) with that build before it merges. Whether to ship that build is the owner's call at that
merge; `ios-core.sh` keeps producing today's build either way.

The regression gate after every pull request: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`, the golden diff and the devtools
cross-checks, `cd wallet && cargo test`, and, from the pull request that changes the Prime app on,
the device build and both simulator checks (`scripts/sim-e2e.sh`, `scripts/sim-wallet-e2e.sh`, in a
foreground shell).

## Corrections to the old Cashu plan

Research on 2026-10-31 (CDK v0.18.3 and main, the NUTs at `1363de4`, two build experiments, three
adversarial reviews of this plan) changed six things in `old/04_CASHU_NOTES.md`:

1. **No CDK fork.** The `cashu` crate exposes the SIG_ALL message through the public trait
   `SpendingConditionVerification` (`sig_all_msg_to_sign`, `verify_spending_conditions`), and the
   witness can be attached through `inputs_mut()`. CDK's high-level `Wallet` still cannot spend
   SIG_ALL proofs, cannot receive into locked proofs and has no signer hook, so the phone runs its
   own flows on the `cashu` types and CDK's `MintConnector`/`HttpClient`.
2. **One SIG_ALL transaction spends one lock key.** All inputs must share `Secret.data` and the
   tags (NUT-11). Merging N batches takes N swaps, not one. Lock keys therefore rotate per *epoch*
   (an explicit action), not per receive, so ordinary spends never need consolidation.
3. **Prime cannot verify where a melt pays.** The signature commits to the quote id, a random
   handle at the mint, and quote responses are unsigned. A compromised phone can show Prime a
   genuine invoice while the quote pays another, so everything a melt spends is at risk, not just
   the invoice amount. Prime counts and shows it that way.
4. **The SIG_ALL message is ambiguous.** NUT-11 joins `amount‖B_` with no separator. A phone that
   grinds blinding factors can build a second output list with the same message, even with
   power-of-two amounts: a review of this plan built one in which Prime would show 5 sats leaving
   while 1,024 leave. Prime refuses any output transcript with more than one reading.
5. **The `cashu` crate cannot run on Prime.** Its `uuid` dependency pulls getrandom 0.4, which has
   no Xous backend, and it would switch on bitcoin's `rand-std` for the whole app. Prime gets a
   small standalone crate instead (about 100 KiB of code, no JSON parser), checked against the
   `cashu` crate on the host.
6. **`cdk-swift` and `cdk-ffi` cannot ship beside fps-mobile.** Two Rust static libraries give
   duplicate symbols (`_rust_eh_personality`, `rustsecp256k1_v0_10_0_*`), and `cdk-ffi` pins UniFFI
   0.30. CDK is compiled into the one library the app links.

## What Prime holds and checks

### Key scheme `fps-cashu-v1`

Notation as in `docs/KEY_SCHEME.md` (`HMAC512`, `parse256`, `ser_P`); `ser32`/`ser64` are
big-endian integers. Per mint, from the 32-byte app seed and the mint's canonical URL:

```
mint_id   = SHA256(mint_url)                       canonical URL bytes (below)
M         = HMAC512(app_seed, "fps-cashu-v1/mint" || mint_id)
p         = parse256(M[..32])                      lock root private key, never exported
v         = M[32..]                                view key, exported
P         = p·G                                    lock root public key, exported

t_e       = parse256(HMAC512(v, "fps-cashu-v1/lock" || ser32(e))[..32])
P_e       = P + t_e·G                              lock key of epoch e
k_e       = p + t_e mod n                          its private key, on Prime only

X         = HMAC512(v, "fps-cashu-v1/output" || ser32(e) || len(K) || K || ser64(c))
nonce     = lowercase hex of X[..32]               secret nonce of output c in keyset K, epoch e
r         = parse256(X[32..])                      its blinding factor
```

`K` is the keyset id's bytes (8 for a v00 id, 33 for a v01 id) and `len(K)` one byte. A derivation
whose scalar is 0 or not below `n` fails; the phone skips that counter. The phone holds `P` and `v`,
so it derives every lock key, secret and blinding factor without Prime, which keeps receives and
restores phone-only. It cannot derive any `k_e`. Prime never releases a Cashu private key, so the
non-hardened tweak is safe (the reason for D11 does not arise). Every tag starts with
`fps-cashu-v1/`, which differs from `fps-v1/` at byte 4, so no Cashu key can equal a Fedimint key.

The canonical URL is CDK's `MintUrl` form: lower-case scheme and host, no trailing `/`. Prime
refuses any other form, so the same mint always gives the same `P` and `v`.

### The secret template

Every proof the wallet holds has exactly this 192-character secret, which is also what CDK writes
for a single-key SIG_ALL lock and what CDK mints accept:

```
["P2PK",{"nonce":"<64 lowercase hex>","data":"<ser_P(P_e) as 66 lowercase hex>","tags":[["sigflag","SIG_ALL"]]}]
```

No `locktime` (an expired lock without a `refund` tag is spendable by anyone), no extra keys. Prime
checks the template byte for byte and parses nothing else. Secrets are never re-serialised: the mint
signed `hash_to_curve` of these exact bytes.

### Requests

| Request | Approval | Answer |
|---|---|---|
| `Info` | none | scheme name and the imported mints |
| `AddMint { url, unit, keysets }` | "Cashu: add mint" (or "Cashu: update mint keys" for a known mint) | `LockMaterial { mint, scheme, lock_pubkey: P, view_key: v }`, never stored for replay |
| `Sign(swap)` | "Cashu: send ecash"; when every output is Prime's, "Cashu: combine ecash" (same epoch) or "Cashu: move to a new key" (a later epoch) | `Signed { message_hash, signature }` |
| `Sign(melt)` | "Cashu: pay Lightning invoice" | the same |

Keysets carry their id, unit, `input_fee_ppk`, `final_expiry` (optional), `active` flag and
per-amount public keys. Prime checks that each id is derived from its keys by the NUT-02 rules (v00:
`00` and the first 7 bytes of SHA-256 over the keys sorted by amount; v01: `01` and SHA-256 of
`amt:hex,…|unit:sat[|input_fee_ppk:f][|final_expiry:t]`), that the unit is `sat` (the only unit
in this version), and pins them. Only existing reject codes are used.

### Routing on Prime

A new crate, `crates/cashu-signer`, holds the Cashu service and `Device`, which the app opens in
place of `Signer`. `Device` derefs to `Signer`, so the app's other calls do not change; in
`prime-app` only the import, the field type and the open call change, plus the simulator reset,
which also deletes `fps-cashu.cbor` (and its `.tmp` and `.new`) in the same block that consumes the
reset flag.

- Undecodable bytes and every non-Cashu request go to `Signer::handle` unchanged. The one exception:
  while a Cashu review waits, the four approval requests (`ImportFederation`, `ExportViewMaterial`,
  `SignTransaction`, `ReleaseNotes`) without a stored answer get the same `Busy` answer `Signer`
  gives.
- `Request::Cashu` goes to the Cashu service, which answers `Busy` while `Signer::pending()` is set.
  Cashu answers are stored in the Cashu file's own ring of results.
- `GetResult` and `Cancel` look in Fedimint state first, then in the Cashu service.
- `GetInfo` and `GetStatus` are answered by `Signer`, unchanged.
- Approve and reject go to whichever service owns the waiting review.
- `Device::open` returns `Signer::open`'s error unchanged. A Cashu load, decode or version error
  only disables Cashu.

### What Prime checks before it signs

For every spend:

- 1 to 64 inputs, all in the template with `data = P_e` for the request's input epoch `e`, which
  Prime derives itself. Every input's keyset is pinned; amounts are powers of two that keyset has.
- 0 to 64 outputs, each with an opening `(secret, r)` and an amount that is a power of two in its
  keyset (0 for a melt's blanks). Prime recomputes `B_ = hash_to_curve(secret) + r·G` and refuses
  any mismatch.
- Change: an output whose opening carries a hint `(epoch, keyset, counter)` must equal the
  `fps-cashu-v1` output at that position (Prime recomputes the nonce and `r` from `v`), with an
  epoch from the input epoch to three above it. So every change output can be found again by a
  restore. Every other output is leaving.
- The input fee is `ceil(Σ input_fee_ppk / 1000)` over the inputs' keysets.
- Prime builds the NUT-11 message itself (`secret‖C` per input, then `amount‖B_` per output, then
  the quote id for a melt; lowercase hex, decimal amounts) and signs `SHA256(message)` with `k_e`
  (BIP-340, auxiliary randomness from the TRNG). It never signs a digest the phone supplies.
- **One reading only.** Prime re-parses the output part of the message it built and refuses it
  unless there is exactly one way to split it into (canonical decimal amount, key) pairs whose keys
  parse as points, the only lists a mint signs: 66 hex characters starting `02` or `03`, or, since
  Nutshell hashes `B_` as posted, 130 starting `04`, `06` or `07`. Honest outputs hit this for 0.6%
  of totals below 1,024 sats and about 10% below 2^20 (mostly a 1,024-sat output that is not last),
  so the phone runs the same check before asking Prime and moves to the next counters. The two
  known collisions are kept as negative vectors.

Swap: `Σ inputs − fee = Σ outputs`, or refuse. What leaves is the sum of the leaving outputs plus
the fee.

Melt: the invoice must parse and carry an amount, and the quote amount must be `floor` or `ceil` of
the invoice's msat over 1000 (real Lightning backends round up; CDK's fake one rounds down). The
inputs must cover `amount + fee_reserve + fee`. Every output is a blank (amount 0) in the template,
and there must be enough blanks to return all of `Σ inputs − amount − fee` (at least its bit
length). Because the message does not commit to input amounts, each input carries its NUT-12 DLEQ
proof and Prime verifies it against the pinned key for its claimed amount. Since Prime cannot check
the quote, the outflow is `Σ inputs`: the review shows the invoice's amount, payee and description,
the fee reserve, the input fee and "at most Σ inputs leaves", with the warning that Prime cannot
confirm the mint pays this invoice. The phone keeps that close to the invoice by splitting first
when its proofs overshoot.

Policy: the outflow (leaving outputs plus fee for a swap, `Σ inputs` for a melt, the fee alone for a
rotation) is checked against the same PIN threshold as Fedimint, read through `Signer::policy()` at
each decision and never written to `fps-state.cbor`.

### What Prime cannot check

- That a melt quote pays the invoice shown (above). Use mints you trust.
- That the mint honours SIG_ALL, or is the mint at that URL. Mint info only says `"11":
  {"supported": true}`, so the phone refuses mints that lack NUT-07, 08, 09, 10, 11 or 12 or whose
  `version` names a release with the old SIG_ALL message (Nutshell ≤ 0.20.2, CDK ≤ 0.13), and keeps
  a new mint *unverified*, capped at 1,000 sat, until an unsigned swap of one of its proofs has been
  refused and the first signed spend has gone through.
- That a proof is unspent. DLEQ proves the mint signed it, nothing more.
- Mint keys are trust-on-first-use from the paired phone, as federation configs are (D12). A v00
  keyset id commits to neither unit nor fee, so for v00 keysets those rest on that trust too.
  Neither id version ties a key to its amount, and a melt counts each input at the amount its DLEQ
  proof's key is pinned for, so an update only adds keysets: a pinned keyset keeps its keys and
  fee, one the mint stops listing stays pinned, and no key may stand for two amounts. The melt
  limit therefore holds for proofs of keysets pinned while the phone was honest; a keyset first
  pinned during a compromise is trusted on first use like any other.
- Receives are phone-only, so a compromised phone can redirect Cashu that is still arriving (D13).
- Within one epoch the mint can link the wallet's proofs, which share a lock key. Rotating starts a
  new, unlinkable key and costs one swap per old epoch.

## The phone

### Workspace

`cashu/` is its own cargo workspace, beside `wallet/`, with its own lockfile and its own CI job:

| Crate | What it is |
|---|---|
| `cashu/fps-cashu` | The watch-only Cashu wallet: mint client, store, flows, the phone half of `fps-cashu-v1` (shared with Prime through `fps-cashu-core`), the Prime link. No fedimint. UniFFI exports behind its `uniffi` feature, namespace `fps_cashu`, every exported name prefixed `Cashu` so nothing collides with fps_mobile's Swift names |
| `cashu/fps-core` | The umbrella the iOS app links: `fps-mobile` (unchanged, by path) plus `fps-cashu`, as one static library, with its own `uniffi-bindgen` binary. Not a default member, so `cargo test` in `cashu/` never builds fedimint |

The lockfile is seeded from `wallet/Cargo.lock` with `fps-core` already present (pull request 3),
because seeding without it prunes the fedimint entries and a later resolution moves 119 of
fps-mobile's crates. A check compares `cargo tree -p fps-mobile --target aarch64-apple-ios -e
normal,build` in both workspaces: they may differ only by the four bumps. Every cargo command in
`cashu/` loads fps-core's path dependencies, so its CI job runs `scripts/fedimint-checkout.sh`,
installs `protoc` (CDK's mint pulls `cdk-signatory` with gRPC), and passes `-p fps-cashu` to fmt,
clippy and test (`cargo fmt --all` there would reformat `third_party/fedimint` and `wallet/`).

Dependencies: `cashu` and `cdk` `=0.18.3`, default features off, `wallet` on. No `cdk-sqlite`,
`cdk-redb` or `cdk-ffi` in shipped code; the store is a JSON file written atomically, as fps-mobile
does. Tests run CDK's mint (`cdk` with `mint`, `cdk-sqlite` in memory, `cdk-fake-wallet`) in the
test process through a copy of CDK's `DirectMintConnection`, which resolves in `cashu/` because
fedimint's dev-dependencies are not members there.

TLS: CDK installs ring as rustls's process default if nothing is installed, while fedimint installs
aws-lc-rs lazily on its first websocket. fps-cashu refuses to create a mint client until a provider
is installed (its tests install ring), and exports a synchronous `cashu_install_tls()` that installs
aws-lc-rs, the provider fedimint would install, when none is set. The app calls it at launch,
before either wallet opens.

### Flows

| Flow | Prime | How |
|---|---|---|
| Add a mint | approve | Canonical `MintUrl`; read mint info and keysets; refuse as above; `AddMint`; keep `P`, `v`; the mint starts unverified |
| Update mint keys | approve | When the mint shows a keyset Prime has not pinned (before spending from it or minting into it) |
| Top up by Lightning | none | Mint quote; once paid, mint outputs in the template for the current epoch, with deterministic counters, into an active keyset |
| Receive a token | none | `cashuA`/`cashuB`; the token's mint must be added; tokens with P2PK or HTLC secrets are refused; swap its plain proofs into the template for the current epoch |
| Send a token | approve | Inputs from the current epoch; outputs are the recipient's plain proofs plus change; SIG_ALL swap; `cashuB` token. An unredeemed token is taken back with a swap of the same secrets |
| Pay an invoice | approve | Melt quote; split first if the proofs overshoot; inputs from the current epoch plus blanks; SIG_ALL melt; change from the blanks; pending melts are checked again later |
| Combine | approve | A same-epoch swap into fewer, larger proofs, when a spend would need more inputs than one request should carry (about 16 over QR), and to move proofs out of a keyset before its `final_expiry` |
| Rotate | approve per old epoch | Next epoch, only if the current one holds proofs; each old epoch's proofs are swapped into the new key |
| Restore | none (re-add the mint for `P`, `v`) | Per epoch from 0 until four in a row are empty (change may sit three epochs above its inputs), and per keyset, inactive ones included: NUT-09 in batches of 100 until three are empty; NUT-07 drops spent proofs |

Counters advance before a request leaves (write-ahead), so a crash never reuses an output; counters
of a request that Prime declined or the mint refused are used again, so restores meet no long gaps.
Before posting a signed swap or melt, the phone saves it with every output's opening, the
recipient's included; after a crash it re-posts the identical request (NUT-19 answers it again) or
finds the outputs with NUT-09. The phone verifies Prime's signature against `P_e` and runs
`verify_spending_conditions` before posting.

### iOS

All Cashu Swift, the generated `fps_cashu.swift` included, sits inside `#if canImport(fps_cashuFFI)`;
the umbrella's xcframework declares that Clang module beside `fps_mobileFFI` in one
`module.modulemap`. With a core from `ios-core.sh` the app is exactly today's. With a core from
`ios-core-cashu.sh`, `PrimeWalletApp.swift` (the one shared edit) puts a Cashu screen in a `ZStack`
layer above the unchanged `RootView`, opened from a small button, rather than in a `TabView`: the
Prime exchange sheet, busy overlay and error alert are modifiers on `RootView`, and the Cashu screen
uses the same `PrimeBridge`, so every exchange, Fedimint refunds included, still shows on the one
sheet.

## Pull requests

Each pull request is implementation commits followed by one test commit, and is stacked on the one
before it.

| # | Branch | Delivers | Tests |
|---|---|---|---|
| 0 | `cashu/0-plan` | This plan, D14 | Compatibility pins against the current code: the CBOR of every v1 request and response, a populated v1 `fps-state.cbor`, the hashes of both Fedimint vector files; a CI guard job running `cargo metadata --locked` in the root and in `wallet/` |
| 1 | `cashu/1-core` | `crates/cashu-core` (`fps-cashu-core`): `fps-cashu-v1`, canonical URLs, template, `hash_to_curve`, `B_`, NUT-11 message and its one-reading check, NUT-12 DLEQ, keyset ids, swap and melt analysis with reviews, signing. `devtools-cashu/` produces vectors with `cashu` 0.18.3 | Unit tests, vectors, cross-checks against the `cashu` crate (its own `verify_spending_conditions` accepts our signatures), the collision transcripts refused, key separation from `fps-v1`, the device build, CI job `cashu-vectors` |
| 2 | `cashu/2-prime` | Protocol variants and `PROTOCOL.md`; `crates/cashu-signer` (the service with `fps-cashu.cbor`, and `Device`); `ApprovalKind::Cashu` and the dispatch arm; the Prime app opens `Device` | Service tests, the differential test against the bare `Signer` (empty and populated Cashu file), cross-backend Busy, GetResult and Cancel, old-phone and old-Prime compatibility, state-file isolation, device build and both simulator checks |
| 3 | `cashu/3-wallet` | `cashu/` workspace (with a bare `fps-core`, so the lockfile seeds right) and `fps-cashu` with every flow | Against CDK's mint in-process and the real Cashu service as Prime: add, top up, receive, send and redeem, take back, pay, decline, combine, rotate, restore on a new phone, the mint refusing an unsigned spend, the subgraph check; CI job `cashu-wallet` |
| 4 | `cashu/4-simulator` | `scripts/sim-cashu-e2e.sh` | `fps-cashu` against the Prime app in the simulator, every review decided on screen; both Fedimint simulator checks again |
| 5 | `cashu/5-ios` | `fps_cashu` UniFFI API with `cashu_install_tls`, the umbrella's bindgen, `scripts/ios-core-cashu.sh`, the Cashu screen in Prime Wallet (only in a Cashu build) | Rust tests for the exported API, the unchanged `fps_mobile.swift`, the fps-mobile subgraph against the allowlist, the fedimint fixtures once on a copy of `wallet/` with the four bumps applied, an `xcodebuild` of the app with each core |

## Risks

| Risk | Mitigation |
|---|---|
| Ambiguous output transcripts (finding 4 above) | The one-reading check and its negative vectors; move to a length-framed message (nuts #404) once mints support it |
| The NUT-11 message changes again (nuts #404 is open) | CDK mints count only signatures that verify, so a later version can sign both forms; the message builder is one function with vectors |
| A mint runs an older SIG_ALL form (Nutshell ≤ 0.20.2, CDK ≤ 0.13) | The locked test spend when the mint is added |
| CDK's API churns | Pinned `=0.18.3`; only the `cashu` types and `MintConnector` are used |
| QR size: a 16-input spend is about 10 KB, 57 codes at medium density | The phone caps inputs per request and prefers the simulator link or QuantumLink for large spends |
| The Cashu build of the iOS app compiles fedimint differently | The allowlisted subgraph diff, `ios-core.sh` kept for a Fedimint-only build, a phone smoke test before pull request 5 merges |
