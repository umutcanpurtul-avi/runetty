# runetty

A GTK4 + libadwaita desktop client for SSH sessions, agents and host
monitoring. The visual language is described in [docs/plan.md](docs/plan.md)
(written in Turkish); the technical floor and distribution matrix are in
[BUILDING.md](BUILDING.md).

## Status

Phase 1 — visual skeleton, plus a working data and monitoring layer.

- [x] Design system in a single file (`data/resources/style.css`); no colours in Rust
- [x] Window skeleton: mode switcher, `OverlaySplitView`, tabbed content
- [x] Sidebar: Recents / Servers, folder tree, drag and drop, filter
- [x] Empty state: command field, "Jump back in" chips, keyboard hints
- [x] VTE terminal tabs (can be turned off with `--no-default-features`)
- [x] Records from `~/.ssh/config` (read-only) and `hosts.toml`
- [x] Host key approval on first connect, passwords in the system keyring
- [x] Monitor: opt-in per host, CPU/MEM/DISK rings, expandable detail
- [x] Light / dark / system theme, font family and size
- [x] Multi-distribution builds: Flatpak, four package managers, CI
- [ ] Command palette (Ctrl K)
- [ ] Agent panel wired to a real source

## Quick start

```bash
./build-aux/install-deps.sh
cargo build --release
./build-aux/install.sh          # installs into ~/.local
runetty
```

If your distribution is below the version floor the script points you at
Flatpak; see [BUILDING.md](BUILDING.md) for the details.

## How records are stored

| What | Where | Note |
| --- | --- | --- |
| Connection settings | `~/.ssh/config` | Read-only. runetty never writes here. |
| runetty's own records | `~/.config/runetty/hosts.toml` | Folders, watch list, timestamps |
| Appearance | `~/.config/runetty/settings.toml` | Theme, fonts, layout |
| Passwords | System keyring | Secret Service; never in a file |

Deleting a record that came from `~/.ssh/config` only hides it from the list,
and the hide can be undone. That file belongs to `ssh`, `scp` and `rsync` too.

## License

MIT. GTK4/libadwaita (LGPL-2.1) and VTE (LGPL-3.0) are linked dynamically;
Inter and JetBrains Mono are OFL.

The design language draws on SigilTTY; the name, logo and icon are our own.
