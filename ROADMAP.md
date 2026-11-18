# Roadmap

What exists today is described in the [README](README.md); this file is only
about what comes next. Nothing here is a promise or a schedule.

## Next

- **Flathub release.** The main thing standing between runetty and anyone
  trying it: a native build currently needs Rust 1.92, which on most
  distributions means installing rustup first.
- **Independent security review.** It handles server access, keys and
  passwords, and has only been read by the people who wrote it.
- **Prebuilt packages** — AUR, `.deb`, `.rpm` — once Flathub is settled.

## Considered

- Sortable columns in the Monitor process table
- Agents from sources other than tmux (systemd units, a chosen process list)
- Monitor detail history, rather than only the current reading
- Per-host appearance: colour, label, terminal palette
- Non-Linux hosts in Monitor; it reads `/proc` and `df` today

## Not planned

- **macOS or Windows.** The project targets Linux desktops and leans on GTK4,
  VTE and the Secret Service keyring.
- **A daemon or agent on the servers.** Everything works over plain `ssh`
  against what is already there, and that constraint is the point.
- **Telemetry of any kind.**

Requests and arguments are welcome in
[Issues](https://github.com/umutcanpurtul-avi/runetty/issues).
