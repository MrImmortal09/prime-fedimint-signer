#!/usr/bin/env bash
# Builds the phone wallet's Rust core for iPhones and packages it for the Xcode project in ios/.
#
# Produces ios/FpsMobile.xcframework (the static library with the C header UniFFI generates) and
# ios/PrimeWallet/Generated/fps_mobile.swift (the Swift bindings). Needs Xcode and the fedimint
# checkout (scripts/fedimint-checkout.sh); adds the Rust iOS target if it is missing. Pass --debug
# for a faster, larger build.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
profile=release
flag=--release
if [ "${1:-}" = "--debug" ]; then
  profile=debug
  flag=
fi
target=aarch64-apple-ios
# The app's minimum iOS version. Rust links for iOS 10 otherwise, which lacks symbols the C
# dependencies use (___chkstk_darwin).
export IPHONEOS_DEPLOYMENT_TARGET=16.0

rustup target add "$target" >/dev/null
cd "$root/wallet"
echo "Building the core for $target ($profile)"
# A static library only for iOS, asked of cargo directly, so the crate's own types (and so the
# Android and JVM builds) stay as they are.
cargo rustc --quiet $flag -p fps-mobile --lib --crate-type staticlib --target "$target"
library="target/$target/$profile/libfps_mobile.a"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo run --quiet -p fps-mobile --features bindgen --bin uniffi-bindgen -- \
  generate --library "$library" --language swift --out-dir "$work/bindings" --no-format
mkdir -p "$work/headers"
cp "$work/bindings/fps_mobileFFI.h" "$work/headers/"
# Xcode finds a framework's Clang module only under this name.
cp "$work/bindings/fps_mobileFFI.modulemap" "$work/headers/module.modulemap"

rm -rf "$root/ios/FpsMobile.xcframework"
xcodebuild -create-xcframework -library "$library" -headers "$work/headers" \
  -output "$root/ios/FpsMobile.xcframework" >/dev/null
mkdir -p "$root/ios/PrimeWallet/Generated"
cp "$work/bindings/fps_mobile.swift" "$root/ios/PrimeWallet/Generated/fps_mobile.swift"
echo "Wrote ios/FpsMobile.xcframework and ios/PrimeWallet/Generated/fps_mobile.swift"
