#!/usr/bin/env bash
# Builds the phone wallet's Rust core for Android on a Linux machine.
#
# Writes libfps_mobile.so for each ABI to wallet/target/android/jniLibs/<abi>/, or under --out
# (an app's src/main/jniLibs, say), and regenerates the Kotlin bindings in
# wallet/fps-mobile/bindings/kotlin/. Builds arm64-v8a (phones) and x86_64 (the emulator) unless
# --abi names others. Pass --debug for a faster, larger build.
#
#   scripts/android-core.sh
#   scripts/android-core.sh --abi arm64-v8a --out ../fedimint-phone/app/src/main/jniLibs
#
# Needs the Android NDK (ANDROID_NDK_HOME, or the newest one under $ANDROID_HOME/ndk), cargo-ndk
# (cargo install cargo-ndk), a C compiler, CMake, make or Ninja and libclang (on Debian and Ubuntu:
# apt install build-essential cmake libclang-dev), and the fedimint checkout
# (scripts/fedimint-checkout.sh). Adds the Rust Android targets if they are missing.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
profile=release
flag=--release
abis=()
out="$root/wallet/target/android/jniLibs"
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) profile=debug; flag= ;;
    --abi) abis+=("${2:?--abi needs an ABI}"); shift ;;
    --out) out="${2:?--out needs a directory}"; shift ;;
    *) echo "usage: $0 [--debug] [--abi ABI]... [--out DIR]" >&2; exit 2 ;;
  esac
  shift
done
[ ${#abis[@]} -gt 0 ] || abis=(arm64-v8a x86_64)

targets=()
for abi in "${abis[@]}"; do
  case "$abi" in
    arm64-v8a) targets+=(aarch64-linux-android) ;;
    armeabi-v7a) targets+=(armv7-linux-androideabi) ;;
    x86_64) targets+=(x86_64-linux-android) ;;
    x86) targets+=(i686-linux-android) ;;
    *) echo "Unknown ABI $abi: use arm64-v8a, armeabi-v7a, x86_64 or x86" >&2; exit 2 ;;
  esac
done

missing=()
for tool in cargo-ndk cmake; do
  command -v "$tool" >/dev/null || missing+=("$tool")
done
command -v make >/dev/null || command -v ninja >/dev/null || missing+=("make or ninja")
if [ ${#missing[@]} -gt 0 ]; then
  echo "Missing: ${missing[*]}" >&2
  exit 1
fi
if [ ! -d "$root/third_party/fedimint" ]; then
  echo "No fedimint checkout; run scripts/fedimint-checkout.sh first" >&2
  exit 1
fi

sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$ndk" ] && [ -d "$sdk/ndk" ]; then
  ndk="$sdk/ndk/$(ls "$sdk/ndk" | sort -V | tail -n 1)"
fi
if [ ! -f "$ndk/source.properties" ]; then
  echo "No Android NDK: set ANDROID_NDK_HOME, or install one under $sdk/ndk" >&2
  exit 1
fi
# cargo-ndk reads ANDROID_NDK_HOME. fedimint-connectors builds aws-lc-sys with bindgen, which sends
# it through CMake, and CMake's Android support finds the NDK only through ANDROID_NDK_ROOT.
export ANDROID_NDK_HOME="$ndk" ANDROID_NDK_ROOT="$ndk"

mkdir -p "$out"
out="$(cd "$out" && pwd)"
rustup target add "${targets[@]}" >/dev/null
cd "$root/wallet"
ndk_targets=()
for abi in "${abis[@]}"; do
  ndk_targets+=(-t "$abi")
done
echo "Building the core for ${abis[*]} ($profile) with NDK $(basename "$ndk")"
cargo ndk "${ndk_targets[@]}" build --quiet $flag -p fps-mobile
# Not cargo-ndk's --output-dir: it also copies the cdylibs that iroh and redb build, which the app
# does not load.
for i in "${!abis[@]}"; do
  mkdir -p "$out/${abis[$i]}"
  cp "target/${targets[$i]}/$profile/libfps_mobile.so" "$out/${abis[$i]}/"
done

# The bindings come from the metadata UniFFI embeds in the library, which is the same for every ABI.
cargo run --quiet -p fps-mobile --features bindgen --bin uniffi-bindgen -- \
  generate --library "target/${targets[0]}/$profile/libfps_mobile.so" --language kotlin \
  --out-dir fps-mobile/bindings/kotlin --no-format
echo "Wrote $out and wallet/fps-mobile/bindings/kotlin"
