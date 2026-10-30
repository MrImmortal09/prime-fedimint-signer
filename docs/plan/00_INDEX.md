# Passport Prime Fedimint signer: planning documents

Scope as of 2026-10-30 (evening revision): **Fedimint only**. The earlier Fedimint + Cashu plan is in
`old/` and stays there as the record of what adding Cashu would take.

| File | What it holds |
|---|---|
| `PASSPORT_PRIME_KEYOS_REFERENCE.md` | Platform reference: device, KeyOS, SDK, permissions, service APIs, limits |
| `01_RESEARCH_FINDINGS.md` | What exists today: Prime integrations, Fedimint client internals, gaps, why Cashu was dropped |
| `02_ARCHITECTURE.md` | The design: roles, key split, input routing, transports, request schema, what Prime verifies |
| `03_TRANSACTION_FLOWS.md` | Step-by-step flows for setup, receiving, paying, withdrawing, sending ecash, failures |
| `04_SERVER_COMPATIBILITY.md` | Why no federation changes are needed, what a federation must run, caveats |
| `05_EXECUTION_PLAN.md` | Repositories, dependency tree, stages with repo edits, team lanes, sizing |
| `06_UPSTREAM_CHANGES.md` | Exact changes per repository with file pointers |
| `07_VERIFICATION_AND_TESTING.md` | Oracles, golden vectors, negative tests, test matrix |
| `08_RISKS.md` | Ranked risks and mitigations |
| `09_NFC.md` | NFC as an optional stretch stage: how easy it is, the two-tap design, the patch |
| `10_DECISIONS.md` | Decision log with reasons |
| `11_SOURCES.md` | Every source used |

## The project in one paragraph

A KeyOS app that turns Passport Prime into a hardware signer for Fedimint ecash, paired with a phone app
that is the networked wallet. The phone holds view material and the public note data, builds every
operation, and talks to the federation. Prime holds the spend keys, decodes every spend from raw bytes,
shows it on its trusted display, and signs after a tap. Receiving never involves Prime. This mirrors how
every existing Prime integration works (Envoy, Tether WDK, Cake signer, Agents wallet) and would be the
first external-signer design for Fedimint.

## Decisions already made

See `10_DECISIONS.md` for the reasons. In short:

- Prime is a signer and trusted display, not a full wallet.
- Fedimint only. Cashu is dropped from the hackathon; the input router still recognises Cashu tokens and
  says they are unsupported.
- Transport: QuantumLink Bluetooth is primary through a KeyOS patch; animated QR/UR and the simulator
  file bridge are the fallbacks and the stock-device path.
- NFC is an optional stretch stage after QR works, not a dependency of anything.
- No changes to the Fedimint federation protocol. All changes are client-side and meant to be upstreamed.
- Phone app: Android first. React Native with Expo for maximum reuse, Kotlin as the fallback.
- Module shape: mint v2, wallet v2 and LNv2 first, because fedimint 0.13 federations run that shape
  by default. The v1 modules (mint, wallet, LN v1) follow later. Corrected in Phase 0, see D8.
- Ecash tokens only ever carry keys from a hardened outgoing branch (D11). Federation import is
  trust-on-first-use from the paired phone (D12). Receives stay phone-only (D13).
