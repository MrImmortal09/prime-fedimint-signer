#!/usr/bin/env bash
# Prepares third_party/fedimint: fedimint at the pinned tag with this repository's patch series
# applied on a branch named fps-signer.
#
#   scripts/fedimint-checkout.sh           # clone if needed, then apply the series if it changed
#   scripts/fedimint-checkout.sh --export  # write the branch's commits back to patches/fedimint/
#
# Forks of public repositories cannot be private, so the changes to fedimint live here as a patch
# series (patches/fedimint/*.patch) instead of in a fork. Work on the fps-signer branch in the
# checkout, commit there, and export.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tag="v0.13.0-beta.1"
url="https://github.com/fedimint/fedimint.git"
dir="$root/third_party/fedimint"
patches="$root/patches/fedimint"
stamp="$dir/.fps-patches"

has_patches() {
  for f in "$patches"/*.patch; do
    [ -e "$f" ]
    return
  done
}

sha256_hex() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | cut -d' ' -f1
  else
    shasum -a 256 | cut -d' ' -f1
  fi
}

series_hash() {
  if has_patches; then
    cat "$patches"/*.patch | sha256_hex
  else
    echo none
  fi
}

if [ "${1:-}" = "--export" ]; then
  rm -f "$patches"/*.patch
  git -C "$dir" format-patch --quiet --no-signature --zero-commit -o "$patches" "$tag..fps-signer"
  series_hash >"$stamp"
  ls "$patches"
  exit 0
fi

if [ ! -d "$dir/.git" ]; then
  git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" "$url" "$dir"
  echo ".fps-patches" >>"$dir/.git/info/exclude"
fi

if [ "$(cat "$stamp" 2>/dev/null)" = "$(series_hash)" ]; then
  exit 0
fi

if [ -n "$(git -C "$dir" status --porcelain --untracked-files=no)" ]; then
  echo "third_party/fedimint has uncommitted changes; commit or export them first." >&2
  exit 1
fi

git -C "$dir" -c advice.detachedHead=false checkout --quiet --force "$tag"
git -C "$dir" checkout --quiet -B fps-signer
if has_patches; then
  git -C "$dir" -c user.name=fps -c user.email=fps@localhost am --quiet --keep-cr "$patches"/*.patch
fi
series_hash >"$stamp"
