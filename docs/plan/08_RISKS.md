# 08. Risks

Ranked by how much they could change the outcome. Each has a mitigation and an owner lane from
`05_EXECUTION_PLAN.md`.

| # | Risk | Likelihood | Impact | Mitigation | Lane |
|---|---|---|---|---|---|
| 1 | No QuantumLink for a sideloaded app; the KeyOS patch needs a dev unit, custom firmware and SAM-BA flashing, with brick risk and waived security guarantees | High | Bluetooth demo slips | Ask Foundation on day one; keep QR/UR as the stock-device path; follow Tether's scripts and the brick-recovery procedure; Stages 0 to 3 are a complete demo without it | C |
| 2 | fedimint-client signer refactor is larger than expected (async build, state machines, recovery, OOB timeout reclaim) | Medium | M2 slips | Start it first; keep the local-signer default so nothing else changes; land the watch-only store before recovery | B |
| 3 | Crypto crates do not build, or BLS is too slow, on `armv7a-unknown-xous-elf` | Low to medium | Prime verifier unviable | Stage 0 spikes before anything else; the Bitcoin app already builds secp256k1 on the device; cap notes per request | A |
| 4 | Target federation runs only the v1 modules (mint v1, wallet v1, LN v1) | Medium | Demo fails on older public federations | First version targets the 0.13 default v2 shape; demo on devimint's v2 shape; add v1 decoders later | A, B |
| 5 | Hash-to-curve or consensus-encoding mismatch on the device | Medium | Wrong decode or wrong signature | Golden vectors from the real crates; byte equality required; fuzzing | A |
| 6 | fedimint-sdk React Native bindings are newer than the Kotlin ones | Medium | Phone app churn | Decide the stack in Stage 0 after a build test; Kotlin fallback | C |
| 7 | fedimint-sdk beta pin and the `pins-agree` CI check | Certain | Fork friction | Local builds only until upstreaming | B |
| 8 | Time: four core stages in a hackathon | High | Scope | Cut-down plan: Stages 0 to 3, QR transport only | all |
| 9 | Security review of a key-handling app | Medium | Credibility | Request Foundation's checklist early; no token logging; never change the app id; PIN gate above a threshold | A |
| 10 | Lost or desynced state between phone and Prime (released paths, pending reviews) | Medium | Stuck funds or double release | Prime keeps a released-path log and the last signed request ids; the phone reconciles via `Status` on every connect; recovery rescans | A, C |
| 11 | Derivation change makes notes unrecoverable by stock fedimint clients | Certain | Lock-in | Document the scheme; recovery needs only xpub, view secret and the scheme version; upstream the scheme | B |
| 12 | NFC two-tap flow fights the NFC server's session timeouts (optional stage) | Medium | Stretch goal slips | Patch the timeouts; design for two taps rather than hold-and-poll; hardware-only testing budgeted | C |
| 13 | Brick during firmware flashing | Low | Lost dev unit time | Follow `DEVELOPMENT.md` recovery; keep one stock device for QR demos | C |
