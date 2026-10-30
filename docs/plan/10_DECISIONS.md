# 10. Decision log

Each entry: the decision, why, and what it would take to reverse it.

## D1. Prime is a signer and trusted display, not a wallet

**Why:** Prime has no network by design, and fedimint-client needs tokio, RocksDB and websockets. Every
existing Prime integration (Envoy, Tether WDK, Cake signer, Agents wallet) splits the work the same way:
phone coordinates, Prime verifies and signs. The split also gives a real security property: the phone
cannot spend without the device.
**Reverse:** port the whole client to the device. Months of work, no precedent.

## D2. Fedimint only; Cashu dropped

**Why:** Fedimint notes already carry signatures the federation checks, so Prime fits as the key holder
with no new protocol mode. Cashu proofs are bearer secrets; a signer only matters if every proof is locked
to a Prime key with `SIG_ALL`, which the Cashu Development Kit's wallet cannot spend today, which makes
proofs linkable by the mint, and which needs a custom recovery scheme and per-mint testing. That is about
the Fedimint effort again for a weaker security and privacy story. Fedimint is also the codebase the
team already contributes to.
**Reverse:** follow `old/04_CASHU_NOTES.md`. The protocol crate reserves a request kind for it. A cheap
middle ground exists: redeem incoming Cashu tokens by melting them to a Lightning invoice paid into the
Fedimint balance, with CDK unmodified on the phone.

## D3. Transport order: QuantumLink primary, UR and file bridge as fallbacks

**Why:** Bluetooth is the natural always-on channel and has Foundation's precedent, pairing and security
story. But KeyOS 1.4.x exposes no QuantumLink app channel to sideloaded apps, so it needs a KeyOS patch
and a dev unit. UR works on a stock retail device today and the file bridge makes CI possible. The
protocol is transport-agnostic so nothing is lost by starting with UR.
**Reverse:** nothing to reverse; QuantumLink v2 replaces the patch when KeyOS ships it.

## D4. Patching KeyOS is acceptable

**Why:** Foundation's own Tether proof of concept did exactly this, with a published patch and firmware
scripts. Custom firmware flashing is documented. The patch is contained to the QuantumLink api crate and
server.
**Cost:** a dev unit, waived security guarantees on that unit, brick risk, and a build that depends on a
KeyOS source tree because the SDK bundle lacks `api/gpio`.

## D5. NFC is an optional stretch stage

**Why:** the OS already emulates a Type 4 tag for FIDO, so the patch is small, but it is hardware-only,
needs the same patched firmware as Bluetooth, and its review flow needs a two-tap design. It adds a
great demo (tap to approve, tap to pay) without being required by any other stage.
**Reverse:** drop Stage 5; nothing else depends on it.

## D6. Phone stack: Android first; React Native with Expo recommended, Kotlin fallback

**Why:** fedimint-sdk ships a React Native package, Tether's QuantumLink module is an Expo module usable
as-is, and JS UR and QR libraries exist, so React Native reuses the most. The Kotlin path is equally
valid with the fedimint Android SDK and a small re-wrap of Tether's Kotlin and Rust. The choice is made
in Stage 0 after a build test of the bindings.
**Reverse:** cheap before Stage 3, expensive after.

## D7. No changes to the federation protocol

**Why:** guardians verify BLS note signatures, spent nonces and Schnorr transaction signatures; none of
that changes. All work is in client libraries and apps, which keeps the project upstreamable and lets it
run on existing federations.

## D8. Module scope: the v2 shape first (mint v2, wallet v2, LNv2), v1 modules later

**Revised in Phase 0.** The first draft said mint v1 first. Reading fedimint 0.13.0-beta.1 showed that
new federations run mint v2, wallet v2 and LNv2 by default, with the v1 modules off unless enabled
with `FM_ENABLE_MODULE_*`, and fedimint-sdk recommends that "v2" shape for its examples and tests.
**Why:** the default shape is what devimint and new federations run. Mint v2 also suits the watch-only
design: every note output carries a public 16-byte tweak, so keys derive from (denomination, tweak)
with no counters, and recovery is a scan over published tweaks.
**Extend:** add the v1 decoders (mint v1, wallet v1, LN v1) for older public federations.

## D9. Derivation: one spend xpub per federation, a separate view secret for blinding keys

**Revised in Phase 0; specified in `docs/KEY_SCHEME.md`.** Per federation, Prime derives a spend root
`(k, c)` and a view secret `v` from the app seed and the federation id, and exports `(k·G, c, v)`.
Mint v2 note keys are non-hardened children keyed by (denomination, tweak).
**Why:** it makes the phone a true watch-only wallet, able to mint, receive and recover without Prime,
and keeps spend keys on the device. Fedimint's upstream scheme derives the spend and blinding keys
from one secret, so it cannot separate the two.
**Cost:** notes under this scheme are not recoverable by a stock fedimint client. Recovery needs the
xpub, the view secret and the scheme version, all re-exportable from Prime's seed.

## D10. Ecash send is a release, with the token shown on Prime's screen or handed to the phone

**Why:** an ecash token is the keys. Showing it on Prime's screen keeps the phone from ever seeing the
keys, which is the strongest option for face-to-face use. Handing it to the phone bounds the exposure to
the approved amount and enables sending over any channel. Both are offered; the user picks per send.

## D11. Released keys come only from a hardened outgoing branch

**Added in Phase 0.** With non-hardened derivation, anyone holding the spend xpub and one child private
key can compute the root private key. An ecash token *is* the note private keys, and the phone holds the
xpub, so releasing a spend-branch key would let a compromised phone, or anyone who later sees both, take
every note.
**Decision:** notes that will be sent as ecash are first moved, by a reissue Prime reviews as a
self-transfer, to an outgoing branch whose keys are hardened (an HMAC of a Prime-only secret). Prime gives
the phone only their public nonces. Prime releases outgoing-branch keys and nothing else, and logs every
release. Spend-branch keys, including LNv2 refund keys, are only ever used to sign.

## D12. Federation import is trust-on-first-use from the paired phone

**Added in Phase 0.** A fedimint federation id is the hash of the guardian API endpoints only. The module
configs (mint keys, fees, network) are not bound to it; clients trust them because a threshold of
guardians serve the same config. Prime has no network, so it checks the federation id against the
endpoints it is given, shows the guardians and the name for the user to confirm, and trusts the paired
phone for the rest. This is safe for spending: a fake mint key can only make Prime accept a note the
federation will reject, and the checks that protect funds (inputs re-derive to Prime's keys, change
re-derives to Prime's tree, external outputs are shown) do not depend on the mint keys.

## D13. Receives stay phone-only, and a compromised phone can intercept them

**Added in Phase 0.** Incoming Lightning claims, ecash reissues and peg-in claims are signed by the phone
with one-shot keys, as decided in D1. Stated explicitly: a compromised phone can redirect funds that are
still on their way in, before they land in Prime-tree notes, and can lie about balances. Everything
already in Prime-tree notes stays safe. A later "secure receive" mode could make Prime's keys the claim
keys, at the cost of needing the device to finish every receive.
