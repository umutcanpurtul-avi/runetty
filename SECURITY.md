# Security

runetty handles server access, SSH keys and, if you ask it to, passwords.
Please treat problems in those paths as security issues rather than bugs.

## Reporting

Use [private vulnerability reporting](https://github.com/umutcanpurtul-avi/runetty/security/advisories/new)
rather than a public issue. If that is not available to you, mail
purtulumut97@gmail.com.

Please include what you did, what happened, and the version
(`runetty --version`). A proof of concept helps but is not required.

This is a small project maintained by one person: expect a first reply within
a week, and no bounty.

## Supported versions

Only the latest release. The project is pre-1.0 and fixes go into the next
release rather than into patches of older ones.

## What is in scope

The paths that handle credentials and trust:

- The keyring and the `SSH_ASKPASS` bridge (`src/data/secrets.rs`, `src/askpass.rs`)
- Host key verification and anything written to `known_hosts` (`src/data/hostkey.rs`)
- How record fields reach a shell (`src/data/mod.rs`, `src/ui/terminal.rs`)
- Permissions on files runetty writes
- The Flatpak manifest's permissions

## What is known and intended

These are deliberate decisions, not oversights. Arguments against them are
welcome as issues, but they are not vulnerabilities:

- **Passwords can be stored in the system keyring.** That is a convenience the
  user opts into per host. Without it, ssh asks as usual.
- **Key passphrases can also be stored**, which is close to removing the
  passphrase. The alternative, an ssh-agent, is used automatically when it is
  running.
- **`--filesystem=home` in the Flatpak.** Private keys kept outside `~/.ssh`
  are common, and the key has to stay readable by the `ssh` child process on
  every later connection, which the file chooser portal does not provide.
- **The Flatpak is not a meaningful sandbox.** Together with `home`, it talks
  to `org.freedesktop.secrets` and uses the ssh-agent socket. The Secret
  Service API gives any client access to the whole keyring, not just
  runetty's entries, so treat the Flatpak as trusted as a native install.
- **runetty never writes to `~/.ssh/config`.** Deleting such a record only
  hides it, because other tools use that file.

## Advisories

Published advisories are listed under
[Security → Advisories](https://github.com/umutcanpurtul-avi/runetty/security/advisories).

## Review

The credential paths have had one external code review, which found that
saved secrets could reach the wrong party; that is fixed in 0.3.1. No formal
audit has happened.
