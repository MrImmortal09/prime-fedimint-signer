# Passport Prime / KeyOS — Developer Reference

Compiled 2026-10-30 from foundation.xyz, docs.foundation.xyz, the Foundation community forum, and the
`Foundation-Devices/KeyOS` GitHub repo (commit `4ea7bc9`, 2026-10-13). Everything an app developer needs to
build for the device, in one file. Section 10 (what third-party apps may actually call) and Section 23
(limits and gotchas) are the two most important sections for scoping a hackathon project.

---

## Table of contents

1. [Hackathon quick start](#1-hackathon-quick-start)
2. [The device: Passport Prime](#2-the-device-passport-prime)
3. [KeyOS architecture](#3-keyos-architecture)
4. [The Foundation SDK](#4-the-foundation-sdk)
5. [Install and environment](#5-install-and-environment)
6. [CLI reference](#6-cli-reference)
7. [App project layout](#7-app-project-layout)
8. [app-config.toml, Cargo.toml, build.rs](#8-app-configtoml-cargotoml-buildrs)
9. [Permissions model](#9-permissions-model)
10. [What third-party apps can call (per service)](#10-what-third-party-apps-can-call-per-service)
11. [Filesystem scopes](#11-filesystem-scopes)
12. [Service API quick reference](#12-service-api-quick-reference)
13. [System navigation helpers (QR scanner, file picker, PIN)](#13-system-navigation-helpers)
14. [QR match rules (register your app as a QR handler)](#14-qr-match-rules)
15. [UI: Slint and the `@ui` component library](#15-ui-slint-and-the-ui-component-library)
16. [Starter code (templates)](#16-starter-code-templates)
17. [Multi-page routing](#17-multi-page-routing)
18. [Theming](#18-theming)
19. [Icons, images, fonts, i18n](#19-icons-images-fonts-i18n)
20. [Reference apps in the KeyOS repo](#20-reference-apps-in-the-keyos-repo)
21. [Testing: simulator, preview, hardware, MCP server](#21-testing-simulator-preview-hardware-mcp-server)
22. [Signing and distribution](#22-signing-and-distribution)
23. [Limits and gotchas](#23-limits-and-gotchas)
24. [App showcase and ideas](#24-app-showcase-and-ideas)
25. [Support, community, developer units](#25-support-community-developer-units)
26. [Links and sources](#26-links-and-sources)

---

## 1. Hackathon quick start

Apps are **Rust binary crates** with a **Slint** UI, running sandboxed on **KeyOS**. You do not need a device
to start: the simulator runs the full KeyOS stack on your laptop. Hardware is only needed for NFC, secure
element, Bluetooth and camera testing.

```bash
# 1. Install the SDK (macOS Apple Silicon / Intel, Linux x86_64 / aarch64, Windows via WSL2)
curl -sSfL https://sdk.foundation.xyz/latest/install.sh | bash
# Needs Nix with flakes. If missing:
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install

# 2. Enter the SDK shell and check readiness
foundation develop
foundation doctor

# 3. Scaffold, run in simulator
foundation new my-app              # templates: default-app (default), multi-page-app, kitchen-sink
cd my-app
foundation sim                     # build + launch hosted simulator
foundation preview ui/app.slint    # UI-only live preview (hot reload)

# 4. Run on a real Passport Prime (one-time signing setup)
#    Device: unlock -> enable USB -> Settings > Apps > Developer Mode ON -> connect USB-C
foundation cert gen "My Publisher"       # creates ~/.foundation/signing/My Publisher/
foundation cert install "My Publisher"   # trust the cert on the device (interactive confirm)
foundation sideload                      # build, sign, upload, launch
foundation logs                          # stream device logs

# 5. Ship a single installable file
foundation pack                          # -> target/keyos/<app-name>.app (install from Settings > Apps > Install App)
```

Inside the project, an `AGENTS.md` and `.mcp.json` are pre-wired so Claude Code / Codex can drive the
simulator and a USB-connected device (screenshots, taps, logs). Run `foundation skills` to install the SDK's
agent skills (`/foundation-cli`, `/foundation-new-page`, `/foundation-localize`, `/foundation-sdk-migrate-newest`).

Hackathon-feasible app shapes in SDK 1.1.0 (see Section 10 for the evidence):

- Anything **offline on the device**: notes, password/secret vaults, calculators, viewers, editors, games,
  tools over encrypted per-app storage (`AppData`) or the user partition.
- **QR in / QR out** workflows: open the system QR scanner (QR, UR, animated UR), show QR codes with the
  built-in `Qrcode` widgets; register `[[qr-match-rules]]` so the Launcher's scanner opens your app.
- **File workflows** over USB mass storage / Airlock (user grants on first use).
- **Crypto apps** using the per-app deterministic seed (`GetAppSeed`), hardware TRNG (`GetRandom`),
  hardware AES/SHA/HMAC/Shamir via `os/crypto`, plus any pure-Rust crate (secp256k1, ed25519, BIP39, etc).
- Haptics + RGB LED feedback, battery status, device name/locale/theme.

Not available to third-party apps today: network (by design, no IP stack), raw USB/HID (coming),
NFC read/write (Foundation-signed only), a generic QuantumLink Bluetooth data channel (only status/rate
subscriptions are open; the Envoy wallet-sync channel is Foundation-signed), the master seed/PIN (Foundation-only).

---

## 2. The device: Passport Prime

**Passport Prime** by Foundation Devices, Inc. Tagline: "The keys to your digital life, in your hand."
Marketed as "the world's first Human Authority Hardware": a hardware wallet that grew into a general secure
personal device with an app platform. Launched publicly May 2026 alongside a $6.4M funding round.

### Hardware

| Spec | Value |
|---|---|
| Display | 3.5" IPS colour touchscreen, Gorilla Glass. Framebuffer **480 × 800** px (portrait) |
| Application processor | Microchip **SAMA5D27** (ARM Cortex-A5, 500 MHz) security processor |
| Secure element | Microchip **ATECC608C** |
| RAM | 128 MB (physical RAM is encrypted) |
| Storage | 60 GB eMMC total, ~50 GB user-accessible, hardware encrypted |
| Battery | 1100 mAh Li-ion, integrated, USB-C charging |
| Connectivity | USB-C (power + data), NFC (Keycards), camera (QR/UR scanning), **QuantumLink** post-quantum Bluetooth (separate BT chip, CRYSTALS-Kyber) |
| No Wi-Fi, no IP stack | Intentionally offline |
| Other | Home button, RGB LEDs, haptics, splash resistant, active tamper protection |
| Body | Anodised aluminium chassis, 55.5 × 104.8 × 11 mm, 93 g |
| Colours | Midnight Bronze, Arctic Copper |
| Assembly | USA, ITAR facility |
| In the box | Passport Prime, Keycards with Faraday sleeves, booklet, cable |
| Price | ₹34,100 shown on the India storefront (region-dependent) |

### Security model (user-facing)

- PIN or password; auto-erase after 10 failed PIN attempts; lockout options.
- Security processor + secure element; seeds never leave the SE unencrypted.
- "Evil maid proof", plausible deniability (passphrases), **security words** (anti-phishing words shown at PIN entry).
- Supply-chain integrity check on first boot (device key generated in SE, pubkey registered with Foundation).
- Third-party audit by Keylabs. Open-source hardware and software; reproducible builds.
- Backups: **Keycards** (encrypted NFC cards) and **Magic Backups** via the Envoy companion app.
- "Full Stateless Mode" listed as coming soon.

### First-party apps shipped on the device (all open source in the KeyOS repo)

| App | What it does | Repo path |
|---|---|---|
| Bitcoin Wallet | Offline signing via QR / file / QuantumLink. Multisig, Taproot, passphrases, testnet, message signing, SeedQR, address explorer, labelled accounts. BDK-based | `apps/gui-app-bitcoin` |
| Vault | Encrypted seed-phrase / secrets storage with Magic Backup; 50 GB encrypted file space | `apps/gui-app-seed-vault` |
| Keys | FIDO2 / WebAuthn / U2F security key (replaces a YubiKey) | `apps/gui-app-security-keys` |
| 2FA | TOTP generator backed by the secure element; imports Aegis, 2FAS, Proton, Google Authenticator migration QR | `apps/gui-app-authenticator` |
| Files | File browser across Airlock, User and USB scopes | `apps/gui-app-file-browser` |
| QR Scanner | Shared QR / UR / animated-QR decoder service used by all apps | `apps/gui-app-qr-scanner` |
| Image Viewer (dev firmware only) | ~160 lines of Rust; the smallest reference app | `apps/gui-app-image-viewer` |
| Playground (dev firmware only) | UI component sandbox | `apps/gui-app-playground` |

### Companion app

**Envoy** (iOS/Android, v2.2.x) pairs over QuantumLink or QR. Handles Magic Backups, firmware updates,
exchange rates, single-sig wallet coordination. Foundation does not ship a multisig coordinator; they point
to Sparrow / Electrum / Nunchuk. Future: an opt-in app catalog inside Envoy (not live yet).

### Firmware versions

KeyOS 1.2.1 → 1.3.1 (seed-word completion) → 1.3.2 (faster updates) → **1.4.0** in beta (redesigned
launcher, PIN before seed words, ATECC608 RNG mixed into master-seed derivation, manual updates without Envoy).
SDK 1.1.0 documents the **KeyOS 1.4.0** API.

---

## 3. KeyOS architecture

- **Microkernel in Rust.** The kernel is **Palladium**, a Foundation fork of the open-source **Xous**
  project (the repo directory is still named `xous/`; docs sometimes say Xous). Drivers and system services run
  as isolated user-space processes.
- **Every app is its own process** with its own address space and its own storage scope. Apps cannot read
  each other's memory or storage. Freed pages are zeroed before reuse; process exit scrubs everything.
- **IPC only via kernel-mediated message passing** to named services (`os/fs`, `os/gui-server`,
  `os/security`, ...). No shared memory, no unrestricted syscalls.
- **Declarative permissions.** The app's signed manifest lists, per service, exactly which messages it may
  send. Enforced at runtime by `app-manager` + `xous-names` + kernel. **No runtime escalation**: the manifest
  is the whole attack surface. The compile-time `MessageAllowed<M>` bounds are developer convenience, not the
  enforcement.
- **Trusted display path.** PIN entry, permission prompts and confirmations are drawn by KeyOS-owned UI
  (lock screen, alerts) so an app cannot spoof a system prompt.
- **Secure element.** Seeds never leave the SE unencrypted. Apps get a **per-app deterministic seed**
  (`GetAppSeed` = HMAC-SHA256(app_id, master_seed) in the hosted implementation) after a user grant. Changing
  your app ID changes every derived key.
- **No network.** No IP stack, no Wi-Fi. Outbound paths: USB-C, NFC, camera, QuantumLink BT. All user-initiated.
- **Hosted mode.** The simulator swaps device drivers for mocks so the whole OS runs on a desktop.
- **Panics abort immediately** (no unwinding); `panic-immediate-abort`. Locks can always be `unwrap()`ed.
- **System services are always running**; if one panics the kernel panics. Connecting to them is infallible.
- **Target triple:** `armv7a-unknown-xous-elf`. Apps are PIC/PIE ELFs signed with `cosign2`, installed under
  `apps/<app-name>/` (built-in) or `sideloaded-apps/<app-id>/`.
- **App IDs** from sideloaded manifests are trusted only as far as the user trusted the publisher cert
  (Developer Mode / allowed publisher waives most guarantees by design).
- **Filesystem reliability classes:** system partition always available; encrypted partition available after
  first unlock until power-off; USB and Airlock can appear/disappear at any time (handle errors).
- **Memory advice:** 128 MB RAM, mostly-complete `std` with heap. It is not a microcontroller. Prefer
  event-driven designs over polling (idle time saves battery, fewer cycles do not).

---

## 4. The Foundation SDK

| Item | Value |
|---|---|
| SDK version | **1.1.0** (public beta) |
| KeyOS API documented | **1.4.0** (API version "1") |
| Language | Rust (nightly, pinned by SDK) |
| UI toolkit | Slint (Foundation fork `v1.17.0-foundation3`) |
| Build env | Nix (provides toolchain, cross-compiler, simulator, Slint tools, signing) |
| Host platforms | `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, Windows via WSL2 (Linux package) |
| Install dir | `~/.foundation/sdk/` ; signing identities in `~/.foundation/signing/` ; themes cache `~/.foundation/themes/` |
| Simulator | Hosted full-stack KeyOS with persistent user disk |
| Feedback | hello@foundation.xyz |

What the SDK bundle ships: the `foundation` CLI, `foundation-slint-viewer`, `foundation-simulator`,
`foundation-passport-drive` (USB CLI + MCP server), `foundation-keyos-log-viewer`, `foundation-theme-editor`,
`foundation-theme-compiler`, `foundation-asset-tool`, `cosign2`, a curated KeyOS source subtree under
`lib/keyos/`, the public `@ui/...` Slint surface, templates, examples, docs (`foundation docs`), agent skills,
and `share/99-passport.rules` (Linux udev).

Public API crates included in the SDK (from `sdk/sdk-build.toml`):

| Crate | Description |
|---|---|
| `server` | Type-safe server traits, checked connections, IPC helpers (`use_api!`, `MessageAllowed`) |
| `app_manager` | App launch, metadata, QR acceptance criteria, third-party certificates |
| `backup` | Backup and restore (Foundation-signed only in practice) |
| `camera` | Camera frame subscription and configuration |
| `crypto` | Hardware AES, SHA, HMAC, Shamir |
| `fs` | Filesystem handles, dirs, metadata, mapped files |
| `gui_server_api` | `os/gui-server`: register app, frames, input, modals, navigation |
| `haptics` | Haptic patterns |
| `nfc` | NFC status (read/write Foundation-only) |
| `quantum_link` | QuantumLink status, rates, pairing events (wallet-sync Foundation-only) |
| `rgb_led` | RGB LED colour and animation |
| `security` | Authentication, seeds, keys, device security state |
| `settings` | Typed settings get/set/subscribe |
| `foundation_manifest` | Parse/inspect KeyOS app manifests |

Also copied into the SDK: `slint-keyos-platform` (runtime, router, build helper), `ui2/components`,
`utils/file-backed` (JSON-backed persisted state), `worker`, `xous/api`, `imports/getrandom` (TRNG-backed).

Full API docs: `foundation docs` (local static bundle with a KeyOS-version selector) or
https://docs.foundation.xyz/developers/api/

---

## 5. Install and environment

Prerequisites: **Nix with flakes**, **git**, a Unix shell. Do **not** install Rust, the ARM cross-compiler or
`cosign2` yourself; `foundation develop` provides them.

```bash
# Install / verify
curl -sSfL https://sdk.foundation.xyz/latest/install.sh | bash
# NixOS / Home Manager users (don't touch shell rc):
curl -sSfL https://sdk.foundation.xyz/latest/install.sh | FOUNDATION_SDK_UPDATE_RC=0 bash

foundation develop        # enters the SDK Nix shell; exports FOUNDATION_SDK_ROOT, FOUNDATION_SDK_BIN
foundation doctor         # run INSIDE the shell (outside it reports the active-shell check as failed)

# Later
foundation update         # install newest SDK release (signature-verified installer)
foundation exit           # nix-collect-garbage + remove ~/.cache/nix (keeps SDK and signing identities)
```

`foundation build`, `foundation sim`, `foundation sideload` must run inside `foundation develop`.
`new`, `doctor`, `docs`, `preview`, `theme`, `cert`, `logs` can run outside it.

After every SDK update: `foundation clean && foundation build`, then `foundation doctor`, restart shell/editor.

Linux USB: copy `<sdk-root>/share/99-passport.rules` to `/etc/udev/rules.d/`, then
`sudo udevadm control --reload && sudo udevadm trigger`. A permission error opening the device means the rules
are missing.

Environment variables: `FOUNDATION_SDK_ROOT` (pin SDK root), `FOUNDATION_SDK_BIN` (set by develop),
`FOUNDATION_THEMES_RUST_DIR` (set by build/sim), `SLINT_DIR` (maintainers only), `FOUNDATION_LANG`
(localised CLI command names, e.g. `foundation nuevo`).

Toolchain facts: nightly Rust pinned by SDK; target `armv7a-unknown-xous-elf`; hardware `RUSTFLAGS`
`--cfg keyos -C relocation-model=pic -C link-arg=-pie`; simulator uses `--cfg keyos` only. Use
`#[cfg(keyos)]` / `#[cfg(not(keyos))]` for device vs. hosted differences.

---

## 6. CLI reference

| Command | Purpose / notes |
|---|---|
| `foundation new <name> [--template T] [--no-git] [--friendly-name ..] [--publisher-name ..] [--app-id ..] [--app-version ..] [--contact-email ..] [--support-url ..]` | Scaffold an app. Prompts (or flags) for friendly name, launcher name, description, publisher, contact email, support URL, icon path, app ID (blank = random 128-bit `0x..` hex), version, min KeyOS version. Templates: `default-app`, `multi-page-app`, `kitchen-sink`. Runs `git init` unless `--no-git`. Installs agent skills and `.foundation-sdk/current` mapping |
| `foundation develop` | Enter SDK Nix shell |
| `foundation exit` | Clean Nix cache state |
| `foundation update [--no-verify]` | Install latest SDK (verifies installer signature against the running SDK's release key) |
| `foundation doctor` | Advisory environment check: nix, SDK root, cargo, Xous target, `arm-none-eabi-strip`, `cosign2`, Slint viewer, git. Always exits 0 today |
| `foundation docs [SDK_VERSION] [--url]` | Open the static API docs bundle (page selector picks KeyOS API version) |
| `foundation skills` | Copy SDK agent skills into `.claude/skills/` and `.agents/skills/` of the current app |
| `foundation build [--release]` | Compile, strip, **sign** hardware bundle → `target/keyos/<app-name>/` (`app.elf`, `manifest.json`, `icon.bin`, `icon-dark.bin`, `resources/`). Prints the app hash |
| `foundation clean` | Remove `target/`, generated `manifest.toml`, `ui/ui` mapping, `.foundation-sdk/`. Run after SDK updates |
| `foundation sim` | Build debug hosted app and launch the simulator (persistent user disk) |
| `foundation preview [file] [--auto-reload]` | Live Slint preview with hot reload in `foundation-slint-viewer`. Default `ui/app.slint`. Run after a first `build`/`sim` so `ui/gen` exists |
| `foundation pack [--release] [--out PATH]` | Build, sign and wrap into one `target/keyos/<app-name>.app` (gzip tar: `manifest.json` first, then hashed files). Installs capped at **64 MiB**. `--out` must not be inside the bundle or a symlink |
| `foundation sideload [--release] [--no-run]` | Build, sign, upload over USB debug to the device, launch. Requires unlocked device, USB, Developer Mode, trusted cert. `--no-run` uploads only |
| `foundation logs [--timeout SECONDS]` | Device log viewer over USB debug (default discovery wait 3 s) |
| `foundation cert gen [name] [--publisher-name N] [--contact-email E] [--support-url U]` | Create secp256k1 key + self-signed X.509 code-signing cert + `cosign2.toml` in `~/.foundation/signing/<name>/` (`private.pem`, `public.pub`, `.crt`). Prints fingerprint |
| `foundation cert print [name]` | Show cert details |
| `foundation cert fingerprint <cert>` | Print full (64 hex) and short (`xxxxxxxx…xxxxxxxx`) canonical fingerprint |
| `foundation cert install [name]` | Register publisher on connected device (interactive Allow prompt; shows "Foundation has NOT verified this publisher's identity"). Appears under Settings > Apps > Trusted Publishers |
| `foundation theme [path]` | Visual theme editor (undo/redo) for `resources/theme.json` or any theme JSON |
| `foundation themes build / list / new <name> [--from <base>]` | Compile theme JSON → Rust in `~/.foundation/themes/rust/`; list inheritable themes; create a named shared brand theme |
| `foundation completions <bash\|zsh\|fish\|powershell> [--stdout]` | Shell completions |
| `foundation plugin ...` | **Disabled** in supported builds while the trust model is redesigned. Do not use |

Recommended loop: small change → `foundation preview` (UI) → `foundation sim` (behaviour) → review →
hardware if needed. Troubleshooting order: `doctor` → `preview --auto-reload` → `sim` → `sideload --no-run` →
inspect `app-config.toml` signing.

---

## 7. App project layout

```
my-app/
├── app-config.toml           # identity, version, publisher, icon, theme, permissions, QR rules, signing
├── permission_templates.toml # named permission bundles expanded into the manifest
├── Cargo.toml                # canonical app version lives here
├── Cargo.lock                # pinned; validated by the SDK
├── build.rs                  # Slint codegen (router, exports, i18n toggles)
├── AGENTS.md                 # guidance for AI coding agents
├── .mcp.json                 # registers the passport-drive MCP server
├── .gitignore                # target/  .foundation-sdk/  ui/ui  manifest.toml
├── .foundation-sdk/current   # generated symlink to active SDK (not committed)
├── src/
│   ├── main.rs               # entry point, state, service calls, callback wiring
│   └── theme.rs              # generated theme application (system light/dark)
├── ui/
│   ├── app.slint             # root window (AppWindow)
│   ├── callbacks.slint       # export global Callbacks { callback ...; }
│   ├── pages/                # multi-page: <slug>/page.slint + <slug>/props.slint
│   └── gen/                  # GENERATED: router.slint, navigate.slint, exports.slint, tr.slint. Do not edit
├── i18n/
│   └── en.json               # translation strings
└── resources/                # MUST be committed or the app won't build for others
    ├── icon.svg              # exactly 110x110 px, transparent glyph
    ├── icon-dark.svg         # optional dark variant, same rules
    ├── theme.json            # app theme (source of truth, edited by `foundation theme`)
    ├── images/               # .svg .png .jpg .jpeg .webp .bmp -> Images.image("name")
    └── fonts/                # .ttf .otf .ttc -> use real family name in font-family
```

Build outputs: `target/keyos/<app-name>/` and `target/keyos/<app-name>.app`. Generated `manifest.toml`
(from `app-config.toml`) and `ui/ui` (SDK UI mapping) are regenerated by the CLI.

---

## 8. app-config.toml, Cargo.toml, build.rs

### app-config.toml fields

| Field | Required | Notes |
|---|---|---|
| `app-name` | yes | Cargo package name and build output dir name (kebab-case) |
| `friendly-app-name` | yes | Display name |
| `launcher-app-name` | no | Falls back to friendly name |
| `description` | yes | One line |
| `icon` | yes | Relative path, validated at build (110×110) |
| `theme` | yes | e.g. `resources/theme.json` |
| `app-id` | yes | `0x`-prefixed even-length hex, unique on device (16 bytes typical). **Derivation input for `GetAppSeed`**; changing it changes all derived keys and storage scope |
| `min-keyos-version` | yes | e.g. `1.0.0` (template default) |
| `version` | no (legacy) | Version now comes from `Cargo.toml`; a duplicate here must match and should be removed |
| `signing-identity` | no | Name under `~/.foundation/signing/`. Don't commit to public repos |
| `cosign2-config` | no | Explicit `cosign2.toml` path. Don't commit |
| `[publisher]` | yes | `name`, `contact-email`, `support-url` |
| `[permissions]` | yes | `template = ["gui-app", ...]` plus `"os/<service>" = ["MessageA", "MessageB"]` |
| `[[qr-match-rules]]` | no | See Section 14 |

Template as generated (`default-app`):

```toml
app-name = "my-app"
friendly-app-name = "My App"
launcher-app-name = "My App"
description = "..."
icon = "resources/icon.svg"
theme = "resources/theme.json"
app-id = "0x..."                 # random 128-bit hex if left blank
min-keyos-version = "1.0.0"

[publisher]
name = "My Publisher"
contact-email = "me@example.com"
support-url = "https://example.com"

[permissions]
template = ["gui-app"]
# multi-page template adds:
# "os/settings" = ["GetDeviceName", "GetLocale", "GetPrimeColor"]
```

Signing identity resolution order at build: (1) `cosign2-config`, (2) `signing-identity`, (3) identity whose
name matches `[publisher].name`, (4) the only identity if exactly one exists, (5) interactive prompt.

### Cargo.toml (template)

```toml
[package]
name = "my-app"
version = "0.1.0"        # canonical app version (copied into manifest + signing header)
edition = "2021"

[workspace]
exclude = [".foundation-sdk"]

[dependencies]
log = "0.4"
log-server = { package = "xous-api-log", path = ".foundation-sdk/current/lib/keyos/xous/api/log" }
server = { path = ".foundation-sdk/current/lib/keyos/server" }
slint-keyos-platform = { path = ".foundation-sdk/current/lib/keyos/slint-keyos-platform/runtime" }
foundation-themes = { path = ".foundation-sdk/current/lib/keyos/sdk/crates/foundation-themes" }
# Add service crates the same way, e.g.
# security = { path = ".foundation-sdk/current/lib/keyos/api/security" }
# crypto   = { path = ".foundation-sdk/current/lib/keyos/api/crypto" }
# haptics  = { path = ".foundation-sdk/current/lib/keyos/api/haptics" }

[build-dependencies]
slint-keyos-platform-build = { path = ".foundation-sdk/current/lib/keyos/slint-keyos-platform/build" }

# Randomness is pre-wired: add `getrandom = "0.2"` (or `rand = "0.8"`) and calls route to the TRNG on device.
# Stay on getrandom 0.2; 0.3+ is not covered.
[patch.crates-io]
getrandom = { path = ".foundation-sdk/current/lib/keyos/imports/getrandom" }
```

(The template writes the path prefix via the `{{sdk_keyos_root}}` variable; the generated project resolves
to `.foundation-sdk/current/lib/keyos`.) Any pure-Rust `no_std`/`std` crate from crates.io can be added
normally (the authenticator app pulls in `image`, protobuf, zip, PGP, etc.).

### build.rs (template)

```rust
use slint_keyos_platform_build::{compile_options, CompileOptions};

fn main() {
    let themes_rust_dir = std::env::var("FOUNDATION_THEMES_RUST_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/.foundation/themes/rust")
    });
    println!("cargo:rustc-env=FOUNDATION_THEMES_RUST_DIR={themes_rust_dir}");
    println!("cargo:rerun-if-env-changed=FOUNDATION_THEMES_RUST_DIR");

    compile_options(CompileOptions {
        module_path: "ui/app.slint",
        include_slint: true,
        include_router: false,            // true for multi-page-app
        include_translations: false,      // true to generate TrId enum from i18n/
        include_time_localization: false,
    });
}
```

---

## 9. Permissions model

- Declared per service in `[permissions]` as `"os/<service>" = ["MessageName", ...]`, or via named
  bundles from `permission_templates.toml` with `template = ["gui-app", "fs-generic", ...]`.
- The CLI expands this into the signed `manifest.json`. The kernel enforces exactly that set. Undeclared
  calls fail at runtime; with `use_api!` they also fail at **compile time** via `MessageAllowed<M>` bounds.
- Each message in a service's own manifest carries three attributes that decide who may hold it:
  - `permissionGroup` (e.g. `file-system.user-files`). **A message with a group is grantable to third-party
    apps; a message with no group is Foundation-signed only** (the default `requiredSignature` derives from this).
  - `requiredSignature = "foundation"` can be set explicitly to lock a grouped message to Foundation apps.
  - `approval`: `autoAllow` (granted silently once declared), `grantOnFirstUse` (user prompted once per
    permission subgroup; answer persists; manageable under Settings > Apps > *app*), or default
    `notUserGrantable` (never grantable via UI).
- Permission groups shown to the user: `ui-and-input.*`, `file-system.*`, `device-secrets.*`,
  `peripherals.*`, `cryptography.*`, `settings.*`, `device-connectivity.*`, `network-and-pairing.*`,
  `power-and-firmware.*`, `app-management.*`.

### Permission templates shipped with new apps

```toml
[gui-app]            # every GUI app needs this
"os/fs" = ["OpenDirMessage","OpenFileMessage","CloseFile","CloseDir","ReadFile","NextEntry","MapFileMessage","GetMetadata"]
"os/gui-server" = ["RegisterAppMessage","RequestRedraw","AnimateNextFrame","SubmitFrame","UpdateKeyboard","HideKeyboard","ShowModal"]
"os/settings" = ["GetSystemTheme","SubscribeSystemTheme","GetDebugTouch","SubscribeDebugTouch"]

[fs-generic]         # read/write files
"os/fs" = ["OpenDirMessage","OpenFileMessage","CloseFile","CloseDir","CreateDirMessage","ReadFile","SeekFile","WriteFile",
           "TruncateFile","SetLen","GetMetadata","NextEntry","Flush","Remove","Rename","AtomicCopy","AsyncRead","AsyncWrite",
           "AsyncCopyBlock","SubscribeFilesystemEvent"]

[navigable]          # receive navigation/QR handoffs
"os/gui-server" = ["GetPendingNavRequest","FinishResponse","NavigationCancel"]

# in the repo root permission_templates.toml (copy into yours if needed):
[crypto-sha]
"os/crypto" = ["ShaSetContext","ShaUpdate","ShaGetContext","ShaDrop"]
[crypto-aes]
"os/crypto" = ["AesSetup","AesExecute","AesAad","AesGcmTag","AesClear"]
```

### Using a service in Rust

```rust
// Generates a <Service>Permissions type bound to your manifest and a client handle.
security::use_api!(security, server);          // -> SecurityApi / security_permissions::SecurityPermissions
crypto::use_api!();                            // form used by Foundation apps
let sec = SecurityApi::default();
let rnd: [u8; 32] = sec.get_random()?;         // compiles only if "os/security" = ["GetRandom"] is declared
let app_seed = sec.app_seed()?;                // needs "GetAppSeed"; user is prompted once (grantOnFirstUse)
```

The `app_ui2!` macro already runs `use_api!` for `fs`, `gui_server_api` and `settings` and defines
`type AppContext = AppContext<GuiPermissions, FileSystemPermissions>`.

---

## 10. What third-party apps can call (per service)

Derived from each service's `manifest.toml` in the repo using the rule in Section 9. "3P" = grantable to a
sideloaded third-party app. "F" = Foundation-signed only. "grant" = user prompted on first use.

### `os/security` (crate `security`)

| Message | Client fn | 3P? | Approval | Group |
|---|---|---|---|---|
| `GetAppSeed` | `app_seed() -> AppSeed` | **3P** | grant | device-secrets.app-scoped-seed |
| `GetRandom` | `get_random() -> [u8;32]` | **3P** | auto | device-secrets.general-status |
| `GetDeviceId` | `device_id()` | **3P** | auto | device-secrets.general-status |
| `GetOsVersionInfo` | `os_version_info()` | **3P** | auto | device-secrets.general-status |
| `GetBootloaderBuildDate` | `bootloader_build_date()` | **3P** | auto | device-secrets.general-status |
| `GetSeed`, `SetSeed`, `Login`, `Logout`, `SetSeedAndPin`, `ChangePin`, `IsPinSet`, `GetAttemptsRemaining`, `Lockout`, `GetSecurityWords`, `SignWithFidoKey`, `GetFidoPubkey`, `SignWithSecurityCheckKey`, `ScChallenge`, `GetSeedFingerprint`, `ComputeSeedFingerprint`, `KeycardAuthenticityMac`, BT challenge msgs, `GetMasterKeyState`, `GetPinEntryMode` | | F | | (no group) |

`AppSeed` is a 32-byte per-app secret. On hardware the request waits until the user is logged in. In the
hosted simulator it is `HMAC-SHA256(key = app_id, data = master_seed)`. Derive everything your app needs
(encryption keys, identity keys, BIP32 roots) from it with HKDF or BIP32 in pure Rust.

### `os/fs` (crate `fs`)

| Messages | 3P? | Approval | Group |
|---|---|---|---|
| `OpenFileMessage`, `ReadFile`, `SeekFile`, `CloseFile`, `OpenDirMessage`, `NextEntry`, `GetMetadata`, `CloseDir`, `MapFileMessage`, `SubscribeFilesystemEvent`, `AsyncRead` | **3P** | auto | file-system.read-handles |
| `WriteFile`, `Remove`, `AtomicCopy`, `CreateDirMessage`, `Rename`, `TruncateFile`, `Flush`, `SetLen`, `SetMtime`, `AsyncWrite`, `AsyncCopyBlock` | **3P** | auto | file-system.write-and-mutate |
| `GetUserReadAccess`, `GetUserWriteAccess` | **3P** | grant | file-system.user-files |
| `GetUsbReadAccess`, `GetUsbWriteAccess` | **3P** | grant | file-system.usb-files |
| `GetAirlockReadAccess`, `GetAirlockWriteAccess` | **3P** | grant | file-system.airlock-files |
| `ReadBlocks`, `WriteBlocks`, `BlockCount`, `Format*`, `MountAirlock`, `RegisterAppResources`, `RemoveAppData`, `GetSystem*Access`, `GetEncryptedRoot*`, `GetBoot*` | F | | |

Your private `AppData` location needs no extra grant ("available to all apps"); the `User`, `Usb` and
`Airlock` locations need the matching `Get*Access` permission declared and granted.

### `os/crypto` (crate `crypto`)

| Messages | Client | 3P? | Group |
|---|---|---|---|
| `AesSetup`, `AesExecute`, `AesAad`, `AesGcmTag`, `AesClear` | `setup_aes(key, AesMode) -> AesContext` → `execute(..)`, `add_aad(..)`, `gcm_tag()` | **3P** auto | cryptography.primitives |
| `Hmac` | `hmac224/256/384/512(key, data)` | **3P** auto | cryptography.primitives |
| `ShamirSplit`, `ShamirRecover` | `split_secret(..)`, `recover_secret(indexes, shares)` | **3P** auto | cryptography.secret-sharing |
| `ShaSetContext`, `ShaUpdate`, `ShaGetContext`, `ShaDrop` (device only, `cfg = keyos`) | hardware SHA | **3P** auto | cryptography.hardware-hashing |
| `DiskEncryptUnsafe`, `SubscribeDiskEncryptComplete` | | F | |

### `os/camera` (crate `camera`) and camera UI in `os/gui-server`

| Message | 3P? | Approval | Group |
|---|---|---|---|
| `Subscribe` (raw frames) | **3P** | grant | peripherals.camera-use |
| `IsEnabled`, `IsInUse` | **3P** | auto | peripherals.camera-status |
| `ShowCamera`, `HideCamera` (gui-server) | **3P** | grant | peripherals.camera-use |
| `SetEnabled`, `NotifyVisible`, `GetParams`, `SetParams` | F | | |

For QR/UR scanning you normally do **not** touch the camera: open the system QR Scanner via `ShowModal`
(Section 13), which is covered by the `gui-app` template.

### `os/gui-server` (crate `gui_server_api`)

| Messages | 3P? | Approval | Group |
|---|---|---|---|
| `RegisterAppMessage`, `SubmitFrame`, `RequestRedraw`, `AnimateNextFrame` | **3P** auto | ui-and-input.app-surface |
| `UpdateKeyboard`, `HideKeyboard` | **3P** auto | ui-and-input.keyboard-request |
| `ShowModal` (QR scanner, file picker, PIN verify, alerts) | **3P** grant | ui-and-input.brokered-modal |
| `GetPendingNavRequest`, `FinishResponse`, `NavigationCancel` | **3P** auto | ui-and-input.navigation-response |
| `SetControlCenterColor` | **3P** auto | ui-and-input.control-center-appearance |
| `ShowCamera`, `HideCamera` | **3P** grant | peripherals.camera-use |
| `SwitchTo`, `SwitchToLauncher`, `CloseApp`, `NavigateTo`, `Claim*Role`, `RegisterKeyboard`, `UpdateKioskPolicy`, ... | F | | |

### `os/settings` (crate `settings`)

| Messages | 3P? | Group |
|---|---|---|
| `GetSystemTheme`, `SubscribeSystemTheme`, `GetLocale`, `SubscribeLocale`, `GetDeviceName`, `SubscribeDeviceName`, `GetPrimeColor`, `GetTimeZone`, `SubscribeTimeZone`, `LookupTimeZone`, `ListTimeZone`, `GetUseStandardTimeFormat`, `SubscribeUseStandardTimeFormat`, `GetDebugTouch`, `SubscribeDebugTouch` | **3P** auto | settings.ui-essentials |
| `GetUsbEnabled`, `SubscribeUsbEnabled`, `GetAirlockMode`, `SubscribeAirlockMode` | **3P** auto | device-connectivity.usb-device-status |
| `Get/Subscribe` brightness, auto-lock, NFC/BT/camera enabled, developer mode, magic backup, security words, board revision, onboarding, touch offset | F (explicit `requiredSignature = "foundation"`) | settings.device-configuration |
| All `Set*` messages | F | |

### `os/haptics`, `os/rgb-server` (crates `haptics`, `rgb_led`)

| Message | Client | 3P? | Group |
|---|---|---|---|
| `Vibrate` | `click()`, `double_click()`, `triple_click()`, `strong_click()`, `vibrate(HapticPattern)` | **3P** auto | peripherals.feedback |
| `SetTo`, `SetAllTo`, `AnimateAllTo` | `set_to(index, RgbColor)`, `set_all_to(RgbColor)`, `animate_all(RgbAnimation)` | **3P** auto | peripherals.feedback |

### `os/power-manager-ext` (crate `power_manager`)

| Message | Client | 3P? | Group |
|---|---|---|---|
| `GetStatus`, `GetExtendedStatus`, `StatusSubscribe` | `status()`, `extended_status()`, `subscribe_status(..)` | **3P** auto | power-and-firmware.battery-status |
| `Reboot`, `Shutdown`, `SetPeripheralEnabled`, ... | | F | |

### `os/nfc` (crate `nfc`)

| Message | 3P? | Group |
|---|---|---|
| `IsEnabled`, `IsActive` | **3P** auto | device-connectivity.nfc-status |
| `ReadNdefRawMsg`, `WriteNdefRawMsg`, `SetEnabled` | F | (no group) |

Keycard-style flows are not available to third-party apps.

### `os/quantum-link` (crate `quantum_link`)

| Messages | 3P? | Approval | Group |
|---|---|---|---|
| `SubscribeConnectionStatus`, `SubscribeEnvoyStatus`, `SubscribeExchangeRate`, `SubscribeExchangeRateHistory`, `EnvoyTimezone` | **3P** | auto | network-and-pairing.status-and-rates |
| `PublishPsbt`, `SubscribeSignPsbt`, `SendAccountUpdate`, `SubscribeAccountUpdate`, `SubscribePublishedAccountUpdate`, `SendApplyPassphrase`, `SendPrimeFiatPreference` | F (`requiredSignature = "foundation"`) | grant | network-and-pairing.wallet-sync |
| pairing / firmware / magic-backup / shard messages | F | | |

**Important:** the capabilities page lists QuantumLink as "Stable" but, in SDK 1.1.0, third-party apps only get
connection status, Envoy status and exchange rates. There is **no general-purpose data channel** to a phone
app yet, and the raw `bt` crate is a stub ("neither copied nor documented"). The showcase concepts that talk
to Envoy/phones over QuantumLink (password manager, Pubky Ring) depend on future SDK work. Plan phone/desktop
interaction via **QR / animated UR** (both directions) or **USB mass storage / Airlock files** today.

### `os/app-manager` (crate `app_manager`)

| Message | 3P? | Group |
|---|---|---|
| `GetAppName`, `GetAppIcon`, `GetQrMatchRules` | **3P** auto | app-management.app-metadata |
| `LaunchApp`, `ListApps`, `InstallAppArchive`, certificate messages, `RemoveApp`, ... | F | |

### Not exposed to third-party apps at all

GPIO, I²C, SPI, DMA, raw USB (`os/usb`: HID / HWI / keyboard driver is "coming"), backup, keycard, update,
master seed / PIN / login.

---

## 11. Filesystem scopes

`fs::Location` enum (from `api/fs/src/lib.rs`):

| Location | Path on device | Access | Notes |
|---|---|---|---|
| `AppData` | `<encrypted>/appdata/<app-id>/` | all apps, RW | **Your private encrypted storage.** Removed if the app is uninstalled or re-signed by a different key |
| `AppResources` | `.../apps/<app-name>/resources/` or `.../sideloaded-apps/<app-id>/resources/` | all apps, RO | Your bundled `resources/` |
| `CommonAssets` | `<system-volume>/common` | all apps, RO, mappable | Shared KeyOS assets (fonts, icons) |
| `User` | `<encrypted>/user` | needs `GetUserRead/WriteAccess` (grant) | The user's encrypted files (what Files/Vault show) |
| `Usb` | external USB drive | needs `GetUsbRead/WriteAccess` (grant) | Mass-storage device plugged into USB-C. Can disappear any time |
| `Airlock` | virtual partition | needs `GetAirlockRead/WriteAccess` (grant) | Quarantine for files arriving from a host over USB (device appears as a drive to the computer). Can disappear any time |
| `SystemAppData` | `<system-volume>/state/<app-id>/` | privileged | Unencrypted per-app state (Foundation) |
| `System`, `EncryptedRoot`, `Boot` | | privileged | Foundation only |

File picker `Location` (UI side) maps as: `Internal` → `fs::Location::User`, `External` → `fs::Location::Usb`,
`Airlock` → `fs::Location::Airlock`. Path strings in the viewer apps use `user:/...`, `usb:/...`, `airlock:/...`.

`FileSystem` client methods: `open_file(path, location, OpenFlags{read,write,create})`, `open_dir`,
`create_dir`, `ensure_parent_dir_exists`, `remove`, `rename`, `atomic_copy`, `metadata`, `map_file`
(CommonAssets only), `durable_file_write(path, location, &[u8])`, `durable_file_read(path, location) -> Vec<u8>`,
`subscribe_filesystem_events`, `wait_for_filesystem(location)`. `File` implements `Read`/`Write`/`Seek` plus
`truncate`, `set_len`, `metadata`, `set_mtime`, `overwrite(&[u8])`, `copy_to`, async variants. `Dir::next_entry()
-> Option<DirEntry{name, is_file, ..}>`, `pick_next_filename`.

For simple persisted app state the reference apps use `slint_keyos_platform::file_backed::JsonBacked` /
`FileBacked` (from `utils/file-backed`) which serialises a serde struct to a file in `AppData`.

---

## 12. Service API quick reference

All clients follow the same pattern: `use_api!` generates a permissions type; the handle is
`XxxApi::default()` (or `try_new_with_timeout`); methods have `where P: MessageAllowed<Msg>` bounds.

```rust
// security
let sec = SecurityApi::default();
sec.get_random() -> Result<[u8;32], AccessDenied>
sec.app_seed()   -> Result<AppSeed, AccessDenied>      // AppSeed(32 bytes); .as_bytes()
sec.device_id()  -> Result<DeviceId, GetDeviceIdError>
sec.os_version_info() -> Result<Option<OsVersionInfo>, AccessDenied>
// also in the crate: Seed::from_mnemonic / to_mnemonic_words / to_standard_seed_qr_data / to_compact_seed_qr_data,
// parse_seedqr(&[u8]) -> Mnemonic   (BIP39 helpers, pure Rust)

// crypto
let c = CryptoApi::default();
c.hmac256(key: Vec<u8>, data: Vec<u8>) -> Result<Vec<u8>, CryptoError>
let ctx = c.setup_aes(&key, AesMode::...)?; ctx.execute(..); ctx.add_aad(&aad); ctx.gcm_tag() -> [u8;16]
c.split_secret(..) / c.recover_secret(indexes, shares)

// haptics / led
HapticsApi::default().click();   // double_click, triple_click, strong_click, vibrate(HapticPattern)
RgbLedApi::default().set_all_to(RgbColor{..}); .set_to(i, color); .animate_all(RgbAnimation)

// settings (SettingsApi; theme subscription shown in theme.rs template)
SettingsApi::default().get_system_theme() -> SystemTheme::{Light,Dark}
SettingsApi::default().get_prime_color()
// GetDeviceName / GetLocale / GetTimeZone via typed get/subscribe helpers; see `foundation docs`

// power
PowerManagerApi::default().status() / extended_status() / subscribe_status(ctx)

// camera status
CameraApi::try_new_with_timeout(d)?.is_enabled() / is_in_use()

// quantum-link (3P subset)
QuantumLinkApi: subscribe connection status / Envoy status / exchange rate (archiveEvent subscriptions)

// app-manager
AppManagerApi: get_app_name(app_id), get_app_icon(app_id), get_qr_match_rules()
```

Runtime helpers from `slint_keyos_platform` used throughout the reference apps: `spawn_local(async)`,
`spawn_worker`, `sleep`, `StoredValue<T>`, `subscribe_scalar::<Perms,_>(SubscribeMsg)`, `Timer`,
`TimerMode`, `VecModel`, `ModelRc`, `SharedString`, re-exported `slint`. The event loop is single-threaded;
do heavy work in a worker (`spawn_worker`) and post results back.

Logging: `log_server::init_wait(env!("CARGO_CRATE_NAME")).unwrap(); log::set_max_level(LevelFilter::Info);`
then `log::info!/warn!/debug!`. View with `foundation logs` or the MCP `logs` tool. Never log secrets;
`trace!` may hexdump messages.

---

## 13. System navigation helpers

These open **KeyOS-owned screens** on top of your app via `ShowModal` (in the `gui-app` template, first use
prompts the user for the "brokered modal" permission). From `slint_keyos_platform::navigation`:

```rust
use slint_keyos_platform::{
    navigation::{open_qr_scanner, async_open_qr_scanner, select_file, verify_pin},
    gui_server_api::navigation::{
        qrscanner::{ScanQrOptions, ScanQrResult},
        filepicker::{SelectFileOptions, AllowedLocations, AllowedExtensions, Location},
        lockscreen::{VerifyPinOptions, VerifyPinResult},
    },
};
use crate::gui_permissions::GuiPermissions;   // generated by app_ui2!

// QR / UR / animated-UR scanner
let mut opts = ScanQrOptions::new();
opts.header_title = "Scan".into();      // header_left_icon/text, header_right_icon/text, message, button_icon/text
opts.request_matching_apps = false;    // true -> result includes apps whose qr-match-rules matched
match open_qr_scanner::<GuiPermissions>(opts)? {
    Some(ScanQrResult::Qr { data, .. })            => { /* raw QR bytes */ }
    Some(ScanQrResult::Ur2 { ur_type, data, .. })  => { /* UR 2.0 payload, e.g. ur_type == "psbt" */ }
    Some(ScanQrResult::LeftClicked | ScanQrResult::RightClicked | ScanQrResult::ButtonClicked) | None => { /* cancelled */ }
}

// File picker (User / USB / Airlock)
let opts = SelectFileOptions::default()
    .with_hidden_allowed(false)
    .with_search_allowed(false)
    .with_start_location(Location::Internal)
    .with_allowed_locations(AllowedLocations::specific(vec![Location::Internal, Location::External]))
    .with_allowed_extensions(AllowedExtensions::specific::<&str, Vec<&str>>(vec!["png", "jpg"]));
if let Some(result) = select_file::<GuiPermissions>(opts)? {
    let (path, location) = result.files().get(0).cloned().unwrap();   // then open with fs::FileSystem
}

// Re-verify the device PIN before a sensitive action
let r = verify_pin::<GuiPermissions>(VerifyPinOptions { title: Some("Confirm".into()), want_security_words: false })?;
if r.success { /* proceed */ }

// Alerts: `InvokeAlert` / `AlertResult` (async) show a KeyOS-styled modal alert with buttons.
```

Receiving a handoff when the Launcher opened your app from a QR match: declare the `navigable` template
(or any `[[qr-match-rules]]`, which auto-grants `GetPendingNavRequest`) and read the pending nav request at
startup; the payload is a `MatchedQrResult { scan_result, matched_rules }`. See
`apps/gui-app-authenticator/src/main.rs` for a complete example (it handles `otpauth://` QRs).

---

## 14. QR match rules

Register your app with the Launcher's QR scanner tile. When a scan matches exactly one app, KeyOS opens it
directly; when several match, the user chooses. Kebab-case in `app-config.toml` (the CLI converts to camelCase):

```toml
[[qr-match-rules]]
id = "otpauth"                       # stable rule id reported to your app with the scan
priority = 5                         # optional, 1..5, default 3, higher sorts first
id-localizations = { en = "OTP Auth" }
sub-rules = { qr = { QR = { regex-pattern = "^otpauth://" } } }     # QR: min-len, max-len, regex-pattern

[[qr-match-rules]]
id = "crypto-psbt"
id-localizations = { en = "PSBT" }
sub-rules = { ur = { UR = { ur-type = "psbt" } } }                   # UR 2.0 by type
```

Unknown fields fail the build. Declaring any rule grants `os/gui-server` `GetPendingNavRequest`.

---

## 15. UI: Slint and the `@ui` component library

- UI is written in **Slint** (`.slint` files), compiled by `build.rs`. Rust talks to it through generated
  structs/globals: `ui.global::<Callbacks>().on_button_clicked(|| ..)`, `ui.global::<Global>().set_xxx(..)`.
- Root component must be `export component AppWindow inherits Window`.
- Screen: `UISize.screen-width` × `UISize.screen-height` (480 × 800). Leave
  `UISize.control-center-status-bar-height` at the top (status bar / control centre pull-down).
- Two UI generations exist:
  - **`@ui/...` (ui2)** — the public SDK surface for third-party apps, used by `app_ui2!`. Import individual
    files: `@ui/theme.slint`, `@ui/button.slint`, `@ui/card.slint`, etc. About 30 themeable components.
  - **Legacy `@ui/widgets.slint`** (`ui/ui`) — used by Foundation's in-repo apps with the `app!` macro. Rich
    (QR codes, PIN pad, seed words, modals). Templates say not to rely on this facade; but the reference
    apps show how it is used.

### `@ui` (ui2) components available to SDK apps

`Accordion`, `Button` (`text`, `size: ControlSize.sm|md|lg`, `full-width`, `clicked => {}`), `Card`
(`title`, `body`, `footer`, `size`), `Checkbox`, `Chip`, `Dialog`, `Divider`, `Dropdown`, `Icon`, `IconButton`,
`MediaImage` / image view, `Input`, `Link`, `Menu`, `MenuItem`, `MenuRow`, `ProgressBar`, `Radio`, `RadioGroup`,
`Search`, `SegmentedControl`, `Sheet`, `Slider`, `SlideToConfirm`, `Spinner`, `Switch`, `Tabs`, `TextArea`,
`Toast`, `BottomBar`. Each has a sibling `<name>_theme.slint` generated from your theme JSON.

Globals from `@ui/theme.slint`: `Theme` (tokens), `UISize` (screen metrics), `Images` (`Images.image("name")`,
`Images.icon(name, size)`), `ControlSize`.

Theme tokens used in templates: `Theme.palette-background`, `palette-foreground`, `palette-muted`,
`palette-card`, `Theme.spacing-xl / lg / md / sm`, `Theme.radius-default`, `Theme.font-primary`,
`Theme.font-size-title / subtitle / body / label`, `Theme.font-weight-bold / semibold / medium`.

Scaffold `kitchen-sink` to see every component with all size/style/tone/state variants:
`foundation new component-lab --template kitchen-sink && cd component-lab && foundation sim`.

### Legacy widget library (what Foundation apps use; `ui/ui/widgets.slint`)

`AccountCard`, `AppHeader`, `NavHeader`, `Arc`, `BasePage`, `PageBody`, `JumboIconPageBody`, `ContentSection`,
`ButtonSection`, `ScrollBody`, `ScrollFadeMask`, `Modal`, `ConfirmationModal`, `ErrorModal`, `ResultModal`,
`OverlayMenu`, `ShieldPage`, `BaseWindow`, `Battery`, `RadioCard`, `RadioCardGroup`, `Button`,
`CapturingTouchArea`, `CardCircle`, `CardColorPicker`, `CardPill`, `Checkbox`, `CheckboxGroup`, `Chip`,
`Circle`, `CircleIcon`, `CenterLayout`, `CrossLayout`, `ColorPicker`, `Dialog`, `Divider`, `Dropdown`,
`DropdownOverlay`, `Editable`, `FolderList`, `Icon`, `IconButton`, `IconToggle`, `Input`, `PasswordInput`,
`JumboIcon`, `LineCard`, `Link`, `TimerLoader`, `ModalWindow`, `Movable`, `HeadingXs..Xl`, `TextXs..Xl`,
`TextTime`, `Panel`, `Pagination`, `PageSelector`, `Pin`, `ProgressBar`, `TimerCircularProgress`, `PopupMenu`,
`Qrcode`, `QrCodeCard`, `RawQrcode`, `RawQrCodeCard`, `DynamicQrCode`, `AnimatedQrCode`, and more
(seed words, keyboard, etc.). QR rendering helpers also exist in Rust at `slint_keyos_platform::qrcode`.

---

## 16. Starter code (templates)

### `src/main.rs` (default-app)

```rust
mod theme;

use slint_keyos_platform::app_ui2;

app_ui2!("My App");   // generates main(), registers with gui-server, wires fs/gui/settings APIs,
                      // binds Images, defines AppContext and calls app_main()

fn app_main(_cx: AppContext, ui: AppWindow) {
    log_server::init_wait(env!("CARGO_CRATE_NAME")).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    theme::init(&ui);   // apply theme + follow system light/dark

    ui.global::<Callbacks>().on_button_clicked(move || {
        log::info!("Get started clicked");
    });

    ui.run().expect("UI running");
}
```

`AppContext` fields: `gui: Arc<GuiApi>`, `fs: Arc<fs::FileSystem>`, `router: StoredValue<Router>`,
`config: Rc<PlatformConfig { enable_swipe_back }>`, method `set_input_handler(..)`.

### `ui/app.slint` (default-app)

```slint
import { Theme, UISize, Images, ControlSize } from "@ui/theme.slint";
import { Callbacks } from "callbacks.slint";
import { Button } from "@ui/button.slint";
import { Card } from "@ui/card.slint";

export { Callbacks, Theme, Images, ControlSize }
export component AppWindow inherits Window {
    title: "My App";
    preferred-width: UISize.screen-width;
    preferred-height: UISize.screen-height;
    background: Theme.palette-background;

    VerticalLayout {
        padding-left: Theme.spacing-xl;
        padding-right: Theme.spacing-xl;
        padding-top: Theme.spacing-xl + UISize.control-center-status-bar-height;
        padding-bottom: Theme.spacing-xl;
        spacing: Theme.spacing-xl;

        Text {
            text: "My App";
            color: Theme.palette-muted;
            font-family: Theme.font-primary;
            font-size: Theme.font-size-body;
            font-weight: Theme.font-weight-semibold;
            horizontal-alignment: center;
        }
        Rectangle {
            width: 100%; height: 154px;
            background: Theme.palette-card; border-radius: Theme.radius-default; clip: true;
            Image { width: parent.width; height: parent.height; source: Images.image("checkmark"); image-fit: cover; }
        }
        Card { width: 100%; title: "Welcome"; body: "A simple, secure experience starts here."; footer: "Tap below."; }
        Rectangle { vertical-stretch: 1; }
        Button {
            width: 100%; text: "Get started"; size: ControlSize.lg; full-width: true;
            clicked => { Callbacks.button-clicked(); }
        }
    }
}
```

### `ui/callbacks.slint`

```slint
export global Callbacks {
    callback button-clicked();
    // add: callback save(string); in-out property <string> status; etc.
}
```

### `src/theme.rs` (generated, keep as is)

```rust
slint_keyos_platform::settings::use_api!(slint_keyos_platform::settings, slint_keyos_platform::server);
use foundation_themes::ColorScheme;
use slint_keyos_platform::slint::ComponentHandle;

foundation_themes::include_theme!(app_theme);

pub fn init(ui: &crate::AppWindow) {
    apply_system_theme(ui, SettingsApi::default().get_system_theme());
    let ui_weak = ui.as_weak();
    let mut updates = slint_keyos_platform::subscribe_scalar::<settings_permissions::SettingsPermissions, _>(
        settings_permissions::settings::messages::SubscribeSystemTheme,
    );
    slint_keyos_platform::spawn_local(async move {
        while let Some(system_theme) = updates.next().await {
            let Some(ui) = ui_weak.upgrade() else { break };
            apply_system_theme(&ui, system_theme);
        }
    }).detach();
}

fn apply_system_theme(ui: &crate::AppWindow, system_theme: settings_permissions::settings::global::SystemTheme) {
    let scheme = match system_theme {
        settings_permissions::settings::global::SystemTheme::Dark => ColorScheme::Dark,
        settings_permissions::settings::global::SystemTheme::Light => ColorScheme::Light,
    };
    foundation_themes::apply_theme!(ui, app_theme::theme(), scheme);
}
```

### Smallest real app: Image Viewer (pattern for file picking + reading)

```rust
app!("Image Viewer");   // legacy macro; use app_ui2! in SDK apps

fn app_main(_cx: AppContext, ui: AppWindow) {
    log_server::init_wait(env!("CARGO_CRATE_NAME")).unwrap();
    ui.global::<Callbacks>().on_popup_select_file({
        let weak_ui = ui.as_weak();
        move || {
            let ui = weak_ui.unwrap();
            if let Some((paths, index)) = file_selection_popup() {
                ui.global::<Global>().set_file_paths(ModelRc::new(
                    paths.into_iter().map(|s| s.into()).collect::<VecModel<SharedString>>()));
                ui.global::<Global>().set_current_file_idx(index as _);
            }
        }
    });
    ui.run().expect("UI running");
}

fn load_image(path: &str) -> Option<Image> {
    let (prefix, path) = path.split_once(":/")?;
    let location = match prefix { "user" => fs::Location::User, "usb" => fs::Location::Usb, _ => return None };
    let fs = FileSystem::default();
    let file = fs.open_file(path, location, fs::OpenFlags { read: true, write: false, create: false }).ok()?;
    let img = image::ImageReader::new(std::io::BufReader::new(file)).with_guessed_format().ok()?.decode().ok()?;
    let rgba = img.into_rgba8();
    Some(Image::from_rgba8(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height())))
}
```

---

## 17. Multi-page routing

`foundation new my-app --template multi-page-app` (or `/foundation-new-page <name>` to add pages).

- `build.rs`: `include_router: true`.
- `ui/app.slint`:

```slint
import { Theme, UISize, Images } from "@ui/theme.slint";
import { Router } from "gen/router.slint";
export { Theme, Images }
export * from "gen/exports.slint";

export component AppWindow inherits Window {
    title: "My App";
    preferred-width: UISize.screen-width;
    preferred-height: UISize.screen-height;
    background: Theme.palette-background;
    Router { }
}
```

- Each page is a directory `ui/pages/<slug>/` with `page.slint` (component named `<Prefix>PagePage`, e.g.
  `SecondPagePage inherits Rectangle`) and `props.slint`:

```slint
// ui/pages/props.slint  (default route)
@rust-attr(route(default, path = "/"))
export struct MainPageProps {}

// ui/pages/second/props.slint
@rust-attr(route(path = "/second"))
export struct SecondPageProps { /* route state fields */ }
```

- Navigation via the generated `Navigate` global (`import { Navigate } from "../gen/navigate.slint";`):
  `Navigate.second-page({ });` pushes, `Navigate.backward();` pops. Exactly one page must be the default route.
  Swipe-back is enabled via `AppContext.config.enable_swipe_back`.
- Rust side can drive the router through `cx.router` (`StoredValue<Router>`) and the generated
  `router_init.rs`.

---

## 18. Theming

- `resources/theme.json` is the source of truth; the CLI compiles it to Rust (`~/.foundation/themes/rust/`)
  and `src/theme.rs` applies it with `foundation_themes::apply_theme!`, following the system light/dark.

```json
{
  "id": "app_theme",
  "name": "My App",
  "parent": "base_theme",
  "tokens": { "light": { "palette-background": "#..." }, "dark": { "...": "..." } },
  "components": { "button": { "variants": { "primary": { "states": { "hover": { } } } } } }
}
```

- `tokens` override colours, spacing, typography, sizing separately for light and dark. `components` override
  per component / variant / state / size (schemas in `ui2/theme-editor/defaults/components/*.schema.json`).
- `foundation theme` opens a visual editor with undo/redo. `foundation themes new <brand> --from base_theme`
  creates a shared brand theme; `foundation themes list` shows inheritable ones.
- **Unrecognised token keys are dropped silently.** If a change does not apply, check spelling first.
- Test with the `kitchen-sink` template in `foundation sim`.

---

## 19. Icons, images, fonts, i18n

- **Icon:** `resources/icon.svg` **exactly 110 × 110 px** or the build fails. Transparent background glyph only:
  the launcher draws the ring/disc, so a filled background shows as a second shape (not checked by the build).
  Optional `resources/icon-dark.svg` for dark mode. Build converts to `icon.bin` / `icon-dark.bin`.
- **Images:** `resources/images/<name>.(svg|png|jpg|jpeg|webp|bmp)` → `Images.image("<name>")`
  (relative path without extension). Converted to `.raw` at build. SDK shared icons/images are searched after
  app resources. Do not symlink SDK resources into your app.
- **Fonts:** `resources/fonts/*.ttf|otf|ttc`, registered before the app starts; reference by real family name.
- **i18n:** `i18n/en.json` (+ `es.json` etc). Set `include_translations: true` in `build.rs` to generate
  `ui/gen/tr.slint` with a `TrId` enum. In Slint: `TR2.lookup(TrId.SomeKey)`; in Rust: `tr::lookup_id(TrId::SomeKey)`.
  Key IDs are dot-notation (`"settings.title"`) → PascalCase variants. `/foundation-localize en es` generates a
  target locale from the source while preserving keys and placeholders.

---

## 20. Reference apps in the KeyOS repo

Clone `https://github.com/Foundation-Devices/KeyOS` and read `apps/`:

| App dir | Learn from it |
|---|---|
| `gui-app-image-viewer` | Smallest full app: file picker, `fs` reads, image decode, Slint models |
| `gui-app-authenticator` (2FA) | Multi-page router, QR scanning via `open_qr_scanner`, `[[qrMatchRules]]`, JSON-backed persistence (`JsonBacked`), workers for crypto, hardware AES via `crypto::use_api!()`, import parsers (Aegis/2FAS/Proton/Google), i18n, timers |
| `gui-app-file-browser` | Airlock / User / USB scopes, directory listing, filesystem events |
| `gui-app-qr-scanner` | How the shared scanner decodes QR / UR / animated UR |
| `gui-app-seed-vault` | Secure storage UX, backups |
| `gui-app-security-keys` | FIDO2 / CTAP flows |
| `gui-app-bitcoin` | Large BDK wallet: PSBT signing, QR/UR, QuantumLink (Foundation-only parts) |
| `gui-app-playground` | UI component sandbox |
| `gui-app-bt-test`, `gui-app-entropy-test`, `gui-app-crypto-perf` | Hardware test apps |
| `app-flux-ethereum`, `app-flux-solana`, `gui-app-emu-flux` | Headless "Flux" child apps (Ledger-style app emulation) |
| `sdk/crates/cli/templates/*` | The three scaffolds |
| `sdk/examples/hello-world` | Bare `println!` crate |
| `test-apps/`, `integration-tests/` | Testing patterns |

Repo-level docs worth reading: `README.md`, `DEVELOPMENT.md` (build KeyOS itself; Ubuntu 22.04/24.04 or Nix),
`REPRODUCIBILITY.md`, `docs/architecture-faq.md` (hardware/memory/panic/IPC facts), `docs/syscalls.md`,
`docs/processes.md`, `docs/usb-architecture.md`, `sdk/docs/foundation-cli.md` (CLI agent guide),
`sdk/docs/MIGRATIONS.md`, `utils/passport-drive/README.md` (MCP server).

Licensing: the repo uses REUSE/SPDX per file. SDK pieces and templates are **MIT**; most OS/app code is
**GPL-3.0-or-later**. Your own app may use any licence (FAQ). Catalog distribution terms are undecided; the
2024 plan was open source + reproducible builds for the Envoy catalog, with direct distribution always allowed.

---

## 21. Testing: simulator, preview, hardware, MCP server

### Simulator (`foundation sim`)

Full hosted KeyOS: callbacks, navigation, permissions prompts, services with mocked drivers, persistent user
disk across restarts. Hosted `os/security` logs "not logged in, request would fail on hardware" instead of
failing, so test seed-gated flows on hardware too.

### Preview (`foundation preview ui/app.slint --auto-reload`)

UI only, hot reload. Needs `ui/gen` from a prior build/sim for routed apps.

### Hardware

1. Unlock Passport Prime, enable USB, connect USB-C.
2. **Settings > Apps > Developer Mode** on. (Disable when done; it waives most security guarantees.)
3. `foundation cert gen "Name"` once; `foundation cert install "Name"` (shows fingerprint + warning; press Allow
   on the host prompt). Cert appears under Settings > Apps > Trusted Publishers.
4. `foundation sideload` (or `--no-run` to upload only). KeyOS launches an app only if its signer is trusted;
   untrusted uploads sit there until a cert is installed.
5. `foundation logs` to stream logs.
6. Users without Developer Mode install a `foundation pack` `.app` via Settings > Apps > Install App from a USB
   drive or the Airlock; they must also have allowed your publisher.

Developer-only apps (Image Viewer, Playground) exist only in development firmware; `xtask` adds them for any
build without `--production-firmware`. Request a dev unit or build from source to use them.

### passport-drive MCP server (`foundation-passport-drive mcp`)

Registered by `.mcp.json` in every new app, so Claude Code / Codex can operate the device over USB-CDC serial
(JSON-RPC 2.0 over stdio). Capabilities: screenshot (480×800 BGRA → PNG), tap, swipe, input text, power button,
read/clear logs, version, process list, upload/launch/close dev apps, cert install. Also low-level: kernel
commands, reboot-to-bootloader (SAM-BA), flash read/write/verify, HID APDU. Approve hardware tool calls
individually, keep the server local, keep PINs/seeds/signing keys out of prompts and logs.

CLI form:

```sh
passport-drive screenshot -o /tmp/screen.png
passport-drive tap 240 400
passport-drive swipe 240 600 240 200 --duration-ms 300 --steps 15
passport-drive logs --max-lines 50 --filter "gui"
passport-drive close-app <pid>
passport-drive run actions.json      # scripted sequences
```

Template `AGENTS.md` rules for agents: prefer `preview` → `sim` → `build`; never run `sideload`, `logs`,
`cert gen`, or `update` unless asked; never generate signing identities silently.

---

## 22. Signing and distribution

- Apps are signed with **cosign2** using a **secp256k1** key and a self-signed X.509 code-signing certificate.
  `foundation cert gen` creates `~/.foundation/signing/<name>/{private.pem, public.pub, *.crt, cosign2.toml}`.
  **Back up `private.pem`**; lose it and you cannot update under that identity.
- One identity per publisher, many apps. A user who allows your cert can run every app you sign. Updates replace
  an installed app only if the same key signed both. Re-signing with a different key = new publisher = app data
  and permission grants are wiped.
- **Fingerprint** = SHA-256 of the compressed 33-byte secp256k1 pubkey, 64 lowercase hex; short form
  `xxxxxxxx…xxxxxxxx`. Publish the full fingerprint on your site / GitHub. Optional DNS TXT convention:
  `_keyos-publisher.<domain> TXT "v=1; k=secp256k1; fp=<hex>"` (not verified by KeyOS in v1).
- **App hash** = SHA-256 of `app.elf` minus its 2048-byte cosign2 header; printed by `foundation build`,
  recorded in `manifest.json` `fileHashes`, shown on device under Settings > Apps > *app* > App Hash.
  Recompute: `tail -c +2049 app.elf | sha256sum` or `tar -xOf myapp.app app.elf | tail -c +2049 | sha256sum`.
  Reproducible builds give identical hashes across machines.
- **Bundle:** `foundation pack` → `<app-name>.app` (gzip tar). Install limit 64 MiB. Installed from Settings
  by the user; no Developer Mode needed, only an allowed publisher.
- **Store:** an opt-in discovery catalog (in Envoy) is on the roadmap, not live. Sideloading stays available.
  Foundation planned the store for end of Q2 2026; no launch confirmed as of Oct 2026.
- Foundation does **not** verify publisher identities in KeyOS v1; the user compares fingerprints out of band.
- Shipping code that handles keys/funds: email hello@foundation.xyz for the security-review checklist.

---

## 23. Limits and gotchas

Platform limits (SDK 1.1.0 / KeyOS 1.4.0):

- **No network at all.** No Wi-Fi, no IP stack. Use a companion phone/desktop app and pass data by QR /
  animated UR, USB mass storage, or Airlock.
- **USB HID / HWI / keyboard driver: "coming", not available.** The community password-manager POC is blocked on
  exactly this. Don't plan on acting as a USB keyboard or WebUSB device in a hackathon.
- **QuantumLink for third-party apps = status + exchange rates only.** No generic BT channel; wallet-sync
  messages require a Foundation signature; raw `bt` crate is a stub.
- **NFC:** status only. NDEF read/write and Keycard flows are Foundation-only.
- **Camera raw frames** need a user grant; QR/UR decoding should go through the system scanner modal.
- **Master seed / PIN / login:** Foundation-only. You get `GetAppSeed` (per-app, user-granted) and `GetRandom`.
- **GPIO / I²C / SPI / DMA:** not available to third-party apps.
- **Install size cap 64 MiB.**
- **Hardware:** 500 MHz Cortex-A5, 128 MB RAM, 480×800 screen, battery powered. Prefer event-driven code;
  avoid busy polling. No CPU throttling, so idle time is what saves power.

Build/tooling gotchas:

- `foundation build/sim/sideload` only work inside `foundation develop`.
- After an SDK update: `foundation clean && foundation build`, `foundation doctor`, restart shell. Use
  `/foundation-sdk-migrate-newest` and `sdk/docs/MIGRATIONS.md` for breaking changes.
- `resources/` **must be committed**. `.foundation-sdk/`, `ui/ui`, `manifest.toml`, `target/` must not be.
- Icon must be **exactly 110×110**; theme token typos are **silently dropped**.
- `getrandom` must stay on **0.2** (`rand` 0.8). 0.3+ is not routed to the TRNG.
- Panics abort; no unwinding, no destructors on panic. Design accordingly.
- Changing `app-id` changes `GetAppSeed` output and your `AppData` directory. Pick it once.
- Changing the signing key makes the device treat the app as a new publisher and wipes its data/grants.
- `foundation doctor` always exits 0; read its output.
- Hosted simulator never fails `GetAppSeed` for "not logged in"; hardware does until unlock.
- USB and Airlock filesystems can vanish mid-operation; handle `Err`.
- `permission_templates.toml` names must exist or the build fails ("missing permission template").
- Windows: WSL2 only and "not yet tested".
- CLI plugins are disabled; ignore `foundation plugin`.
- Don't put Foundation SPDX headers in your own app files.

Documentation inconsistencies to be aware of: product pages say "Palladium" microkernel, docs say "Xous"
(same thing, Palladium is the fork). The capabilities page says QuantumLink is "Stable" while the manifest
exposes only status/rates to third parties. The Get Started page mentions `--template` values differently from
the CLI reference; the repo has exactly three templates. The developer docs still say "public developer
channels coming soon" while a Developers category already exists on the community forum.

---

## 24. App showcase and ideas

Foundation's showcase (https://foundation.xyz/app-showcase) lists 25 concepts/proofs of concept, all
"KeyOS applications built with Rust and Slint" that talk to the outside via WebUSB, QuantumLink, WebSocket or
animated QR (several therefore depend on capabilities not yet open to third parties):

Offline Cake Wallet (Monero spend-key signer), Nostr Event Signer, Password Manager, Liana Signer (descriptor-
bound PSBT signing), Nunchuk Agent Signer (spending-policy co-signer for an AI agent), Spark Signer, Frostsnap
Coordinator (FROST), Social Recovery (Shamir across guardians), Pubky Ring (identity keys, sign-ins over
QuantumLink), Secure Notes (logins, notes, cards, SSH keys), Nunchuk Inheritance (dead-man's switch, heir guide),
x401 Approvals (approve credential release to AI agents), Calculator, Tether Wallet fork, Seed XOR, Graffito
(sign notes into bitcoin txs), OpenPGP Keys, Paper Wallet, Image Viewer, PDF Viewer, Text Editor, Coinjoin
Signer (WabiSabi/Wasabi), Fieldnote (Markdown editor over encrypted storage), SeedQR Tool (transcribe/verify
SeedQR on metal), Verify (SHA-256/512 + OpenPGP release signature checks).

Categories Foundation says it wants: Bitcoin and crypto signing, identity and access control, SSH and FIDO2
keys, credential/password managers, encrypted messaging, secure file workflows, internal approval apps,
"release-with-approval" primitives (the device shows intent on the trusted display and only then releases a
secret/signature). Public starter examples (multisig coordination, FIDO2, password-manager flows) are planned in
`foundation-devices/prime-examples` but not live yet.

Hackathon-friendly ideas that fit today's third-party surface:

- Offline signer for any chain/protocol via QR/UR round trip (Nostr, Ethereum EIP-712, Solana, SSH challenge,
  PGP) with `GetAppSeed`-derived keys and a clear approval screen.
- Encrypted notes / secrets / TOTP-style vault in `AppData`, exported/imported as QR or encrypted file over USB.
- Shamir or Seed-XOR tooling using `os/crypto` (`ShamirSplit/Recover`) and the QR widgets.
- Document/file tools over USB/Airlock: hash verifier, Markdown/PDF/text viewer, dice/entropy generator using
  the TRNG, paper-wallet printer via QR.
- "Approval device" for AI agents: agent posts a request as a QR/animated UR, device shows intent, user
  confirms, device returns a signed response QR.
- Games/utilities that show off haptics + RGB LED + touch.

---

## 25. Support, community, developer units

- Developer portal: https://foundation.xyz/developers (inquiry form: name, email, project, GitHub URL,
  category, details). **Complimentary dev units** for companies, active OSS developers and teams building KeyOS
  apps. Submit finished apps (repo + device screenshots) for the "Built for Passport" showcase.
- Docs: https://docs.foundation.xyz/developers/home/ (Get Started, KeyOS, Building Apps, CLI, Capabilities,
  FAQ, API Reference at https://docs.foundation.xyz/developers/api/).
- Forum: https://community.foundation.xyz (Developers category: https://community.foundation.xyz/c/developers/13).
  Example thread: Password Manager App POC (QnAI Labs, built with Claude + KeyOS SDK; blocked on USB).
- Email: hello@foundation.xyz (SDK feedback, bugs, security-review checklist for key-handling apps).
- GitHub: https://github.com/Foundation-Devices/KeyOS (releases: KeyOS-Releases repo). Issue tracker "coming".
- Responsible disclosure: https://foundation.xyz/responsible-disclosure
- X: https://x.com/foundationhq
- Phishing warning on every Foundation page: emails saying "Your Passport funds are at risk" are scams; Foundation
  never asks for seed words or PINs.

---

## 26. Links and sources

Product and company:
- https://foundation.xyz/products/passport-prime
- https://foundation.xyz/blog/introducing-passport-prime
- https://foundation.xyz/blog/foundation-raises-6-4m-human-authority-hardware-launch
- https://foundation.xyz/2024/12/building-keyos/
- https://foundation.xyz/envoy · https://foundation.xyz/security · https://foundation.xyz/support
- https://foundation.xyz/llms.txt

Developer:
- https://foundation.xyz/developers
- https://foundation.xyz/app-showcase
- https://docs.foundation.xyz/developers/home/
- https://docs.foundation.xyz/developers/get-started/
- https://docs.foundation.xyz/developers/keyos/
- https://docs.foundation.xyz/developers/building-apps/
- https://docs.foundation.xyz/developers/cli/
- https://docs.foundation.xyz/developers/capabilities/
- https://docs.foundation.xyz/developers/faq/
- https://docs.foundation.xyz/developers/api-reference/ · https://docs.foundation.xyz/developers/api/
- https://github.com/Foundation-Devices/KeyOS (README, `sdk/docs/foundation-cli.md`, `sdk/SDK-README.md`,
  `sdk/sdk-build.toml`, `sdk/crates/cli/templates/*`, `api/*/manifest.toml`, `os/app-manifest/src/lib.rs`,
  `docs/architecture-faq.md`, `utils/passport-drive/README.md`, `apps/*`)
- https://sdk.foundation.xyz/latest/install.sh
- https://community.foundation.xyz/t/password-manager-app-poc/1061
- https://community.foundation.xyz/t/keyos-v1-4-0-beta3-is-now-available-for-testing/1219

User docs:
- https://docs.foundation.xyz/prime/home · /prime/setup · /prime/manual-setup · /prime/prime-apps/wallet/ ·
  /prime/prime-apps/vault/ · /firmware-updates · /troubleshooting/prime/ · /glossary/home/

A shallow clone of the KeyOS repo used for this document is at
`/private/tmp/claude-501/-Users-oms-Coding-opensource-foundation-app/7184587f-7077-4013-be58-0639e3022bee/scratchpad/keyos-repo/KeyOS`
(session scratch; re-clone from GitHub for anything lasting).
