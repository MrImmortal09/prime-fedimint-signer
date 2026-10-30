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
kernel="lib/keyos/simulator/xous/kernel/keyos-kernel"

if pgrep -f "$kernel" >/dev/null; then
  echo "A KeyOS simulator is already running; close it first." >&2
  exit 1
fi

rm -rf "$out"
mkdir -p "$out"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-sim-e2e
driver="$root/target/debug/fps-sim-e2e"

sim_pid=""

start_simulator() {
  local log="$out/simulator-$1.log"
  # Job control gives the simulator its own process group, so stopping it stops all of it.
  set -m
  "$root/scripts/sdk-shell.sh" bash -c "cd '$root/prime-app' && exec foundation sim" >"$log" 2>&1 &
  sim_pid=$!
  set +m
  for _ in $(seq 1 1200); do
    if grep -q "simulator bridge watching" "$log"; then
      return 0
    fi
    if ! kill -0 "$sim_pid" 2>/dev/null; then
      echo "The simulator exited before the app started:" >&2
      tail -30 "$log" >&2
      return 1
    fi
    sleep 0.5
  done
  echo "The app did not start within 10 minutes:" >&2
  tail -30 "$log" >&2
  return 1
}

stop_simulator() {
  if [ -n "$sim_pid" ]; then
    kill -TERM -- "-$sim_pid" 2>/dev/null || true
    for _ in $(seq 1 40); do
      pgrep -f "$kernel" >/dev/null || break
      sleep 0.25
    done
    kill -KILL -- "-$sim_pid" 2>/dev/null || true
    sim_pid=""
  fi
}

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
