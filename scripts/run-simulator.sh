#!/usr/bin/env bash
# Run one simulator and clean up its entire process group on exit.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
SDK_ROOT="${FOUNDATION_SDK_ROOT:-$HOME/.foundation/sdk/current}"
if pgrep -f 'lib/keyos/simulator/xous/kernel/keyos-kernel' >/dev/null; then
  echo 'A simulator is already running. Close it before starting another.' >&2
  exit 1
fi
# Copying over an executed Mach-O can leave macOS with cached signature pages.
# Move the old stage aside so the SDK writes the new app onto a fresh inode.
stage="$SDK_ROOT/lib/keyos/simulator/sideloaded-apps/f73ee1e71ece205f0008ff41f7bc3cc4/app.elf"
if [ -f "$stage" ]; then mv -f "$stage" "$stage.previous"; fi
sim_pid=''
finish() {
  trap - EXIT INT TERM
  if [ -n "$sim_pid" ]; then
    kill -TERM -- "-$sim_pid" 2>/dev/null || true
    wait "$sim_pid" 2>/dev/null || true
  fi
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
set -m
"$root/scripts/sdk-shell.sh" bash -c 'cd "$1/prime-app" && exec foundation sim' _ "$root" &
sim_pid=$!
set +m
wait "$sim_pid"
