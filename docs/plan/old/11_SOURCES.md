# 11. Sources

All checked on 2026-10-30.

## Foundation and Passport Prime

- KeyOS repository: https://github.com/Foundation-Devices/KeyOS (commits through 2026-10-21; `api/quantum-link/manifest.toml`, `api/nfc/manifest.toml`, `os/nfc/src/atsama5d2.rs`, `DEVELOPMENT.md`)
- Foundation-Devices organisation: https://github.com/orgs/Foundation-Devices/repositories
- QuantumLink v2 crates: https://github.com/Foundation-Devices/quantum-link
- Foundation API (GSTP): https://github.com/Foundation-Devices/foundation-api
- Tether WDK proof of concept: https://github.com/Foundation-Devices/passport-tether-wdk (`docs/PROTOCOL.md`, `docs/QUANTUM_LINK.md`, `docs/ARCHITECTURE.md`)
- Cake signer: https://github.com/Foundation-Devices/passport-cake-signer
- Agents wallet: https://github.com/Foundation-Devices/passport-agents-wallet
- Proof Authority: https://github.com/Foundation-Devices/passport-proof-authority
- ST25R95 Rust driver: https://docs.rs/crate/st25r95/0.1.0 and https://github.com/Foundation-Devices/st25r95
- NFC Forum tags crate: https://github.com/Foundation-Devices/nfc-forum-tags
- Hummingbird (Java UR): https://github.com/Foundation-Devices/hummingbird
- Developer capabilities matrix: https://docs.foundation.xyz/developers/capabilities/
- Developers page: https://foundation.xyz/developers
- App showcase: https://foundation.xyz/app-showcase
- Keys app docs (NFC with phones): https://docs.foundation.xyz/prime/prime-apps/keys/
- QuantumLink blog post: https://foundation.xyz/blog/from-qr-air-gapping-to-quantumlink-what-comes-next
- Blockchain Commons QuantumLink and GSTP: https://developer.blockchaincommons.com/assets/pdfs/202503-quantumlink.pdf
- KeyOS 1.4.0 beta thread: https://community.foundation.xyz/t/keyos-version-1-4-0-beta1-now-available-for-testing/1200
- Passport Prime launch coverage: https://idtechwire.com/foundation-raises-6-4m-and-launches-passport-prime-device/
- Product page: https://foundation.xyz/products/passport-prime
- ST25R95 product page: https://www.st.com/en/nfc/st25r95.html
- ST community on ST25R95 peer-to-peer: https://community.st.com/st25-nfc-rfid-tags-and-readers-54/reading-emulated-cards-with-st25r95-st95hf-9958

## Fedimint

- fedimint repository: https://github.com/fedimint/fedimint (inspected: `fedimint-core/src/transaction.rs`, `fedimint-core/Cargo.toml`, `fedimint-core/src/base32.rs`, `fedimint-core/src/invite_code.rs`, `fedimint-client-module/src/transaction/builder.rs`, `fedimint-client/src/client.rs`, `modules/fedimint-mint-client/src/{lib.rs,output.rs}`, `modules/fedimint-mint-common/src/lib.rs`, `modules/fedimint-lnv2-common/src/contracts.rs`, `modules/fedimint-wallet-common/src/lib.rs`, `crypto/tbs/src/lib.rs`)
- fedimint-sdk repository and Rust crate: https://github.com/fedimint/fedimint-sdk (`rust/fedimint-sdk/README.md`, `Cargo.toml`, `src/ecash.rs`, `src/types/notes.rs`, `src/sdk.rs`, `CHANGELOG.md`)
- LN v1 preimage issue: https://github.com/fedimint/fedimint/issues/8969
- Fedimint wallets: https://fedimint.org/wallets
- Fedimint FAQ: https://fedimint.org/users/faq
- Ecash App (Flutter): https://github.com/fedimint/ecash-app
- Fedi: https://github.com/fedixyz/fedi and https://unchainedcrypto.com/fedi-unveils-production-version-of-its-bitcoin-super-app/
- Harbor: https://harbor.cash
- Vipr: https://vipr.cash/

## Cashu

- NUT-00 token formats: https://github.com/cashubtc/nuts/blob/main/00.md
- NUT-07 state check: https://github.com/cashubtc/nuts/blob/main/07.md
- NUT-09 restore: https://github.com/cashubtc/nuts/blob/main/09.md
- NUT-11 P2PK and `SIG_ALL`: https://github.com/cashubtc/nuts/blob/main/11.md
- NUT-12 DLEQ: https://github.com/cashubtc/nuts/blob/main/12.md
- NUT-13 deterministic secrets: https://github.com/cashubtc/nuts/blob/main/13.md
- NUT-18 payment requests: https://github.com/cashubtc/nuts/blob/main/18.md
- NUT-CCP Cashu Card Protocol: https://github.com/cashucard/nut
- Cashu card applet: https://github.com/cashucard/cashucard
- Flash cashu-card and cashu-javacard: https://github.com/lnflash/cashu-card and https://github.com/lnflash/cashu-javacard
- Cashu Development Kit: https://github.com/cashubtc/cdk (v0.18.3; inspected `crates/cashu/src/nuts/nut11/mod.rs`, `crates/cashu/src/nuts/nut06.rs`, `crates/cdk/src/wallet/melt/saga/mod.rs`, `crates/cdk/src/wallet/util.rs`, `crates/cdk/src/mint/{swap,melt}/tests/`, `crates/cdk-ffi/README.md`)
- cdk-ffi: https://docs.rs/crate/cdk-ffi/latest
- cdk-kotlin: https://github.com/cashubtc/cdk-kotlin
- cdk-swift: https://github.com/cashubtc/cdk-swift
