#!/usr/bin/env bash
# The Flathub manifest is a copy of the development one with a different
# source block. Copies drift, so this checks that nothing else differs.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
local_manifest="$root/build-aux/dev.runetty.Runetty.yaml"
flathub_manifest="$root/build-aux/flathub/dev.runetty.Runetty.yaml"

# Strip comments, the source block of the runetty module, and blank lines:
# what remains must be identical.
strip() {
  sed -e '/^ *#/d' \
      -e '/type: dir/d' \
      -e '/path: \.\./d' \
      -e '/url: https:\/\/github.com\/umutcanpurtul-avi\/runetty.git/d' \
      -e '/tag: v[0-9]/d' \
      -e '/commit: [0-9a-f]\{40\}/d' \
      -e '/^ *- type: git$/d' \
      -e '/^$/d' "$1"
}

if diff -u <(strip "$local_manifest") <(strip "$flathub_manifest"); then
  echo "manifests agree"
else
  echo >&2
  echo "The two manifests differ beyond their source blocks." >&2
  echo "Keep build-aux/flathub/ in step with build-aux/." >&2
  exit 1
fi

# cargo-sources.json is vendored into both; they must be the same file.
if ! cmp -s "$root/build-aux/cargo-sources.json" "$root/build-aux/flathub/cargo-sources.json"; then
  echo >&2 "cargo-sources.json differs between build-aux/ and build-aux/flathub/."
  echo >&2 "Regenerate it and copy it across."
  exit 1
fi

echo "cargo sources agree"
