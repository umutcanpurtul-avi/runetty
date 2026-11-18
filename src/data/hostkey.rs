//! Host key verification on first connect.
//!
//! With a saved password, ssh runs with `SSH_ASKPASS_REQUIRE=force` and its
//! "do you trust this host?" question goes to askpass instead of the
//! terminal. So runetty asks first: fetch the key with `ssh-keyscan`, show
//! the fingerprint, and write it to `known_hosts` only on approval. We never
//! accept automatically - trust is the user's call.
//!
//! Hosts behind a jump host or proxy cannot be scanned from here; for those
//! askpass puts ssh's own question to the user.

use std::path::PathBuf;
use std::process::Stdio;

use super::Host;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Known {
    /// Recorded in known_hosts and still matching; safe to connect.
    Yes,
    /// Never seen; needs approval.
    No,
    /// Recorded, but the host now presents a different key. Either the host
    /// was rebuilt, an address was reused, or someone is in the middle.
    Changed,
    /// Undecidable (no ssh-keygen, no network); leave it to ssh's own flow.
    Unclear,
}

/// Where and under which name ssh itself will look for this host's key.
///
/// Taken from `ssh -G`, not from runetty's own reading of the config: ssh
/// applies `Match` blocks, `Host *` defaults, first-value-wins and
/// `HostKeyAlias`, and the key checked here has to be the one ssh checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The address ssh connects to - what gets scanned.
    pub host: String,
    pub port: u16,
    /// `HostKeyAlias`, if set: ssh then records and looks up the key under
    /// this name, without a port.
    alias: Option<String>,
    /// `HashKnownHosts`: new entries are written hashed.
    hash: bool,
    /// `UserKnownHostsFile`; the first one is where new keys go.
    files: Vec<PathBuf>,
}

impl Target {
    /// `None` when the key cannot be checked from here: the connection goes
    /// through a jump host or a proxy command, so a direct scan would reach a
    /// different machine (or none). ssh then asks about the key itself.
    pub fn resolve(host: &Host) -> Option<Self> {
        let args = host.ssh_invocation(&["-G"], &[]);
        let output = std::process::Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        Self::parse(&String::from_utf8_lossy(&output.stdout))
    }

    fn parse(config: &str) -> Option<Self> {
        let value = |key: &str| {
            config.lines().find_map(|line| {
                let (name, value) = line.split_once(' ')?;
                (name == key).then(|| value.trim().to_string())
            })
        };
        let set = |key: &str| value(key).filter(|value| !value.eq_ignore_ascii_case("none"));

        if set("proxyjump").is_some() || set("proxycommand").is_some() {
            return None;
        }

        let home = dirs::home_dir();
        let files = value("userknownhostsfile")
            .unwrap_or_default()
            .split_whitespace()
            .filter(|path| !path.eq_ignore_ascii_case("none"))
            .map(|path| match (path.strip_prefix("~/"), &home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => PathBuf::from(path),
            })
            .collect::<Vec<PathBuf>>();

        Some(Self {
            host: value("hostname")?,
            port: value("port").and_then(|port| port.parse().ok()).unwrap_or(22),
            alias: set("hostkeyalias"),
            hash: value("hashknownhosts").is_some_and(|value| value == "yes"),
            files: if files.is_empty() { known_hosts().into_iter().collect() } else { files },
        })
    }

    /// The name known_hosts entries for this host carry: `HostKeyAlias` as is,
    /// otherwise `host` or `[host]:port` - ssh's standard notation.
    fn lookup_name(&self) -> String {
        match &self.alias {
            Some(alias) => alias.clone(),
            None if self.port == 22 => self.host.clone(),
            None => format!("[{}]:{}", self.host, self.port),
        }
    }

    /// The main known_hosts file: the one new keys are written to.
    fn file(&self) -> Option<&std::path::Path> {
        self.files.first().map(PathBuf::as_path)
    }
}

fn known_hosts() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh/known_hosts"))
}

