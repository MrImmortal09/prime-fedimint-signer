# 09. Risks

Ranked by how much they could change the outcome. Each has a mitigation and an owner lane from
`06_EXECUTION_PLAN.md`.

| # | Risk | Likelihood | Impact | Mitigation | Lane |
|---|---|---|---|---|---|
| 1 | No QuantumLink for a sideloaded app; the KeyOS patch needs a dev unit, custom firmware and SAM-BA flashing, with brick risk and waived security guarantees | High | Bluetooth demo slips | Ask Foundation on day one; keep QR/UR as the stock-device path; follow Tether's scripts and the brick-recovery procedure | D |
| 2 | fedimint-client signer refactor is larger than expected (async build, state machines, recovery) | Medium | M2 slips | Start it first; keep the local-signer default so nothing else changes; land the watch-only store before recovery | B |
| 3 | CDK wallet lacks `SIG_ALL` spending and the API churns weekly | High | M3 slips | Fork at `v0.18.3`; implement the melt saga reorder first; cut Cashu to receive-only if needed | C |
| 4 | Crypto crates do not build or are too slow on `armv7a-unknown-xous-elf` | Low to medium | Prime verifiers unviable | Stage 0 spikes before anything else; the Bitcoin app already builds secp256k1 on the device | A |
| 5 | Target federation runs LN v1 only | Medium | Lightning demo fails on public federations | Add the LN v1 output type to the decoder; note the fedimint-sdk LN v1 error on success | A, B |
| 6 | `SIG_ALL` melt message differs between mint implementations | Medium | Cashu melt rejected on some mints | Test each mint; prefer cdk-mintd and nutshell | C |
| 7 | Linkability from P2PK locks disappoints Cashu purists | Certain | Perception | Batch key rotation; document the trade-off honestly | C |
| 8 | Hash-to-curve or message formatting mismatch on the device | Medium | Wrong signatures | Golden vectors from the real crates; byte equality required | A |
| 9 | fedimint-sdk beta pin and `pins-agree` CI | Certain | Fork friction | Local builds only until upstreaming | B |
| 10 | Time: five stages in a hackathon | High | Scope | The cut-down plan: Fedimint signed spends plus Cashu receive-only, QR transport | all |
| 11 | Security review of a key-handling app | Medium | Credibility | Request Foundation's checklist early; keep the no-logging rules; never change the app id | A |
| 12 | Lost or desynced state between phone and Prime (released paths, batch keys) | Medium | Stuck funds | Prime keeps a log of released paths and signed requests; the phone reconciles on connect; recovery rescans | A, D |
