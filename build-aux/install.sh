#!/usr/bin/env bash
# Installs a native build. PREFIX=~/.local or PREFIX=/usr/local.
set -euo pipefail

PREFIX=${PREFIX:-$HOME/.local}
root=$(cd "$(dirname "$0")/.." && pwd)

[ -x "$root/target/release/runetty" ] || {
  echo "run first: cargo build --release" >&2; exit 1;
}

install -Dm755 "$root/target/release/runetty" "$PREFIX/bin/runetty"
install -Dm644 "$root/data/io.github.umutcanpurtul_avi.runetty.desktop" \
  "$PREFIX/share/applications/io.github.umutcanpurtul_avi.runetty.desktop"
install -Dm644 "$root/data/io.github.umutcanpurtul_avi.runetty.metainfo.xml" \
  "$PREFIX/share/metainfo/io.github.umutcanpurtul_avi.runetty.metainfo.xml"
install -Dm644 "$root/data/icons/io.github.umutcanpurtul_avi.runetty.svg" \
  "$PREFIX/share/icons/hicolor/scalable/apps/io.github.umutcanpurtul_avi.runetty.svg"

command -v gtk-update-icon-cache >/dev/null &&
  gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true
command -v update-desktop-database >/dev/null &&
  update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true

echo "installed: $PREFIX/bin/runetty"
