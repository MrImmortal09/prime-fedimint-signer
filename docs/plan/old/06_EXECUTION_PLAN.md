# 06. Execution plan

## Repositories

| Repo | Role in the project | Do we edit it? | How we consume it |
|---|---|---|---|
| `fedimint/fedimint` | Client crates: transaction builder, mint client | Yes. Fork from tag `v0.13.0-beta.1`, keep the version string | `[patch.crates-io]` in fedimint-sdk so the exact-version pins still resolve |
| `fedimint/fedimint-sdk` | Rust SDK, Kotlin and Swift bindings | Yes. Fork | Build the Android SDK from the fork with the patched fedimint crates |
| `cashubtc/cdk` | Cashu wallet, mint, `cdk-ffi` bindings | Yes. Fork | Generate Kotlin bindings locally with `just ffi-generate kotlin` from the fork |
| `cashubtc/cdk-kotlin`, `cdk-swift` | Published mobile packages | Not until upstreaming | Replaced by locally generated bindings during the hackathon |
| `Foundation-Devices/KeyOS` | OS, SDK, QuantumLink service | Yes. One patch, Stage 5 only | Build firmware from the tree, flash a dev unit |
| `Foundation-Devices/passport-tether-wdk` | Reference for the KeyOS patch, firmware scripts, phone QL module | No | Copy its patterns and scripts |
| `Foundation-Devices/quantum-link`, `foundation-api` | QuantumLink protocol crates for the phone side | No | Vendored into the phone QL module |
| `Foundation-Devices/hummingbird` | Java UR implementation | No | Phone-side animated UR |
| **new** `prime-ecash-signer` | The KeyOS app and its workspace: `protocol`, `fedimint-signer-core`, `cashu-signer-core`, `policy`, UI | Ours | Scaffolded with `foundation new`, sideloaded as a signed `.app` |
| **new** `ecash-phone` | Kotlin Android wallet: both SDKs, input router, transports, signer bridge | Ours | Built against the fork artifacts |
| **new** `ecash-devtools` | devimint and cdk-mintd scripts, simulator file bridge, golden vector generator | Ours | Host only |

Not edited and not needed: the Fedimint federation code and the Cashu mint.

## Dependency tree

Arrows read "needs". Leaves at the bottom can start on day one.

```
M5  Bluetooth demo on hardware
├── KeyOS QuantumLink patch + custom firmware          [KeyOS fork, dev unit]
├── Phone QuantumLink transport module                 [ecash-phone, ported from tether-wdk mobile/modules/quantumlink]
└── M4  Both backends on hardware over QR/UR
    ├── Phone app                                      [ecash-phone]
    │   ├── fedimint-sdk Android with signer hook      [fedimint-sdk fork]
    │   │   └── signer trait + watch-only mint         [fedimint fork]
    │   ├── CDK Kotlin bindings with SIG_ALL signer    [cdk fork]
    │   ├── protocol crate                             [prime-ecash-signer/protocol]
    │   ├── input router                               [ecash-phone]
    │   └── UR transport via hummingbird               [ecash-phone]
    └── Prime app                                      [prime-ecash-signer]
        ├── fedimint-signer-core  <- golden vectors from fedimint-core on the host   [ecash-devtools]
        ├── cashu-signer-core     <- golden vectors from the cashu crate on the host [ecash-devtools]
        ├── protocol crate
        ├── policy + review screens
        └── transports: file bridge -> QR/UR -> QuantumLink

M2 (Fedimint end to end) and M3 (Cashu end to end) run on the simulator file bridge with host test
drivers before the phone app exists.
```

Critical path: fedimint fork, then fedimint-sdk hook, then the phone app. The Prime app, the CDK work and
the KeyOS patch all run in parallel to it.

## Stages

### Stage 0: de-risk and set up. Repos edited: none

- Install the Foundation SDK, run `foundation sim`, request a dev unit through the developer portal.
- Ask Foundation two questions: can hackathon teams get the QuantumLink v2 app channel early, and is
  custom firmware allowed on the unit they provide.
- Spike one: compile `secp256k1`, `bls12_381`, `serde_json` and `ciborium` for
  `armv7a-unknown-xous-elf` inside `foundation develop`. Spike two: time one BLS pairing on the
  simulator and on hardware if available. These decide whether the Prime verifiers are viable as designed.
- Stand up devimint from the fedimint repo and `cdk-mintd` with the fake Lightning backend. Run the
  fedimint-sdk examples and `cdk-cli` against them once so the oracle flows are known.
- Exit: both crypto crates compile for the target, timings known, local federation and mint running.

### Stage 1: protocol and Prime-side cores. Repos edited: `prime-ecash-signer`

- Define the CBOR request schema with a version field (`02_ARCHITECTURE.md`, request schema).
- Write `fedimint-signer-core` and `cashu-signer-core` (`02_ARCHITECTURE.md`, Prime-side code).
- Generate golden vectors in `ecash-devtools` with the real fedimint-core and cashu crates; test
  byte-for-byte on the host.
- Build the simulator app: file-bridge transport, review screens for all request kinds, policy limits,
  tap and PIN gates, `[[qr-match-rules]]` for our UR types.