pub fn status(target: &Target) -> Known {
    if target.file().is_none() {
        return Known::Unclear;
    }

    let recorded = recorded_keys(target);
    if recorded.is_empty() {
        return Known::No;
    }

    // Recorded is not the same as current. A rebuilt host, a reused address or
    // a man in the middle all look identical here, and ssh refuses to connect
    // in every case - so the difference has to be surfaced, not assumed away.
    let Ok(offered) = scan(target) else {
        // No answer from the host: nothing to compare against.
        return Known::Unclear;
    };

    if offered.iter().any(|key| recorded.contains(&key_material(key))) {
        Known::Yes
    } else {
        Known::Changed
    }
}

/// The key material (type + base64) of the entries known for this host, from
/// every known_hosts file ssh would read.
fn recorded_keys(target: &Target) -> Vec<String> {
    let mut keys = Vec::new();

    for path in target.files.iter().filter(|path| path.exists()) {
        let output = std::process::Command::new("ssh-keygen")
            .arg("-F")
            .arg(target.lookup_name())
            .arg("-f")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        let Ok(output) = output else { continue };
        if !output.status.success() {
            continue;
        }

        keys.extend(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(key_material)
                .filter(|material| !material.is_empty()),
        );
    }

    keys
}

/// Drops the host field so entries can be compared regardless of how the
/// host was written, including hashed known_hosts lines.
fn key_material(line: &str) -> String {
    line.split_whitespace().skip(1).take(2).collect::<Vec<&str>>().join(" ")
}

/// The keys currently recorded for a host, for showing what is being replaced.
pub fn recorded_fingerprints(target: &Target) -> Vec<(String, String)> {
    let lines: Vec<String> = recorded_keys(target)
        .into_iter()
        .map(|material| format!("{} {material}", target.lookup_name()))
        .collect();

    fingerprints(&lines)
}

/// Removes every recorded key for a host. Used only when the user accepts a
/// changed key, so the new one does not collide with the old.
pub fn forget(target: &Target) -> Result<(), String> {
    for path in target.files.iter().filter(|path| path.exists()) {
        let status = std::process::Command::new("ssh-keygen")
            .arg("-R")
            .arg(target.lookup_name())
            .arg("-f")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("could not run ssh-keygen: {error}"))?;

        if !status.success() {
            return Err("ssh-keygen could not remove the old key".into());
        }
    }

    Ok(())
}

/// `ssh-keyscan` output: one key per line.
pub fn scan(target: &Target) -> Result<Vec<String>, String> {
    // `--`: a host such as `-f/some/file` must stay a host name.
    let output = std::process::Command::new("ssh-keyscan")
        .arg("-T")
        .arg("5")
        .arg("-p")
        .arg(target.port.to_string())
        .arg("--")
        .arg(&target.host)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("could not run ssh-keyscan: {error}"))?;

    let keys: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();

    if keys.is_empty() {
        let reason = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next_back()
            .unwrap_or("no key returned by the host")
            .trim()
            .to_string();
        return Err(reason);
    }

    Ok(keys)
}

/// Fingerprints to show the user: ("ED25519", "SHA256:...").
pub fn fingerprints(keys: &[String]) -> Vec<(String, String)> {
    use std::io::Write;

    let mut child = match std::process::Command::new("ssh-keygen")
        .arg("-l")
        .arg("-f")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Vec::new(),
    };

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(keys.join("\n").as_bytes());
    }

    let Ok(output) = child.wait_with_output() else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // "256 SHA256:xxx host (ED25519)"
            let mut fields = line.split_whitespace();
            let _bits = fields.next()?;
            let fingerprint = fields.next()?.to_string();
            let kind = line
                .rsplit('(')
                .next()?
                .trim_end_matches(')')
                .to_string();
            Some((kind, fingerprint))
        })
        .collect()
}

/// Appends keys to known_hosts. Never rewrites the file, only appends.
///
/// Entries are written under the name ssh will look them up by, which with
/// `HostKeyAlias` is not the address that was scanned, and hashed when
/// `HashKnownHosts` says so.
pub fn trust(target: &Target, keys: &[String]) -> Result<(), String> {
    let path = target.file().ok_or_else(|| "no known_hosts file".to_string())?;

    let mut lines: Vec<String> = keys
        .iter()
        .map(|key| format!("{} {}", target.lookup_name(), key_material(key)))
        .collect();

    if target.hash {
        lines = hashed(&lines)?;
    }

    trust_into(path, &lines)
}

