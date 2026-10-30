# Key scheme `fps-v1`

How Passport Prime derives every key it uses for a Fedimint federation, and what it gives the phone.
The reference implementation is `crates/signer-core/src/keys.rs`; frozen test vectors are in
`crates/signer-core/tests/vectors/key-scheme-fps-v1.json`. Any other implementation, such as the
phone-side derivation added to the fedimint client in Phase 2, must reproduce those vectors.

## Goals

- The phone can derive the public nonce and the blinding key of every note the wallet owns, so it can
  mint, receive, recover and rescan without the device.
- The phone cannot derive any private key that spends a note.
- Keys handed over in an ecash token reveal nothing about any other key.
- Different federations share nothing, so nobody can link a user's notes across federations.

## Notation

- `HMAC512(key, data)` is HMAC-SHA512; `||` is concatenation; `a[..32]` is the first 32 bytes.
- `G` is the secp256k1 generator, `n` its order. `ser_P(P)` is the 33-byte compressed point.
- `parse256(b)` reads 32 bytes big-endian. A derivation whose scalar is 0 or not below `n` fails; the
  probability is about 2^-128 and callers treat that position as unusable.
- Tags are ASCII strings, written here in quotes.

## Per-federation secrets (device only)

Inputs: the 32-byte app seed from KeyOS `GetAppSeed`, and the 32-byte federation id.

```
F             = HMAC512(app_seed, "fps-v1/federation" || federation_id)
I             = HMAC512(F, "fps-v1/spend-root")
k             = parse256(I[..32])                  spend root private key
c             = I[32..]                            chain code
K             = k·G                                spend root public key
v             = HMAC512(F, "fps-v1/view")[..32]    view secret
o             = HMAC512(F, "fps-v1/outgoing")[..32] outgoing secret, never exported
```

The device never stores these; it re-derives them from the app seed. Changing the app id changes
`GetAppSeed`, and with it every key: never change it.

## View material (exported to the phone)

`(K, c, v)`, as `ViewMaterial { spend_key: ser_P(K), chain_code: c, view_secret: v }`. Exporting needs
the user's approval: whoever holds it sees every note in the federation.

## Spend-branch keys (derivable from the view material)

A child of `(K, c)` for a tag and a payload, in the style of BIP32 public derivation with an
arbitrary-length index:

```
t       = parse256(HMAC512(c, tag || ser_P(K) || payload)[..32])
child   = K + t·G           public, computable by the phone
secret  = k + t mod n       private, device only
```

| Use | Tag | Payload |
|---|---|---|
| Mint v2 note | `"fps-v1/mintv2-note"` | `denomination` (1 byte) `|| tweak` (16 bytes) |
| LNv2 refund key | `"fps-v1/lnv2-refund"` | `ser_P(ephemeral_pk)` of the contract |

The note's public key is its mint v2 nonce. A refund key depends on the contract's ephemeral point,
which the contract publishes, so a recovering phone can recognise its own contracts.

**Never release a spend-branch private key.** Anyone with `(K, c)` and one child secret computes
`k = secret - t` and with it every key. Spend-branch keys are only ever used to sign
(decision D11).

## Outgoing-branch keys (device only)

Notes that will be sent as ecash:

```
secret  = parse256(HMAC512(o, "fps-v1/mintv2-outgoing" || denomination || tweak)[..32])
nonce   = secret·G
```

These are hardened: knowing one secret reveals nothing about `o` or any other key. They are the only
keys that ever leave the device, inside a token. The phone learns their public nonces through
`DeriveOutgoingNonces`, which needs no approval because the answer is public keys only.

## Blinding keys and the blinded nonce

For a note at `(branch, denomination, tweak)`, with `branch_byte` 0 for spend and 1 for outgoing:

```
b       = Scalar::from_bytes_wide(HMAC512(v, "fps-v1/mintv2-blind" || branch_byte || denomination || tweak))
M       = tbs Message::from_bytes_sha256(ser_P(nonce))      mint v2 nonce_message
B       = M · b                                             the output's blinded nonce
```

`b` is a BLS12-381 scalar reduced from 64 bytes, as fedimint's own `to_bls12_381_key` does. The mint
signs `B`; the phone unblinds the mint's signature with `b^-1`.

## Tweaks and the recovery filter

Mint v2 publishes a 16-byte tweak in every note output. Clients grind tweaks so that recovery can skip
almost every foreign output with one hash. The filter seed is derived from the view secret:

```
filter  = HMAC512(v, "fps-v1/mintv2-tweak-filter")[..32]
valid   ⇔ SHA-256(tweak || filter)[0..2] == [0, 0]
```

This is exactly mint v2's `check_tweak` (the consensus encoding of `(tweak, filter)` is the two arrays
back to back). About one tweak in 65,536 passes. The device refuses change and staged outputs whose
tweak fails the filter, because recovery would never find them.

## Recovery

Recovering a wallet needs only the device and its seed. The phone gets the view material again and
scans the federation's published outputs: for each output whose tweak passes the filter, it computes
the spend-branch nonce and blinded nonce for that `(denomination, tweak)` and compares the hash with
the published one. Outgoing-branch notes cannot be recognised from view material alone; the phone can
ask the device for their nonces. They are transient (sent or reclaimed), so this is rarely needed.

## Compatibility

Notes under this scheme are not recoverable by a stock fedimint client, which derives notes from a
single root secret with no public derivation. Phase 2 adds this scheme to a fork of the fedimint mint
v2 client.
