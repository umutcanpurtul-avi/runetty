#!/usr/bin/env bash
# Installs runetty's build dependencies and verifies the version floor.
# Supported package managers: pacman, apt, dnf, zypper.
set -euo pipefail

MIN_GTK=4.14
MIN_ADW=1.5
MIN_VTE=0.76
MIN_SECRET=0.20
MIN_RUST=1.92

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m==> %s\033[0m\n' "$*" >&2; }
die()  { printf '\033[31m==> %s\033[0m\n' "$*" >&2; exit 1; }

sudo_cmd() {
  if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi
}

detect_family() {
  [ -r /etc/os-release ] || die "cannot read /etc/os-release."
  # shellcheck disable=SC1091
  . /etc/os-release
  case " ${ID:-} ${ID_LIKE:-} " in
    *" arch "*|*" cachyos "*|*" endeavouros "*|*" manjaro "*) echo arch ;;
    *" debian "*|*" ubuntu "*)                                echo debian ;;
    *" fedora "*|*" rhel "*|*" centos "*|*" rocky "*|*" almalinux "*) echo rhel ;;
    *" suse "*|*" opensuse "*)                                echo suse ;;
    *) die "Unrecognised distribution: ${ID:-unknown}. Install packages manually (see BUILDING.md)." ;;
  esac
}

install_arch() {
  sudo_cmd pacman -S --needed --noconfirm \
    base-devel pkgconf rust gtk4 libadwaita vte4 libsecret \
    inter-font ttf-jetbrains-mono
}

install_debian() {
  sudo_cmd apt-get update
  # rustc/cargo are deliberately absent: the Debian and Ubuntu packages are
  # far below the Rust 1.92 that gtk4-rs needs. Use rustup instead.
  sudo_cmd apt-get install -y --no-install-recommends \
    build-essential pkg-config \
    libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libsecret-1-dev \
    fonts-inter fonts-jetbrains-mono || \
  sudo_cmd apt-get install -y --no-install-recommends \
    build-essential pkg-config \
    libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libsecret-1-dev
}

install_rhel() {
  # vte291-gtk4-devel exists on Fedora but not RHEL/Rocky 9, nor in EPEL.
  sudo_cmd dnf install -y \
    gcc pkgconf-pkg-config \
    gtk4-devel libadwaita-devel libsecret-devel || true
  sudo_cmd dnf install -y vte291-gtk4-devel || \
    warn "vte291-gtk4-devel not found; use Flatpak for terminal support."
}

install_suse() {
  sudo_cmd zypper --non-interactive install -y \
    gcc pkg-config \
    gtk4-devel libadwaita-devel vte-gtk4-devel libsecret-devel
}

# $1 module, $2 minimum version -> 0 fine, 1 too old, 2 missing
check() {
  local have
  have=$(pkg-config --modversion "$1" 2>/dev/null) || { echo "    $1: yok"; return 2; }
  if printf '%s\n%s\n' "$2" "$have" | sort -V -C; then
    printf '    %-16s %s  (>= %s)\n' "$1" "$have" "$2"
    return 0
  fi
  printf '    %-16s %s  (ESKI, >= %s gerekiyor)\n' "$1" "$have" "$2"
  return 1
}

check_rust() {
  local have
  have=$(rustc --version 2>/dev/null | awk '{print $2}') || true
  if [ -z "${have:-}" ]; then
    printf '    %-16s %s\n' rustc "missing"
    return 1
  fi
  if printf '%s\n%s\n' "$MIN_RUST" "$have" | sort -V -C; then
    printf '    %-16s %s  (>= %s)\n' rustc "$have" "$MIN_RUST"
    return 0
  fi
  printf '    %-16s %s  (ESKI, >= %s gerekiyor)\n' rustc "$have" "$MIN_RUST"
  return 1
}

main() {
  local family
  family=$(detect_family)
  say "Distribution family: $family"

  case $family in
    arch)   install_arch ;;
    debian) install_debian ;;
    rhel)   install_rhel ;;
    suse)   install_suse ;;
  esac

  say "Version check"
  local ok=0
  check gtk4          "$MIN_GTK"    || ok=1
  check libadwaita-1  "$MIN_ADW"    || ok=1
  check libsecret-1   "$MIN_SECRET" || ok=1
  check vte-2.91-gtk4 "$MIN_VTE"    || ok=2

  check_rust || ok=3

  case $ok in
    0) say "Ready:  cargo build --release" ;;
    3) warn "Rust is too old. Use rustup rather than the distribution package:"
       warn "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
       warn "    rustup default stable" ;;
    2) warn "VTE is missing or too old. Build without the terminal:"
       warn "    cargo build --release --no-default-features"
       warn "For the full build use Flatpak: flatpak install flathub dev.runetty.Runetty" ;;
    *) die  "GTK or libadwaita is too old. Use Flatpak on this distribution (see BUILDING.md)." ;;
  esac
}

main "$@"
