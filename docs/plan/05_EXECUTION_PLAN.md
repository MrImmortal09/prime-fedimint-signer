# 05. Execution plan

## Repositories

| Repo | Role in the project | Do we edit it? | How we consume it |
|---|---|---|---|
| `fedimint/fedimint` | Client crates: transaction builder, mint client | Yes. Fork from tag `v0.13.0-beta.1`, keep the version string | `[patch.crates-io]` in fedimint-sdk so the exact-version pins still resolve |
| `fedimint/fedimint-sdk` | Rust SDK, React Native, Kotlin and Swift bindings | Yes. Fork | Build the React Native package or Android SDK from the fork with the patched fedimint crates |
| `Foundation-Devices/KeyOS` | OS, SDK, QuantumLink service, NFC service | Yes. One patch for QuantumLink (Stage 4), optionally one for NFC (Stage 5) | Build firmware from the tree, flash a dev unit |
| `Foundation-Devices/passport-tether-wdk` | Reference for the KeyOS patch, firmware scripts, phone QuantumLink Expo module | No | Copy its patterns and scripts; use the Expo module as-is |
| `Foundation-Devices/quantum-link`, `foundation-api` | QuantumLink protocol crates for the phone side | No | Vendored inside Tether's module |
| `Foundation-Devices/hummingbird` | Java UR implementation | No | Only if the phone app is Kotlin |
| **new** `prime-fedimint-signer` | The KeyOS app and its workspace: `protocol`, `fedimint-signer-core`, `policy`, UI | Ours | Scaffolded with `foundation new`, sideloaded as a signed `.app` |
| **new** `fedimint-phone` | The phone wallet: fedimint-sdk bindings, input router, transports, signer bridge | Ours | Built against the fork artifacts |
| **new** `fedimint-signer-devtools` | devimint scripts, simulator file bridge driver, golden vector generator | Ours | Host only |

Not edited and not needed: the Fedimint federation code.

## Dependency tree

Arrows read "needs". Leaves at the bottom can start on day one.

```
M5  NFC tap-to-approve (optional stretch)
├── KeyOS NFC applet-registry patch                   [KeyOS fork, dev unit]
├── Phone NFC reader transport                        [fedimint-phone]
└── (same firmware build as M4)

M4  Bluetooth demo on hardware
├── KeyOS QuantumLink patch + custom firmware         [KeyOS fork, dev unit]
├── Phone QuantumLink transport                       [fedimint-phone, Tether's Expo module]
└── M3  Full flow on hardware over QR/UR
    ├── Phone app                                     [fedimint-phone]
    │   ├── fedimint-sdk bindings with signer hook    [fedimint-sdk fork]
    │   │   └── signer trait + watch-only mint        [fedimint fork]
    │   ├── protocol crate                            [prime-fedimint-signer/protocol]
    │   ├── input router                              [fedimint-phone]
    │   └── UR transport                              [fedimint-phone]
    └── Prime app                                     [prime-fedimint-signer]
        ├── fedimint-signer-core  <- golden vectors from fedimint-core on the host [devtools]
        ├── protocol crate
        ├── policy + review screens
        └── transports: file bridge -> QR/UR -> QuantumLink -> (NFC)

M2 (Fedimint end to end on the simulator) runs with a host test driver over the file bridge before the
phone app exists.
```

Critical path: fedimint fork, then fedimint-sdk hook, then the phone app. The Prime app and the KeyOS
patches run in parallel to it.

## Stages

### Stage 0: de-risk and set up. Repos edited: none

- Install the Foundation SDK, run `foundation sim`, request a dev unit through the developer portal.
- Ask Foundation: can hackathon teams get the QuantumLink v2 app channel early; is custom firmware
  allowed on the unit they provide; is an NFC applet registry for third-party apps on their roadmap.
- Spike one: compile `secp256k1`, `bls12_381` and `ciborium` for `armv7a-unknown-xous-elf` inside
  `foundation develop`. Spike two: time one BLS pairing on the simulator and on hardware if available.
  These decide whether the Prime verifier is viable as designed.
- Stand up devimint and run the fedimint-sdk examples against it once so the oracle flows are known.
- Pick the phone stack (React Native with Expo recommended, Kotlin fallback) and confirm the fedimint
  bindings build locally for Android.
- Exit: crypto crates compile for the target, timings known, local federation running, phone bindings
  building.

### Stage 1: protocol and Prime-side core. Repos edited: `prime-fedimint-signer`, `fedimint-signer-devtools`

- Define the CBOR request schema with a version field (`02_ARCHITECTURE.md`, request schema).
- Write `fedimint-signer-core` (`02_ARCHITECTURE.md`, Prime-side code).
- Generate golden vectors with the real fedimint-core, mint-common, lnv2-common, ln-common and
  wallet-common crates; test byte-for-byte on the host.
