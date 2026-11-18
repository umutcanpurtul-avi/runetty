# Flathub submission

The manifest here is the one submitted to Flathub. It differs from
`../dev.runetty.Runetty.yaml` in a single respect: the source is a pinned git
tag rather than the working directory, which Flathub requires. Keep the two in
step whenever either changes.

## Releasing a new version

1. Bump `version` in `Cargo.toml` and add a `<release>` entry to
   `data/dev.runetty.Runetty.metainfo.xml`, then validate it:

   ```bash
   appstreamcli validate data/dev.runetty.Runetty.metainfo.xml
   ```

2. Regenerate the vendored cargo sources, since `Cargo.lock` has changed:

   ```bash
   python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
   cp build-aux/cargo-sources.json build-aux/flathub/
   ```

   The generator lives in
   [flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools).

3. Commit, tag, push the tag, then point the manifest at it:

   ```bash
   git tag -a vX.Y.Z -m "runetty X.Y.Z" && git push origin vX.Y.Z
   git rev-parse vX.Y.Z    # the commit to pin
   ```

4. Build it exactly as Flathub will, from the tag rather than your tree:

   ```bash
   flatpak run org.flatpak.Builder --user --force-clean --disable-rofiles-fuse \
       --install-deps-from=flathub build build-aux/flathub/dev.runetty.Runetty.yaml
   ```

5. Open a pull request against
   [flathub/flathub](https://github.com/flathub/flathub) on a branch off
   `new-pr`, containing this manifest and `cargo-sources.json`.

## Permissions, and why

| Permission | Why |
| --- | --- |
| `--share=network` | It is an SSH client. |
| `--socket=ssh-auth` | So an existing ssh-agent is used rather than asking for passphrases again. |
| `--filesystem=~/.ssh` | Reads `config` and `known_hosts`; writes `known_hosts` only after the user approves a host key. |
| `--filesystem=home` | Private keys kept outside `~/.ssh` are common, and the key chosen in the dialog has to stay readable by the `ssh` child process afterwards. |
| `--filesystem=xdg-config/runetty:create` | Shares its records with a native install of the same app. |
| `--talk-name=org.freedesktop.secrets` | Passwords live in the system keyring, not in a file. |
| `--socket=wayland`, `--socket=fallback-x11`, `--device=dri`, `--share=ipc` | Ordinary GTK application needs. |

`--filesystem=home` is the broad one. The narrower alternative, relying on the
file chooser portal, does not work here: the portal grants access to the app,
while the file has to be readable by the `ssh` process the app spawns, on every
later connection rather than once.
