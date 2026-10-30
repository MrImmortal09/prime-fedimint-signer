# Simulator screenshot capture

`0001-passport-drive-simulator-rgba.patch` applies to Foundation-Devices/KeyOS
commit `4ea7bc95399001eee79fc8b3390a6419a4624697`. It fixes the capture tool,
not the wallet theme or screen detector.

The [capture API](https://github.com/Foundation-Devices/KeyOS/blob/4ea7bc95399001eee79fc8b3390a6419a4624697/api/gui-server/src/msg/capture.rs)
specifies RGBA8888 for simulator frames and BGRA8888 for hardware. The unpatched
CLI and MCP screenshot handlers pass both through the BGRA decoder. The patch
selects the format from the transport and adds tests for both formats and truncated
frames. It does not change USB/hardware decoding. The patch retains the upstream
GPL-3.0-or-later license.

Apply it in a KeyOS source checkout at that commit and build the capture tool
with the SDK-pinned nightly compiler available in your development shell:

```sh
git apply /absolute/path/to/0001-passport-drive-simulator-rgba.patch
cargo test -p passport-drive screenshot::tests
cargo build -p passport-drive
```

From this wallet repository, use the corrected tool with the existing simulator test:

```sh
FPS_SIM_DRIVE=/absolute/path/to/KeyOS/target/debug/passport-drive \
  scripts/sdk-shell.sh scripts/sim-wallet-e2e.sh
```

The format mismatch was reproduced with SDK 1.1.0 in its standard pinned Nix
shell. A temporary pure red/green/blue row produced `(0,0,255)`, `(0,255,0)`,
`(255,0,0)` through the unpatched tool and the correct `(255,0,0)`, `(0,255,0)`,
`(0,0,255)` through the patched tool. The application reported its primary color
as `(214,154,120)` in both runs; the unpatched PNG encoded `(120,154,214)`.
The probe is not included in the application.

The wallet detector intentionally accepts only the configured RGB accent colors;
it does not accept a swapped blue accent. No palette inversion or automatic
image-based format guessing is used.