- Build the simulator app: file-bridge transport, review screens for all request kinds, policy limits,
  tap and PIN gates, `[[qr-match-rules]]` for our UR types, a released-path log in `AppData`.
- Exit: every golden vector passes; the simulator app reviews and signs canned requests.

### Stage 2: Fedimint signer boundary. Repos edited: `fedimint` fork, `fedimint-sdk` fork, `fedimint-signer-devtools`

- fedimint-client-module: local-or-remote key references on `ClientInput`, a `TransactionSigner`
  trait, transaction finalisation calls it.
- fedimint-mint-client: watch-only note store with paths, xpub spend-key derivation, view-secret
  blinding keys, release of notes by path, reclaim of released notes, refund keys from the refund branch.
- fedimint-sdk: `SdkBuilder::signer(...)`, view material export, a UniFFI callback interface so the
  phone implements the signer, release and reclaim calls on the ecash facade.
- devtools: a host driver that runs the SDK on a laptop, forwards sign requests to the simulator over the
  file bridge, and pays a devimint invoice end to end.
- Exit, milestone M2: Lightning payment, on-chain withdrawal, ecash send, receive and reclaim on devimint
  with the simulated Prime signing every spend.

### Stage 3: phone app and hardware over QR. Repos edited: `fedimint-phone`, `prime-fedimint-signer`

- Phone app with the fork-built fedimint bindings, the input router, balance and history, the signer
  bridge, and the UR transport both ways.
- On Prime, the system scanner plus the UR widgets, driven by the same request handling as the file
  bridge.
- Sideload the signed app on hardware; run every flow from Stage 2 through the camera.
- Exit, milestone M3: the full flow working on a real Prime over QR.

### Stage 4: Bluetooth. Repos edited: `KeyOS` fork, `fedimint-phone`, `prime-fedimint-signer`

- KeyOS: a patch adding our request and response messages to `api/quantum-link` and routing in the
  `os/quantum-link` server, modelled on the Tether patch. Handle the missing `api/gpio` by building the
  app from the KeyOS tree with Tether's scripts.
- Build firmware with `just build-all`, flash the dev unit over SAM-BA, keep the brick-recovery steps at
  hand.
- Phone: Tether's QuantumLink Expo module; pairing by static BLE QR then animated identity QR; identity
  pinning; replay protection.
- Exit, milestone M4: the same flows over Bluetooth, with QR still working on a stock device.

### Stage 5 (optional stretch): NFC tap-to-approve. Repos edited: `KeyOS` fork, `fedimint-phone`, `prime-fedimint-signer`

- KeyOS: the applet-registry patch in the NFC service (`09_NFC.md`), built into the same firmware as
  Stage 4.
- Prime: an APDU handler that reassembles our CBOR requests and serves the two-tap flow.
- Phone: Android reader mode with our AID.
- Exit, milestone M5: approve a Lightning payment by tapping Prime on the phone, twice.

### Stage 6: harden and upstream. Repos edited: all forks

- Negative tests on Prime (`07_VERIFICATION_AND_TESTING.md`).
- Pull requests in this order: fedimint signer trait, fedimint watch-only mint store, fedimint-sdk hook.
  Each is useful to the project on its own.
- Future items, in order: migrate to QuantumLink v2 when KeyOS ships it; the v1 module decoders (mint,
  wallet, LN v1) for older federations; feature-gating fedimint-core's network dependencies so the device could share the real
  decoder; Cashu per `old/04_CASHU_NOTES.md` if ever wanted.

## Running it as a team

Three parallel lanes fit the tree:

| Lane | Owns | Stages |
|---|---|---|
| A | `prime-fedimint-signer`, signer core, review UI | 1, 3 (Prime side), 4 and 5 (Prime side) |
| B | `fedimint` and `fedimint-sdk` forks, host driver | 2, 6 |
| C | `fedimint-phone`, KeyOS patches and firmware | 3, 4, 5 |

Rough sizing:

| Stage | Working days | Blocked by |
|---|---|---|
| 0 | 2 | Nothing |
| 1 | 4 | Stage 0 spikes |
| 2 | 5 | Stage 1 schema |
| 3 | 4 | Stage 2 |
| 4 | 4 | Dev unit; runs beside Stage 3 |
| 5 | 3 | Stage 4 firmware; optional |
| 6 | ongoing | Stage 3 |

If the hackathon is shorter, ship Stages 0 to 3: the signer working on a real Prime over QR is a complete
demo on its own. Bluetooth and NFC are additions, not foundations.

## What to ask Foundation now

1. A dev unit, with permission to flash custom firmware.
2. Whether a QuantumLink v2 app channel preview exists for hackathon teams, and its timeline in KeyOS.
3. Whether the `api/gpio` SDK packaging gap will be fixed in the next SDK.
4. Whether an NFC applet registry for third-party AIDs is planned; the TODO in `os/nfc` suggests it is.
5. The security-review checklist for key-handling apps (hello@foundation.xyz).