- Exit: every golden vector passes; the simulator app reviews and signs canned requests.

### Stage 2: Fedimint signer boundary. Repos edited: `fedimint` fork, `fedimint-sdk` fork, `ecash-devtools`

- fedimint-client-module: local-or-remote key references on `ClientInput`, a `TransactionSigner` trait,
  transaction finalisation calls it.
- fedimint-mint-client: watch-only note store with paths, xpub spend-key derivation, view-secret blinding
  keys, release of notes by path, reclaim of released notes.
- fedimint-sdk: `SdkBuilder::signer(...)`, view material export, a UniFFI callback interface so Kotlin
  implements the signer, a release call on the ecash facade.
- `ecash-devtools`: a host driver that runs the SDK on a laptop, forwards sign requests to the simulator
  over the file bridge, and pays a devimint invoice end to end.
- Exit, milestone M2: Lightning payment, on-chain withdrawal, ecash send and receive on devimint with the
  simulated Prime signing every spend.

### Stage 3: Cashu signer boundary. Repos edited: `cdk` fork, `prime-ecash-signer`, `ecash-devtools`

- cdk wallet: `SIG_ALL` witness assembly for swap and melt; melt saga reordered so blank change outputs
  exist before signing; a signer callback trait; wallet-wide default output conditions with batch
  rotation; deterministic P2PK nonce and blinding derivation from a view seed; NUT-09 restore over it.
- cdk-ffi: expose the signer callback and the view-seed setup; regenerate Kotlin bindings locally.
- Prime: wire `cashu-signer-core` into the review screen showing mint URL, unit, amount, fee reserve,
  change.
- Exit, milestone M3: melt and swap on `cdk-mintd` with the simulated Prime signing; a `SIG_INPUTS`-only
  mint refused; one run against a public nutshell test mint.

### Stage 4: phone app and hardware over QR. Repos edited: `ecash-phone`, `prime-ecash-signer`

- Kotlin app with the fork-built fedimint-sdk Android package and the local CDK Kotlin bindings, the
  input router from the token table, per-backend balances and history, the signer bridge.
- Transport two: animated UR both ways, hummingbird on the phone, the system scanner plus the UR widgets
  on Prime.
- Sideload the signed app on hardware; run every flow from Stages 2 and 3 through the camera.
- Exit, milestone M4: both backends working on a real Prime over QR.

### Stage 5: Bluetooth. Repos edited: `KeyOS` fork, `ecash-phone`, `prime-ecash-signer`

- KeyOS: a patch adding our request and response messages to `api/quantum-link` and routing in the
  `os/quantum-link` server, modelled on the Tether patch. Handle the missing `api/gpio` by building the
  app from the KeyOS tree with Tether's scripts.
- Build firmware with `just build-all`, flash the dev unit over SAM-BA, keep the brick-recovery steps
  at hand.
- Phone: port the Tether QuantumLink module; pairing by static BLE QR then animated identity QR; identity
  pinning; replay protection.
- Exit, milestone M5: the same flows over Bluetooth, with QR still working on a stock device.

### Stage 6: harden and upstream. Repos edited: all forks

- Negative tests on Prime (`08_VERIFICATION_AND_TESTING.md`).
- Pull requests in this order: fedimint signer trait, fedimint-sdk hook, cdk `SIG_ALL` wallet signing.
  Each is useful to its project on its own, which helps them land.
- Future items, in order: migrate to QuantumLink v2 when KeyOS ships it; Fedimint mint v2 tokens; NFC via
  the applet-registry patch (`10_NFC_FUTURE.md`); feature-gating fedimint-core's network dependencies so
  the device could share the real decoder.

## Running it as a team

Four parallel lanes fit the tree:

| Lane | Owns | Stages |
|---|---|---|
| A | `prime-ecash-signer`, signer cores, review UI | 1, 3 (Prime side), 4, 5 (Prime side) |
| B | `fedimint` and `fedimint-sdk` forks, host driver | 2 |
| C | `cdk` fork and Kotlin bindings | 3 |
| D | `ecash-phone`, KeyOS patch and firmware | 4, 5 |

Rough sizing:

| Stage | Working days | Blocked by |
|---|---|---|
| 0 | 2 | Nothing |
| 1 | 4 | Stage 0 spikes |
| 2 | 5 | Stage 1 schema |
| 3 | 5 | Stage 1 schema; runs beside Stage 2 |
| 4 | 5 | Stages 2 and 3 |
| 5 | 4 | Dev unit; runs beside Stage 4 |
| 6 | ongoing | Stage 4 |

If the hackathon is shorter, cut Stage 3 to receive-only on Cashu and keep Fedimint as the signed-spend
demo, since the Fedimint path has no SDK gaps on the server side.

## What to ask Foundation now

1. A dev unit, with permission to flash custom firmware.
2. Whether a QuantumLink v2 app channel preview exists for hackathon teams, and its timeline in KeyOS.
3. Whether the `api/gpio` SDK packaging gap will be fixed in the next SDK, which would let the app build
   with `quantum-link` from the SDK alone.
4. The security-review checklist for key-handling apps (hello@foundation.xyz).
