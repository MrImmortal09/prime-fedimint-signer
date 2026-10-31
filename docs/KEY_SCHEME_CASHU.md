# Key scheme `fps-cashu-v1`

How Passport Prime derives every key it uses for a Cashu mint, and what it gives the phone. The
reference implementation is `crates/cashu-core/src/keys.rs`; frozen test vectors are the
`key_scheme` section of `crates/cashu-core/tests/vectors/cashu-0.18.3.json`, written by
`devtools-cashu/src/bin/golden.rs`. The phone half lives in the same crate (`PhoneMintKeys`), so
Prime and the phone share one implementation. The Fedimint scheme, `fps-v1`, is
[`KEY_SCHEME.md`](KEY_SCHEME.md); the design behind both is
[`plan/12_CASHU.md`](plan/12_CASHU.md).

## Goals

- The phone can derive every lock key, proof secret and blinding factor of the wallet, so it can
  receive, make change and restore without the device.
- The phone cannot derive any key that signs a spend.
- Every proof a restore must find has a secret and blinding factor the phone can rebuild byte for
  byte, because NUT-09 restores by `B_` alone.
- Different mints share nothing, and no Cashu key equals a Fedimint key.

## Notation

As in `KEY_SCHEME.md`: `HMAC512(key, data)` is HMAC-SHA512, `||` is concatenation, `a[..32]` the
first 32 bytes, `G` the secp256k1 generator and `n` its order, `ser_P(P)` the 33-byte compressed
point. `parse256(b)` reads 32 bytes big-endian and fails when the result is 0 or not below `n`.
`ser32` and `ser64` are big-endian integers. Tags are ASCII strings.

## Per-mint secrets (device only)

Inputs: the 32-byte app seed from KeyOS `GetAppSeed`, and the mint's canonical URL (below).

```
mint_id   = SHA256(mint_url)
M         = HMAC512(app_seed, "fps-cashu-v1/mint" || mint_id)
p         = parse256(M[..32])                      lock root private key, never exported
v         = M[32..]                                view key, exported
P         = p·G                                    lock root public key, exported
```

The device never stores these; it re-derives them from the app seed.

## Lock material (exported to the phone)

`(P, v)`, as `CashuLockMaterial { mint, scheme: "fps-cashu-v1", lock_pubkey: ser_P(P), view_key: v }`,
the answer to an approved `AddMint`. It is never stored for replay. Whoever holds it can see
every proof of the wallet at that mint, but spend none.

## Lock keys

Lock keys rotate per epoch, an explicit action of the user, not per receive: one `SIG_ALL` spend
can only spend proofs of one lock key, so ordinary spends must not need to merge epochs.

```
t_e       = parse256(HMAC512(v, "fps-cashu-v1/lock" || ser32(e))[..32])
P_e       = P + t_e·G                              lock key of epoch e
k_e       = p + t_e mod n                          its private key, device only
```

The tweak is not hardened: anyone with `v` and one `k_e` computes `p = k_e − t_e` and with it every
lock key. That is safe because Prime never releases a Cashu private key; it only signs with
`k_e`. (Decision D11 forbids non-hardened keys for keys that leave the device, which is the
Fedimint ecash case.)

## Outputs

Every output the wallet creates for itself, at keyset `K` (its id's bytes: 8 for a version 00 id,
33 for a version 01 id), counter `c` and epoch `e`:

```
X         = HMAC512(v, "fps-cashu-v1/output" || ser32(e) || len(K) || K || ser64(c))
nonce     = lowercase hex of X[..32]
r         = parse256(X[32..])                      blinding factor
secret    = the template below, with this nonce and data = ser_P(P_e)
B_        = hash_to_curve(secret) + r·G            NUT-00
```

`len(K)` is one byte. A derivation whose scalar is 0 or not below `n` fails, with probability
about 2^-128; the phone skips that counter. Counters are per keyset and epoch, so a restore knows
where to look.

## The secret template

Every proof the wallet holds has exactly this 192-character secret, which is also what CDK writes
for a single-key `SIG_ALL` lock:

```
["P2PK",{"nonce":"<64 lowercase hex>","data":"<ser_P(P_e) as 66 lowercase hex>","tags":[["sigflag","SIG_ALL"]]}]
```

No `locktime` (an expired lock without a `refund` tag is spendable by anyone) and no other keys.
Prime checks the template byte for byte and parses nothing else. Secrets are never
re-serialised: the mint signed `hash_to_curve` of these exact bytes, and the `SIG_ALL` message
embeds them as they are.

## Canonical mint URLs

`mint_id` hashes the URL, so the same mint must always be spelled the same way. The canonical form
is the one CDK's `MintUrl` renders (`cashu` 0.18.3, `mint_url.rs`): every trailing `/` removed,
scheme and host lower-cased, the path kept as it is. The phone canonicalises with
`url::canonicalize`; Prime refuses any URL that is not already canonical, and any that is not
`https` or `http`, has no host, has user info, a query or a fragment, is longer than 256 bytes or is
not printable ASCII, since the review shows the URL as the mint's name.

## Restore

The phone gets `(P, v)` again with `AddMint` and scans per epoch from 0, and per keyset, inactive
ones included: it derives the outputs of counters 0, 1, … in batches, asks the mint for any
signatures on those `B_` (NUT-09), and drops spent proofs (NUT-07). It stops after four epochs in a
row in which the mint signed nothing. Prime signs change only at the input epoch or up to three
above it (`MAX_EPOCHS_AHEAD`), and only when it is exactly the output at its stated position, and
the wallet moves to a new epoch only from one that holds proofs. So every epoch with proofs has one
with proofs at most three below it, and every change output a restore must find is where it looks.

## Separation from `fps-v1`

Both schemes start from the same app seed. Every Cashu tag starts with `fps-cashu-v1/`, every
Fedimint tag with `fps-v1/`; they differ at byte 4, so no tag of one scheme is a prefix of a tag of
the other and no HMAC message of one equals a message of the other, even for a mint id equal to a
federation id. `crates/cashu-core/tests/key_scheme.rs` checks this for the seeds of the frozen
`fps-v1` vectors: no Cashu key, public or private, equals any `fps-v1` value. The `fps-v1` vectors
themselves are pinned by hash in `crates/signer-core/tests/pins.rs` and do not change.

## Compatibility

Proofs under this scheme are ordinary NUT-11 P2PK `SIG_ALL` proofs, which any mint supporting
NUT-10 and NUT-11 accepts, but no stock wallet can find them again: NUT-13 defines no derivation for
P2PK secrets. Restoring needs this scheme and the lock material from the device.
