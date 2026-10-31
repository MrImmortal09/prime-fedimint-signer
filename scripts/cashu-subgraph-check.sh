#!/usr/bin/env bash
# Checks that the Cashu workspace compiles fps-mobile's dependencies as `wallet/` does, apart from
# what docs/plan/12_CASHU.md allows ("Rule zero"): four patch bumps that CDK forces, and features
# that CDK adds to six crates. Fails on any other difference.
#
#   scripts/cashu-subgraph-check.sh
#
# Compares `cargo tree -p fps-mobile --target aarch64-apple-ios -e normal,build`, the graph
# `scripts/ios-core.sh` links, in `wallet/` and in `cashu/`, twice: packages and versions, then
# packages with their features. In `cashu/` cargo unifies features over the whole workspace, so the
# second view shows what CDK switches on in fps-mobile's crates. Needs the fedimint checkout
# (scripts/fedimint-checkout.sh), which both workspaces load; changes neither lockfile.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
target=aarch64-apple-ios
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The bumps CDK forces on crates fedimint compiles (parking_lot by cdk-common, the rest follow;
# unicode-normalization by cashu), as `old new`.
bumps=(
  "lock_api v0.4.13|lock_api v0.4.14"
  "parking_lot v0.12.4|parking_lot v0.12.5"
  "parking_lot_core v0.9.11|parking_lot_core v0.9.12"
  "unicode-normalization v0.1.24|unicode-normalization v0.1.26"
)
# The one package the Cashu build adds to fps-mobile's graph: the optional dependency bitcoin's
# `base64` feature (below) switches on.
added_package="base64 v0.21.7"
# Features CDK may add, per crate. bitcoin, ring, uuid, dnssec-prover (`validation`, with the two
# optional dependencies it turns on) and hex_lit as the plan lists them. "regex-syntax `unicode`"
# is CDK's `regex` with its default features reaching the regex crates that fedimint's logging
# uses through `matchers`: regex-syntax gains `unicode`, and regex-automata the engines and
# Unicode tables that `regex` turns on.
allowed_features='
bitcoin base64 rand
ring std
uuid serde v7
dnssec-prover validation bitcoin_hashes hex_lit
hex_lit rust_v_1_46
regex-syntax default unicode unicode-age unicode-bool unicode-case unicode-gencat unicode-perl unicode-script unicode-segment
regex-automata dfa-onepass hybrid meta nfa-backtrack nfa-pikevm perf-inline perf-literal perf-literal-multisubstring perf-literal-substring unicode unicode-age unicode-bool unicode-case unicode-gencat unicode-perl unicode-script unicode-segment unicode-word-boundary
'

# fps-mobile's graph from one workspace, one line per package (and feature set), without local
# paths, which differ between checkouts.
listing() {
  local workspace=$1 format=$2
  (cd "$root/$workspace" && cargo tree --locked --quiet -p fps-mobile --target "$target" \
    -e normal,build --prefix none -f "$format") |
    sed -e 's/ (\*)$//' -e 's# (/[^)]*)##' | LC_ALL=C sort -u
}

listing wallet '{p}' >"$work/wallet-packages.txt"
listing cashu '{p}' >"$work/cashu-packages.txt"
listing wallet '{p} [{f}]' >"$work/wallet-features.txt"
listing cashu '{p} [{f}]' >"$work/cashu-features.txt"

# Both listings are real ones: fps-mobile and fedimint's client are in them, with the old versions
# of the bumped crates on the wallet side.
for side in wallet cashu; do
  grep -qx 'fps-mobile v0.1.0' "$work/$side-packages.txt"
  grep -q '^fedimint-client v' "$work/$side-packages.txt"
done
for bump in "${bumps[@]}"; do
  grep -qx "${bump%|*}" "$work/wallet-packages.txt"
done
echo "fps-mobile for $target: $(wc -l <"$work/wallet-packages.txt" | tr -d ' ') packages in wallet/, $(wc -l <"$work/cashu-packages.txt" | tr -d ' ') in cashu/"

# 1. Packages and versions: the wallet's, with the bumps applied and the one added package.
renames=()
for bump in "${bumps[@]}"; do
  renames+=(-e "s/^${bump%|*}\$/${bump#*|}/")
done
{
  sed "${renames[@]}" "$work/wallet-packages.txt"
  echo "$added_package"
} | LC_ALL=C sort -u >"$work/expected-packages.txt"
if ! diff -u "$work/expected-packages.txt" "$work/cashu-packages.txt" >"$work/packages.diff"; then
  echo "fps-mobile's packages in cashu/ differ from wallet/ beyond the allowed bumps:"
  cat "$work/packages.diff"
  exit 1
fi

# 2. Features: every package keeps its feature sets, and gains only allowed features. Each wallet
# line must have a cashu line of the same package whose features add only allowed ones, and each
# cashu line must come from a wallet line that way, or be the added package. Two wallet copies of a
# crate may become one, when the added features make their sets equal.
renames=()
for bump in "${bumps[@]}"; do
  renames+=(-e "s/^${bump%|*} \\[/${bump#*|} [/")
done
sed "${renames[@]}" "$work/wallet-features.txt" | LC_ALL=C sort -u >"$work/wallet-renamed.txt"
printf '%s\n' "$allowed_features" >"$work/allowed.txt"
awk -v added="$added_package" '
  function parse(line, parts,   at) {
    at = index(line, " [")
    parts["key"] = substr(line, 1, at - 1)
    parts["features"] = substr(line, at + 2, length(line) - at - 2)
    split(parts["key"], words, " ")
    parts["name"] = words[1]
  }
  # Whether the features `to` hold every one of `from` and add only allowed ones for `name`.
  function grows(name, from, to,   have, list, n, i) {
    delete have
    n = split(to, list, ",")
    for (i = 1; i <= n; i++) if (list[i] != "") have[list[i]] = 1
    n = split(from, list, ",")
    for (i = 1; i <= n; i++) {
      if (list[i] == "") continue
      if (!(list[i] in have)) return 0
      delete have[list[i]]
    }
    for (feature in have) if (!((name SUBSEP feature) in allowed)) return 0
    return 1
  }
  FILENAME == ARGV[1] {
    for (i = 2; i <= NF; i++) allowed[$1, $i] = 1
    next
  }
  FILENAME == ARGV[2] { wallet[++w] = $0; next }
  { cashu[++c] = $0 }
  END {
    bad = 0
    for (i = 1; i <= w; i++) {
      parse(wallet[i], a)
      found = 0
      for (j = 1; j <= c && !found; j++) {
        parse(cashu[j], b)
        if (a["key"] == b["key"] && grows(a["name"], a["features"], b["features"])) found = 1
      }
      if (!found) { print "  wallet/ has, cashu/ does not: " wallet[i]; bad = 1 }
    }
    for (j = 1; j <= c; j++) {
      parse(cashu[j], b)
      if (b["key"] == added) continue
      found = 0
      for (i = 1; i <= w && !found; i++) {
        parse(wallet[i], a)
        if (a["key"] == b["key"] && grows(a["name"], a["features"], b["features"])) found = 1
      }
      if (!found) { print "  cashu/ has, wallet/ does not: " cashu[j]; bad = 1 }
    }
    exit bad
  }
' "$work/allowed.txt" "$work/wallet-renamed.txt" "$work/cashu-features.txt" >"$work/features.diff" || {
  echo "fps-mobile's features in cashu/ differ from wallet/ beyond the allowed additions:"
  cat "$work/features.diff"
  exit 1
}

echo "Only the allowed differences (< wallet/, > cashu/):"
LC_ALL=C diff "$work/wallet-features.txt" "$work/cashu-features.txt" | grep '^[<>]' | sed 's/^/  /' || true
