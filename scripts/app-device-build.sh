#!/usr/bin/env bash
# Compiles and links the KeyOS app for Passport Prime, unsigned, as a build check.
#
#   scripts/app-device-build.sh            # release build
#   scripts/app-device-build.sh --verbose  # extra cargo arguments pass through
#
# `foundation build` makes the signed, installable bundle, which needs a publisher identity
# (`foundation cert gen`). This builds the same ELF without one, with the RUSTFLAGS
# `foundation build` uses. It reads the theme module `foundation sim` or `foundation build`
# generates, so run one of those in prime-app once first.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="$root/prime-app"
themes="$app/target/foundation/themes/rust"
if [ ! -f "$themes/app_theme.rs" ]; then
  echo "No generated theme in $themes: run 'foundation sim' in prime-app once first." >&2
  exit 1
fi

exec "$root/scripts/sdk-shell.sh" bash -c '
  cd "$1" && shift
  FOUNDATION_THEMES_RUST_DIR="$PWD/target/foundation/themes/rust" \
  RUSTFLAGS="--cfg keyos -C relocation-model=pic -C link-arg=-pie -Zunstable-options" \
    cargo build --release --target armv7a-unknown-xous-elf --target-dir target/device "$@"
' app-device-build "$app" "$@"