/// Hashes the host field the way ssh does, by letting `ssh-keygen -H` rewrite
/// a private scratch copy - the real file is never handed to it.
fn hashed(lines: &[String]) -> Result<Vec<String>, String> {
    let dir = crate::data::control_dir()
        .ok_or_else(|| "no runtime directory for hashing".to_string())?;
    create_private_dir(&dir)?;

    let scratch = dir.join(format!("known_hosts-{}", std::process::id()));
    let backup = scratch.with_extension("old");
    crate::data::write_private(&scratch, &(lines.join("\n") + "\n"))
        .map_err(|error| format!("could not prepare hashing: {error}"))?;

    let status = std::process::Command::new("ssh-keygen")
        .arg("-H")
        .arg("-f")
        .arg(&scratch)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let contents = std::fs::read_to_string(&scratch);
    let _ = std::fs::remove_file(&scratch);
    let _ = std::fs::remove_file(&backup);

    match (status, contents) {
        (Ok(status), Ok(contents)) if status.success() => Ok(contents
            .lines()
            .filter(|line| line.starts_with("|1|"))
            .map(str::to_string)
            .collect()),
        _ => Err("ssh-keygen could not hash the entry".into()),
    }
}

/// ssh refuses to use a ~/.ssh that others can write to, and the directory is
/// private by convention. Creating it at the default umask would be wrong.
fn create_private_dir(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    if path.is_dir() {
        return Ok(());
    }

    std::fs::create_dir_all(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not set permissions on {}: {error}", path.display()))
}

fn trust_into(path: &std::path::Path, keys: &[String]) -> Result<(), String> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }

    // A file that does not end in a newline would glue the first new entry
    // onto its last line, breaking both.
    let unterminated = std::fs::read(path)
        .map(|contents| contents.last().is_some_and(|byte| *byte != b'\n'))
        .unwrap_or(false);

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("could not open known_hosts: {error}"))?;

    if unterminated {
        writeln!(file).map_err(|error| format!("could not write known_hosts: {error}"))?;
    }

    for key in keys {
        writeln!(file, "{key}")
            .map_err(|error| format!("could not write known_hosts: {error}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_material_ignores_the_host_field() {
        let a = "example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5";
        let b = "|1|hashed|form= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5";
        assert_eq!(key_material(a), key_material(b));
        assert_eq!(key_material(a), "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5");
    }

    /// A comment after the key must not change the comparison.
    #[test]
    fn key_material_stops_after_the_key() {
        let with_comment = "example.test ssh-rsa AAAAB3Nza comment here";
        assert_eq!(key_material(with_comment), "ssh-rsa AAAAB3Nza");
    }

    fn target(config: &str) -> Option<Target> {
        Target::parse(config)
    }

    #[test]
    fn port_is_part_of_the_lookup_name() {
        let plain = target("hostname 10.0.0.1\nport 22\n").unwrap();
        let other = target("hostname 10.0.0.1\nport 2222\n").unwrap();
        assert_eq!(plain.lookup_name(), "10.0.0.1");
        assert_eq!(other.lookup_name(), "[10.0.0.1]:2222");
    }

    /// ssh records keys under HostKeyAlias, port or not.
    #[test]
    fn host_key_alias_is_the_lookup_name() {
        let aliased = target("hostname 10.0.0.1\nport 2222\nhostkeyalias db\n").unwrap();
        assert_eq!(aliased.lookup_name(), "db");
        assert_eq!(aliased.host, "10.0.0.1");
        assert_eq!(aliased.port, 2222);
    }

    /// A scan would reach the machine in front, not the one ssh ends up on.
    #[test]
    fn proxied_hosts_are_not_scanned() {
        assert_eq!(target("hostname inner\nproxyjump bastion\n"), None);
        assert_eq!(target("hostname inner\nproxycommand nc %h %p\n"), None);
        assert!(target("hostname inner\nproxycommand none\n").is_some());
    }

    #[test]
    fn known_hosts_files_and_hashing_come_from_ssh() {
        let found = target(
            "hostname h\nport 22\nhashknownhosts yes\n\
             userknownhostsfile /k/one /k/two\n",
        )
        .unwrap();
        assert!(found.hash);
        assert_eq!(found.files, vec![PathBuf::from("/k/one"), PathBuf::from("/k/two")]);
    }

    /// The effective values of a real `ssh -G`, when ssh is installed.
    #[test]
    fn resolves_through_ssh() {
        if std::process::Command::new("ssh").arg("-V").output().is_err() {
            return;
        }

        let host = Host {
            alias: "box".into(),
            hostname: "example.test".into(),
            user: Some("me".into()),
            port: Some(2222),
            folder: None,
            identity_file: None,
            last_used: None,
            monitored: false,
            source: crate::data::Source::Local,
        };
        let found = Target::resolve(&host).expect("ssh -G should answer");
        assert_eq!(found.host, "example.test");
        assert_eq!(found.port, 2222);
    }

    /// Must not overwrite existing entries: the file should only grow.
    #[test]
    fn trust_appends_without_touching_existing_entries() {
        let dir = std::env::temp_dir().join(format!("runetty-test-{}", std::process::id()));
        let path = dir.join("known_hosts");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(&path, "old.example ssh-rsa AAAA\n").unwrap();

        trust_into(&path, &["new.example ssh-ed25519 BBBB".to_string()]).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("old.example"), "existing entry must survive");
        assert!(contents.contains("new.example"), "new entry must be appended");
        assert_eq!(contents.lines().count(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trust_does_not_glue_onto_an_unterminated_line() {
        let dir = std::env::temp_dir().join(format!("runetty-test-nl-{}", std::process::id()));
        let path = dir.join("known_hosts");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(&path, "old.example ssh-rsa AAAA").unwrap();

        trust_into(&path, &["new.example ssh-ed25519 BBBB".to_string()]).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "old.example ssh-rsa AAAA\nnew.example ssh-ed25519 BBBB\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_fingerprints_from_keyscan_lines() {
        // Without ssh-keygen the test is moot; verify real output when present.
        if std::process::Command::new("ssh-keygen").arg("-?").output().is_err() {
            return;
        }

        let keys = vec![
            "example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ1IYbMBDJqUPNa0zRcVIcA1UGQCGQcYfVKPTFIbTkSX".to_string(),
        ];

        let found = fingerprints(&keys);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "ED25519");
        assert!(found[0].1.starts_with("SHA256:"), "got {}", found[0].1);
    }

    /// Against a real sshd: `RUNETTY_TEST_SSHD=<port>:<client key>` on
    /// 127.0.0.1. A trusted key, hashed or under an alias, must then be
    /// accepted by ssh itself with StrictHostKeyChecking=yes.
    #[test]
    fn trusted_keys_are_accepted_by_ssh() {
        let Some((port, key)) = std::env::var("RUNETTY_TEST_SSHD")
            .ok()
            .and_then(|value| value.split_once(':').map(|(p, k)| (p.to_string(), k.to_string())))
        else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("runetty-test-sshd-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        for (alias, hash) in [(None, true), (Some("lab-box".to_string()), false)] {
            let file = dir.join(format!("known_hosts-{hash}"));
            let _ = std::fs::remove_file(&file);
            let target = Target {
                host: "127.0.0.1".into(),
                port: port.parse().unwrap(),
                alias: alias.clone(),
                hash,
                files: vec![file.clone()],
            };

            assert_eq!(status(&target), Known::No);
            trust(&target, &scan(&target).unwrap()).unwrap();
            assert_eq!(status(&target), Known::Yes);
            let written = std::fs::read_to_string(&file).unwrap();
            assert_eq!(written.starts_with("|1|"), hash, "{written}");

            let mut ssh = std::process::Command::new("ssh");
            ssh.args(["-F", "/dev/null", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes"])
                .arg("-o")
                .arg(format!("UserKnownHostsFile={}", file.display()))
                .args(["-i", &key, "-p", &port]);
            if let Some(alias) = &alias {
                ssh.arg("-o").arg(format!("HostKeyAlias={alias}"));
            }
            let accepted = ssh.args(["--", "127.0.0.1", "true"]).status().unwrap();
            assert!(accepted.success(), "ssh rejected the key written for {alias:?}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
