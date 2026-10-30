#!/usr/bin/env bash
# Builds workspace library crates for Passport Prime (`armv7a-unknown-xous-elf`) with the SDK's
# pinned nightly toolchain. Proves a crate and all of its dependencies compile for the device,
# including C dependencies such as secp256k1-sys. Linking a full app is `foundation build`'s job.
#
#   scripts/device-build.sh -p fps-spike-xous-crypto
#   scripts/device-build.sh -p fps-signer-core --release
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Same flags `foundation build` passes for hardware builds, minus `-C link-arg=-pie`, which only
# matters when linking an app. `-Zunstable-options` is needed because the target is a custom
# target spec shipped in the SDK sysroot. A separate target dir keeps the nightly device
# artifacts apart from host builds.
exec "$REPO_ROOT/scripts/sdk-shell.sh" bash -c '
  set -euo pipefail
  export RUSTFLAGS="--cfg keyos -C relocation-model=pic -Zunstable-options"
  echo "toolchain: $(rustc --version)"
  cargo build --target armv7a-unknown-xous-elf --target-dir target/device --lib "$@"
' device-build "$@"
