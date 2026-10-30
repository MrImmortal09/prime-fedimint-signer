#!/usr/bin/env bash
# Runs a command inside the Foundation SDK development shell, non-interactively.
#
#   scripts/sdk-shell.sh foundation doctor
#   scripts/sdk-shell.sh bash -c 'cd prime-app && foundation build'
#
# `foundation develop` opens an interactive shell; this does the same `nix develop` with a command,
# which is what scripts and CI need. FOUNDATION_SDK_ROOT defaults to the installed SDK.
set -euo pipefail

SDK_ROOT="${FOUNDATION_SDK_ROOT:-$HOME/.foundation/sdk/current}"
if [ ! -f "$SDK_ROOT/flake.nix" ]; then
  echo "Foundation SDK not found at $SDK_ROOT (set FOUNDATION_SDK_ROOT)" >&2
  exit 1
fi
if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

# The shell hook defaults FOUNDATION_SDK_ROOT to $PWD, so it has to be set explicitly.
export FOUNDATION_SDK_ROOT="$SDK_ROOT"
exec nix develop "$SDK_ROOT" --command "$@"
