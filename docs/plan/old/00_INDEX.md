# Passport Prime ecash signer: planning documents

Compiled 2026-10-30 from a research and planning session. These files cover the research, the
recommended design, the transaction flows, the Cashu specifics, server compatibility, the execution
plan with a dependency tree, the exact upstream changes, testing, risks, NFC as future work, and
sources. Read them in order the first time; afterwards each file stands on its own.

| File | What it holds |
|---|---|
| `PASSPORT_PRIME_KEYOS_REFERENCE.md` | Platform reference: device, KeyOS, SDK, permissions, service APIs, limits |
| `01_RESEARCH_FINDINGS.md` | What exists today: Prime integrations, Fedimint and Cashu client internals, gaps |
| `02_ARCHITECTURE.md` | The recommended design: roles, key split, token routing, transports, verification, alternatives |
| `03_TRANSACTION_FLOWS.md` | Step-by-step flows for every operation on both backends, plus failure cases |
| `04_CASHU_NOTES.md` | Cashu-specific design, CDK gaps, problems to expect, mint requirements |
| `05_SERVER_COMPATIBILITY.md` | Why no federation or mint changes are needed, and what servers must support |
| `06_EXECUTION_PLAN.md` | Repositories, dependency tree, stages with repo edits, team lanes, sizing |
| `07_UPSTREAM_CHANGES.md` | Exact changes per repository with file pointers |
| `08_VERIFICATION_AND_TESTING.md` | Oracles, golden vectors, negative tests, test matrix |
| `09_RISKS.md` | Ranked risks and mitigations |
| `10_NFC_FUTURE.md` | NFC: what the hardware and OS can do today, and how to add tap-to-pay later |
| `11_SOURCES.md` | Every source used |

## The project in one paragraph

A KeyOS app that turns Passport Prime into a hardware signer for ecash, covering both Fedimint and
Cashu, paired with a phone app that is the networked wallet. The phone holds "view" material and the
public note and proof data, builds every operation, and talks to federations and mints. Prime holds the
spend keys, decodes every spend from raw bytes, shows it on its trusted display, and signs after a tap.
Receiving never involves Prime. This mirrors how every existing Prime integration works (Envoy, Tether
WDK, Cake signer, Agents wallet) and would be the first external-signer design for either ecash system.

## Decisions already made

- Prime is a signer and trusted display, not a full wallet. A full client on Prime is not feasible.
- Both Fedimint (fedimint-sdk Rust crate) and Cashu (Cashu Development Kit) are in scope.
- Transport: QuantumLink Bluetooth is primary, obtained through a KeyOS patch like Foundation's own
  Tether proof of concept. Animated QR/UR and the simulator file bridge are fallbacks and the path for
  stock retail devices.
- NFC is deferred to future work and documented in `10_NFC_FUTURE.md`.
- No changes to the Fedimint federation or the Cashu mint protocols. All changes are client-side and
  meant to be upstreamed.
- Phone app in Kotlin on Android first, since both SDKs ship Kotlin bindings and the only proven
  QuantumLink phone module is Android.
