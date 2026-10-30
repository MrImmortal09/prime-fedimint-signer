# Prime Ecash UI implementation

Implemented on repository commit `289f7fdae532b89e38491f7bed5340917c05768c`.

## What changed

The existing SwiftUI and Slint applications now share a charcoal, copper, and warm-white identity. Cryptographic code, transaction protocols, UniFFI bindings, the application ID, and the QR transport remain intact.

### Passport Prime

- `prime-app/resources/theme.json` and `prime-app/src/theme.rs`: shared colors and rounded controls through the KeyOS theme system. Runtime semantic overrides preserve brand colors and dark button text where SDK JSON normalization supplies base defaults. Keep the JSON and runtime overrides in sync.
- `prime-app/ui/pages/page.slint`: Ecash identity, federation cards, pairing guidance, scan/retry action, settings navigation. No live balance on the offline device.
- `prime-app/ui/pages/review/page.slint`: large amount, scrollable device-generated details and warnings, persistent Reject action, KeyOS SlideToConfirm approval. PIN verification still runs through the existing system flow. UI errors remain visible on review.
- `prime-app/ui/pages/result/page.slint`: response/token distinction, animated QR support, decision notice, explicit explanation that the phone still needs to process the result.
- `prime-app/ui/pages/settings/page.slint`: scrollable federation and policy settings, wrapping descriptions, accessible switch labels, visible save errors.

### iPhone

- `WalletStyle.swift`: reusable panels, buttons, step rows, notices, semantic color tokens, and system typography.
- `JoinView.swift`: welcome and guided federation/Prime setup with honest availability and federation trust copy.
- `HomeView.swift`: real available balance, pairing information, Send/Receive actions, stored ecash sends, recent activity and operation details. Forgetting an ecash tracking record requires confirmation and explains that it does not reclaim funds.
- `SendView.swift`: review sheet before the existing backend operation. The authorization exchange opens after that sheet dismisses. Final details and fees are reviewed on Prime. Payment results appear only after the backend returns. Ecash token possession and Bitcoin submission are distinguished from recipient redemption/confirmation.
- `ReceiveView.swift`: receive-without-Prime explanation and shared appearance, preserving the existing invoice, ecash, and deposit operations.
- `PrimeExchangeView.swift`: two-stage request/response journey, scan progress, recovery instructions, and pause/resume for animated request QR codes.
- `Components.swift`: high-contrast QR presentation, larger scan/paste targets, clearer busy overlay.
- `RootView.swift`: shared appearance, opening-error retry, labeled Settings control, existing bridge and error presentations.
- `SettingsView.swift`: federation/gateway settings, QR pairing explanation, privacy/trust information and supported feature states.
- `WalletModel.swift`: prevent overlapping foreground wallet actions and clear the opening error before retry.
- `Format.swift`: reject amount inputs that would overflow millisatoshi conversion.

### Simulator test driver

`crates/simdrive/src/lib.rs`, `crates/simdrive/src/hand.rs`, and `tools/sim-e2e/src/main.rs` now use a deliberate swipe for approval and recognize the new copper slider/charcoal Reject layout. This preserves the existing test scenarios after replacing the tap-to-approve control. The simulator scenario verified the actual swipe interactions. The detector accepts RGB and BGR screenshot channel ordering.

## Verification

- Rust workspace: **128 tests passed**, 0 failures, using compact build settings.
- Swift frontend parsing: passed for all authored app views, models, support files, and app entry point. This is syntax verification, not an iOS build or SwiftUI type check.
- Amount parser: passed positive/zero/negative/fractional input and UInt64 overflow boundary checks using the actual amount helper.
- Slint viewer `--check`: passed for the complete app import/router graph.
- Source formatting: `git diff --check` passed; Rust changes formatted with `cargo fmt --all`.
- Color checks: primary text/background 17.34:1, secondary text/surface 6.93:1, button text/copper 7.87:1, slider hint/track 5.27:1.
- Slint layout QA: rendered and inspected at 480 × 800 using the SDK Montserrat fonts. Reviewed home, review, long destination + warning + PIN review, settings, and result message layout. Fixed an expanding approval slider, clipped settings text, and overlapping slider hint.

