# Building and distribution

## The decision

**Flatpak is the primary distribution format; native builds come second.**

The reason fits in one table: GTK4 and libadwaita versions span four years
across distributions. Dropping to the common denominator (GTK 4.6 /
libadwaita 1.1) would take `OverlaySplitView`, `ToolbarView`,
`PreferencesDialog`, `SwitchRow` and `SpinRow` off the table — that is half
the plan. Pinning the runtime removes the problem entirely.

The floor for native builds is **Ubuntu 24.04 LTS**: GTK 4.14 / libadwaita
1.5 / VTE 0.76. That triple is enforced together by the `v4_14` / `v1_5` /
`v0_76` feature flags in `Cargo.toml` and the runtime check in
`src/config.rs`.

## Distribution matrix

| Distribution | GTK4 | libadwaita | vte-gtk4 | Native build | Flatpak |
| --- | --- | --- | --- | --- | --- |
| Arch / CachyOS / EndeavourOS | 4.24 | 1.10 | 0.84 | full | full |
| Fedora 40+ | 4.14+ | 1.5+ | yes | full | full |
| Ubuntu 24.04 LTS (noble) | 4.14.5 | 1.5.0 | 0.76.0 | full (the floor) | full |
| Ubuntu 25.04+ | 4.18+ | 1.7+ | 0.78+ | full | full |
| Debian 13 (trixie) | 4.18.6 | 1.7.6 | 0.80.1 | full | full |
| Debian 12 (bookworm) | 4.8.3 | 1.2.2 | 0.70.6 | **no** | full |
| Ubuntu 22.04 LTS (jammy) | 4.6.9 | 1.1.7 | none | **no** | full |
| Rocky / RHEL 10 | 4.16 | 1.6 | none | shell only `*` | full |
| Rocky / RHEL 9 | 4.6 | none/old | none | **no** | full |
| openSUSE Tumbleweed | current | current | yes | full | full |

`*` Rocky 10 has new enough GTK and libadwaita but no `vte291-gtk4` package,
so `--no-default-features` builds the shell without a terminal. Use Flatpak
for the terminal.

Versions in the matrix were verified against the distributions' own archives
(Launchpad, packages.debian.org). Anything below them goes to Flatpak.

## Flatpak (every distribution)

```bash
flatpak install -y flathub org.gnome.Platform//50 org.gnome.Sdk//50 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08
pip install --user aiohttp toml tomlkit   # for flatpak-cargo-generator
curl -LO https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
flatpak-builder --user --install --force-clean build build-aux/dev.runetty.Runetty.yaml
flatpak run dev.runetty.Runetty
```

The runtime is pinned to GNOME 50: the same GTK 4.20 / libadwaita 1.8 on
every distribution. VTE is built from source at 0.78.4 inside the manifest,
so the host's version is irrelevant.

Build directory: keep it on the same filesystem as `.flatpak-builder`, or
flatpak-builder fails with "Invalid cross-device link".

## Native build

```bash
./build-aux/install-deps.sh     # detects pacman / apt / dnf / zypper
cargo build --release
./build-aux/install.sh          # PREFIX=~/.local by default
```

The script finishes by comparing `pkg-config` versions against the floor and
points you at Flatpak when they fall short.

### Systems without vte-gtk4

```bash
cargo build --release --no-default-features
```

Terminal tabs are replaced by an explanatory label; everything else works.
Enough to work on the interface on Rocky 10 and similar.

### Package lists

| Family | Packages |
| --- | --- |
| Arch | `base-devel pkgconf rust gtk4 libadwaita vte4 libsecret inter-font ttf-jetbrains-mono` |
| Debian / Ubuntu | `build-essential pkg-config libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libsecret-1-dev fonts-inter fonts-jetbrains-mono` |
| Fedora / RHEL | `gcc pkgconf-pkg-config gtk4-devel libadwaita-devel libsecret-devel vte291-gtk4-devel` |
| openSUSE | `gcc pkg-config gtk4-devel libadwaita-devel vte-gtk4-devel libsecret-devel` |

### Rust version

**Rust 1.92 or newer is required** (gtk4-sys's MSRV). The packaged `rustc` on
Debian, Ubuntu and RHEL is below that, which is why it is absent from the
package lists — use rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

Arch ships a current package; in Flatpak the `rust-stable` SDK extension
handles it.

## Passwords

Saved passwords go to the system keyring (Secret Service) — GNOME Keyring, or
KWallet on KDE. No password is written in plain text to `hosts.toml` or any
other file. On connect the password reaches ssh through `SSH_ASKPASS`, so it
never appears on a command line and no tool like `sshpass` is needed.

If the keyring is unavailable (gnome-keyring or kwallet not running) the
password cannot be saved; the app says so and ssh asks in the terminal as
usual.

## Shared SSH channel

Terminal sessions and Monitor probes share one `ControlMaster` channel per
host, so authentication happens once: with a tab open, a Monitor round rides
that channel and authenticates not at all.

A unix socket path is capped at 108 bytes. When the control path would exceed
that, sharing is disabled silently — slower, but it still connects, which
beats ssh refusing with "ControlPath too long".

## Fonts

Inter for the interface, JetBrains Mono for identifiers. Without them the
fallback chain in `style.css` takes over (Adwaita Sans / Cantarell, DejaVu
Sans Mono): the app runs, but the typographic contrast between human text and
machine identifiers weakens. The Flatpak build sees the host's fonts, so
installing the distribution packages is enough.

## Visual verification

```bash
./target/debug/runetty --screenshot /tmp/runetty.png
```

Renders the window offscreen to a PNG and exits. `RUNETTY_SCREENSHOT_DELAY_MS`
stretches the delay for screens still waiting on the network.

Development flags, all of which also force a non-unique instance so they work
while a window is already open:

| Flag | Purpose |
| --- | --- |
| `--screenshot <file>` | Render the window and exit |
| `--mode <sessions\|agents\|monitor>` | Start in a given mode |
| `--connect <alias[,alias]>` | Connect; a `+` prefix forces an extra session |
| `--exec <command>` | Open a tab running a command |
| `--edit <alias>` | Open the record dialog |
| `--context-menu <alias>` | Open a row's context menu |
| `--action <name>` | Trigger a parameterless window action |
| `--expand <alias>` | Expand a Monitor card's detail |
| `--demo-metrics` | Fill the rings without a network |
| `--version` | Print runtime GTK and libadwaita versions |
