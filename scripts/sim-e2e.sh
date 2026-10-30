#!/usr/bin/env bash
# End-to-end check of the KeyOS app in the hosted simulator.
#
# Builds the app with `foundation sim`, plays the golden scenario through the file bridge and the
# simulator screen (tools/sim-e2e), restarts the simulator, checks that the app kept what it
# stored, and puts the developer's own simulator seed back. Needs the Foundation SDK (see
# scripts/sdk-shell.sh). Screenshots and simulator logs land in target/sim-e2e/.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/sim-e2e"
export FPS_SIM_DIR="${FPS_SIM_DIR:-$HOME/.fps-signer-sim}"
# shellcheck source=simulator.sh
source "$root/scripts/simulator.sh"
refuse_running_simulator

rm -rf "$out"
mkdir -p "$out"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-sim-e2e
driver="$root/target/debug/fps-sim-e2e"

finish() {
  local status=$?
  stop_simulator
  "$driver" restore || true
  exit "$status"
}
trap finish EXIT

"$driver" prepare
start_simulator first
"$driver" run --shots "$out"
stop_simulator
start_simulator restarted
"$driver" after-restart --shots "$out/restarted"
echo "Simulator end-to-end check passed."
