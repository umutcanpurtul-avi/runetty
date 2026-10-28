#!/usr/bin/env bash
# runetty gelistirme bagimliliklarini kurar ve surum tabanini dogrular.
# Desteklenen paket yoneticileri: pacman, apt, dnf, zypper.
set -euo pipefail

MIN_GTK=4.14
MIN_ADW=1.5
MIN_VTE=0.76

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m==> %s\033[0m\n' "$*" >&2; }
die()  { printf '\033[31m==> %s\033[0m\n' "$*" >&2; exit 1; }

sudo_cmd() {
  if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi
}

detect_family() {
  [ -r /etc/os-release ] || die "/etc/os-release okunamadi."
  # shellcheck disable=SC1091
  . /etc/os-release
  case " ${ID:-} ${ID_LIKE:-} " in
    *" arch "*|*" cachyos "*|*" endeavouros "*|*" manjaro "*) echo arch ;;
    *" debian "*|*" ubuntu "*)                                echo debian ;;
    *" fedora "*|*" rhel "*|*" centos "*|*" rocky "*|*" almalinux "*) echo rhel ;;
    *" suse "*|*" opensuse "*)                                echo suse ;;
    *) die "Taninmayan dagitim: ${ID:-bilinmiyor}. Paketleri elle kurun (bkz. BUILDING.md)." ;;
  esac
}

install_arch() {
  sudo_cmd pacman -S --needed --noconfirm \
    base-devel pkgconf rust gtk4 libadwaita vte4 \
    inter-font ttf-jetbrains-mono
}

install_debian() {
  sudo_cmd apt-get update
  sudo_cmd apt-get install -y --no-install-recommends \
    build-essential pkg-config cargo rustc \
    libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev \
    fonts-inter fonts-jetbrains-mono || \
  sudo_cmd apt-get install -y --no-install-recommends \
    build-essential pkg-config cargo rustc \
    libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev
}

install_rhel() {
  # vte291-gtk4-devel Fedora'da var; RHEL/Rocky 9'da yok. EPEL de saglamiyor.
  sudo_cmd dnf install -y \
    gcc pkgconf-pkg-config cargo rust \
    gtk4-devel libadwaita-devel || true
  sudo_cmd dnf install -y vte291-gtk4-devel || \
    warn "vte291-gtk4-devel bulunamadi; terminal destegi icin Flatpak kullanin."
}

install_suse() {
  sudo_cmd zypper --non-interactive install -y \
    gcc pkg-config cargo rust \
    gtk4-devel libadwaita-devel vte-gtk4-devel
}

# $1 modul, $2 asgari surum -> 0 uygun, 1 eski, 2 yok
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

main() {
  local family
  family=$(detect_family)
  say "Dagitim ailesi: $family"

  case $family in
    arch)   install_arch ;;
    debian) install_debian ;;
    rhel)   install_rhel ;;
    suse)   install_suse ;;
  esac

  say "Surum kontrolu"
  local ok=0
  check gtk4          "$MIN_GTK" || ok=1
  check libadwaita-1  "$MIN_ADW" || ok=1
  check vte-2.91-gtk4 "$MIN_VTE" || ok=2

  case $ok in
    0) say "Hazir:  cargo build --release" ;;
    2) warn "VTE yok ya da eski. Terminalsiz derleyin:"
       warn "    cargo build --release --no-default-features"
       warn "Tam surum icin Flatpak: flatpak install flathub dev.runetty.Runetty" ;;
    *) die  "GTK ya da libadwaita cok eski. Bu dagitimda Flatpak kullanin (bkz. BUILDING.md)." ;;
  esac
}

main "$@"