The included review images use clearly identified QA fixture values. They render the actual implemented components; they are not evidence of a live payment. The standalone viewer does not bind the Rust QR-render callback. Separately, the runtime simulator scenario decoded the displayed ecash QR and checked it against the expected token. Its renderer also reports that the SDK slider arrow SVG cannot be decoded; the text and native slide control remain visible.

## Availability and limits

Cashu is not implemented in the current core and is labeled unavailable. The mobile wallet remains limited to one federation. No fiat quote service was added. History exposes only title, module, and time; amounts and final payment status are not invented. Stored ecash send records do not prove redemption status. No Bluetooth/NFC integration or recovery guarantees were introduced.

Xcode is absent (the active developer directory contains only Command Line Tools). An iOS build, iPhone simulator/device run, VoiceOver, Dynamic Type, landscape, and camera checks remain outstanding. The Rust iOS framework also needs to be built with the existing script on an Xcode-equipped machine.

Initial simulator attempts encountered disk exhaustion while fetching the SDK environment and compiling dependencies. Only build outputs created for this task were removed; compact compilation was then used. The SDK's normal Nix environment failed during dependency setup, and stable Rust could not compile the SDK's nightly-only server macro.

The KeyOS app built successfully with installed `nightly-2026-04-01`, compact build settings, and Foundation SDK 1.1.0, then ran in the actual KeyOS simulator. The existing golden scenario passed **19 checks**, and **4 further checks after restart**. These verified pairing, slide approvals, rejection, signed transactions, refunds, a PIN-gated withdrawal, ecash staging/release, exact on-screen QR token decoding, reclaim, replay handling, malformed input, and persisted federation/ledger state.

The first scenario attempt stopped at a first-run system-dialog permission prompt. After allowing the prompt in the simulator, the complete fresh scenario and restart check passed. Tests used a known fixture seed in a workspace bridge directory; no hardware or real network payment was used. The simulator was stopped after testing.

SDK screenshot exports reverse red/blue channel ordering: configured copper RGB (214,154,120) appears as RGB (120,154,214), and background channels are likewise reversed. Rendered source QA images show the specified palette; runtime screenshots are preserved unmodified. Actual hardware color parity still needs verification. No reverse-color compensation was applied to the product theme.

The full phone-wallet/federation integration script and the physical camera request/response loop were not run. The successful golden scenario uses the simulator file bridge for requests and verifies the ecash QR on the result screen; it does not prove an end-to-end iPhone camera journey.

## Next verification on a fully configured machine

1. Repeat `scripts/sim-e2e.sh` in the normal SDK shell, and run `scripts/sim-wallet-e2e.sh` for the separate wallet/federation integration suite. Add a PIN-cancellation check; the successful scenario covered PIN approval.
2. Run `scripts/fedimint-checkout.sh`, `scripts/ios-core.sh`, and build the existing Xcode project for an iPhone.
3. Verify the full camera journey on both devices: prepare ecash → scan request on Prime → review and slide → scan response on phone → actual backend result.
4. Check large Dynamic Type, VoiceOver, small iPhones, landscape, long federation names, malformed QR responses, failed payments/refunds, and interrupted exchanges.

## References inspected

- https://foundation.xyz/app-showcase
- https://foundation.xyz/app-showcase/spark-signer
- https://foundation.xyz/app-showcase/liana-signer
- https://foundation.xyz/app-showcase/cake-signer
- https://docs.foundation.xyz/

Actual Spark, Liana, and Cake screenshots were downloaded from the showcase for visual reference and inspected. Local SDK 1.1.0 CLI, migration, theme, and native control sources guided implementation.
