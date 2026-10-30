#!/usr/bin/env bash
# The watch-only wallet against the KeyOS app in the hosted simulator.
#
# Starts the app with `foundation sim` and runs the wallet's simulator test
# (wallet/fps-wallet/tests/simulator.rs): a federation in this process, the app signing, and every
# review decided by an approval swipe or a Reject tap. The scenario runs twice, each on a fresh app:
# over the file bridge, then over the network link, the stand-in for QuantumLink. Then stops the
# simulator and puts the developer's own simulator seed back. Needs the Foundation SDK (see
# scripts/sdk-shell.sh) and the fedimint checkout (scripts/fedimint-checkout.sh). Screenshots and
# the simulator log land in target/sim-wallet/.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/sim-wallet"
export FPS_SIM_DIR="${FPS_SIM_DIR:-$HOME/.fps-signer-sim}"
# shellcheck source=simulator.sh
source "$root/scripts/simulator.sh"
refuse_running_simulator

rm -rf "$out"
mkdir -p "$out"
# fps-sim-e2e pins a known app seed and asks for a clean start, so the app needs no seed consent
# and holds no federation from earlier runs; afterwards it puts the developer's seed back.
cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-sim-e2e
driver="$root/target/debug/fps-sim-e2e"
# From the wallet directory, so its cargo config gives fedimint's fixtures their test timings.
cd "$root/wallet"
wallet=(cargo test -p fps-wallet --test simulator)
"${wallet[@]}" --no-run --quiet

finish() {
  local status=$?
  stop_simulator
  "$driver" restore || true
  exit "$status"
}
trap finish EXIT

for link in bridge net; do
  "$driver" prepare
  start_simulator "wallet-$link"
  mkdir -p "$out/$link"
  echo "== The wallet over the $link link =="
  # The test prints its steps; fedimint's own logging stays at warnings unless RUST_LOG says otherwise.
  RUST_LOG="${RUST_LOG:-warn}" FPS_SIM_LINK="$link" FPS_SIM_SHOTS="$out/$link" \
    "${wallet[@]}" -- --ignored --nocapture
  stop_simulator
done
echo "Wallet end-to-end check against the simulator passed over both links."
