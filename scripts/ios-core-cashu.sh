#!/usr/bin/env bash
# Builds the phone wallet's Rust core with Cashu and packages it for the Xcode project in ios/:
# fps-mobile (unchanged) and fps-cashu as one static library, from the umbrella crate
# cashu/fps-core, for iPhones and the iOS Simulator (docs/plan/12_CASHU.md, "The phone").
#
#   scripts/ios-core-cashu.sh [--release]
#
# Writes:
# - ios/FpsMobile.xcframework: the library for aarch64-apple-ios and aarch64-apple-ios-sim, with
#   both C headers and one module.modulemap declaring fps_mobileFFI and fps_cashuFFI. It has the
#   name scripts/ios-core.sh gives its Fedimint-only core, so the Xcode project needs no change.
# - ios/PrimeWallet/Generated/fps_cashu.swift: the Cashu bindings inside
#   `#if canImport(fps_cashuFFI)`, so a core from scripts/ios-core.sh still builds today's app.
#
# Both namespaces' bindings come from the device library, in library mode, run from cashu/.
# fps_mobile.swift generated beside them must be byte-identical to the tracked
# ios/PrimeWallet/Generated/fps_mobile.swift, because fps-mobile is linked unchanged; otherwise the
# script fails before it writes anything.
#
# Debug by default, for speed, with line-table debug info only (CARGO_PROFILE_DEV_DEBUG overrides
# it) to keep the libraries and the target directory small. Pass --release for an optimised build.
# Needs Xcode and the fedimint checkout (scripts/fedimint-checkout.sh), and adds the Rust iOS
# targets if they are missing. Honours CARGO_TARGET_DIR. UNIFFI_BINDGEN names a uniffi-bindgen
# 0.32.2 to use; by default it is the one scripts/ios-core.sh builds from fps-mobile's `bindgen`
# feature, built the same way if it is missing.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
profile=debug
flag=
case "${1:-}" in
  "") ;;
  --release)
    profile=release
    flag=--release
    ;;
  *)
    echo "usage: $0 [--release]" >&2
    exit 2
    ;;
esac
targets=(aarch64-apple-ios aarch64-apple-ios-sim)
bindgen_version="uniffi-bindgen 0.32.2"
generated="$root/ios/PrimeWallet/Generated"
# The app's minimum iOS version, on the device and in the simulator. Rust links for iOS 10
# otherwise, which lacks symbols the C dependencies use (___chkstk_darwin).
export IPHONEOS_DEPLOYMENT_TARGET=16.0
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-line-tables-only}"

# A workspace's target directory, as cargo resolves it (CARGO_TARGET_DIR, config, or ./target).
target_directory() {
  (cd "$1" && cargo metadata --format-version 1 --no-deps) |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
}

cd "$root/cashu"
target_dir="$(target_directory "$root/cashu")"
if [ -z "$target_dir" ]; then
  echo "cargo metadata in cashu/ gave no target directory" >&2
  exit 1
fi

rustup target add "${targets[@]}" >/dev/null
for target in "${targets[@]}"; do
  echo "Building the core with Cashu for $target ($profile)"
  # A static library only for iOS, asked of cargo directly, as scripts/ios-core.sh does. fps-cashu
  # exports its UniFFI API only with its `uniffi` feature.
  cargo rustc --quiet $flag -p fps-core --lib --crate-type staticlib --target "$target" \
    --features fps-cashu/uniffi
done
device_library="$target_dir/aarch64-apple-ios/$profile/libfps_core.a"
simulator_library="$target_dir/aarch64-apple-ios-sim/$profile/libfps_core.a"

bindgen="${UNIFFI_BINDGEN:-}"
if [ -z "$bindgen" ]; then
  wallet_target_dir="$(target_directory "$root/wallet")"
  for candidate in "$target_dir/debug/uniffi-bindgen" "$wallet_target_dir/debug/uniffi-bindgen"; do
    if [ -x "$candidate" ] && [ "$("$candidate" --version 2>/dev/null)" = "$bindgen_version" ]; then
      bindgen="$candidate"
      break
    fi
  done
  if [ -z "$bindgen" ]; then
    echo "Building $bindgen_version from fps-mobile's bindgen feature, as scripts/ios-core.sh does"
    (cd "$root/wallet" && cargo build --quiet -p fps-mobile --features bindgen --bin uniffi-bindgen)
    bindgen="$wallet_target_dir/debug/uniffi-bindgen"
  fi
fi
if [ "$("$bindgen" --version 2>/dev/null)" != "$bindgen_version" ]; then
  echo "$bindgen is not $bindgen_version, the UniFFI fps-mobile and fps-cashu use" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Library mode finds each crate's uniffi.toml through cargo metadata in the working directory, so
# it runs here, in the workspace that built the library.
"$bindgen" generate --library "$device_library" --language swift --out-dir "$work/bindings" \
  --no-format
for file in fps_mobile.swift fps_mobileFFI.h fps_mobileFFI.modulemap \
  fps_cashu.swift fps_cashuFFI.h fps_cashuFFI.modulemap; do
  if [ ! -f "$work/bindings/$file" ]; then
    echo "uniffi-bindgen wrote no $file. Does fps-core re-export fps-cashu, built with its" >&2
    echo "uniffi feature?" >&2
    exit 1
  fi
done
if ! cmp -s "$work/bindings/fps_mobile.swift" "$generated/fps_mobile.swift"; then
  echo "fps_mobile.swift from the combined core differs from the tracked" >&2
  echo "ios/PrimeWallet/Generated/fps_mobile.swift; fps-mobile must be linked unchanged:" >&2
  diff -u "$generated/fps_mobile.swift" "$work/bindings/fps_mobile.swift" | head -n 60 >&2 || true
  exit 1
fi

mkdir -p "$work/headers"
cp "$work/bindings/fps_mobileFFI.h" "$work/bindings/fps_cashuFFI.h" "$work/headers/"
# Xcode finds a framework's Clang modules only in a file of this name: one file, both modules.
{
  cat "$work/bindings/fps_mobileFFI.modulemap"
  echo
  cat "$work/bindings/fps_cashuFFI.modulemap"
} >"$work/headers/module.modulemap"

{
  echo "// Written by scripts/ios-core-cashu.sh. Only a core from that script declares the"
  echo "// fps_cashuFFI module; with one from scripts/ios-core.sh this file compiles to nothing."
  echo "#if canImport(fps_cashuFFI)"
  cat "$work/bindings/fps_cashu.swift"
  echo
  echo "#endif // canImport(fps_cashuFFI)"
} >"$work/fps_cashu.swift"

rm -rf "$root/ios/FpsMobile.xcframework"
xcodebuild -create-xcframework \
  -library "$device_library" -headers "$work/headers" \
  -library "$simulator_library" -headers "$work/headers" \
  -output "$root/ios/FpsMobile.xcframework" >/dev/null
mkdir -p "$generated"
cp "$work/fps_cashu.swift" "$generated/fps_cashu.swift"
echo "Wrote ios/FpsMobile.xcframework (device and simulator, Fedimint and Cashu) and"
echo "ios/PrimeWallet/Generated/fps_cashu.swift"
