//! Host key verification on first connect.
//!
//! Because we set `SSH_ASKPASS_REQUIRE=force`, OpenSSH also routes the
//! "do you trust this host?" question to askpass; ours returns a password,
//! so the question never reached the user at all.
//!
//! The fix is to ask it ourselves: fetch the key with `ssh-keyscan`, show the
//! fingerprint, and write it to `known_hosts` only on approval. We never
//! accept automatically - trust is the user's call.

use std::path::PathBuf;
use std::process::Stdio;

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

pub fn known_hosts() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh/known_hosts"))
}

/// `[host]:port` - ssh's standard notation.
fn lookup_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

pub fn status(host: &str, port: u16) -> Known {
    let Some(path) = known_hosts() else {
        return Known::Unclear;
    };

    if !path.exists() {
        return Known::No;
    }

    let recorded = recorded_keys(&path, host, port);
    if recorded.is_empty() {
        return Known::No;
    }

    // Recorded is not the same as current. A rebuilt host, a reused address or
    // a man in the middle all look identical here, and ssh refuses to connect
    // in every case - so the difference has to be surfaced, not assumed away.
    let Ok(offered) = scan(host, port) else {
        // No answer from the host: nothing to compare against.
        return Known::Unclear;
    };

    if offered.iter().any(|key| recorded.contains(&key_material(key))) {
        Known::Yes
    } else {
        Known::Changed
    }
}

/// The key material (type + base64) of the entries known for this host.
fn recorded_keys(path: &std::path::Path, host: &str, port: u16) -> Vec<String> {
    let output = std::process::Command::new("ssh-keygen")
        .arg("-F")
        .arg(lookup_name(host, port))
        .arg("-f")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let Ok(output) = output else { return Vec::new() };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(key_material)
        .filter(|material| !material.is_empty())
        .collect()
}

/// Drops the host field so entries can be compared regardless of how the
/// host was written, including hashed known_hosts lines.
fn key_material(line: &str) -> String {
    line.split_whitespace().skip(1).take(2).collect::<Vec<&str>>().join(" ")
}

/// The keys currently recorded for a host, for showing what is being replaced.
pub fn recorded_fingerprints(host: &str, port: u16) -> Vec<(String, String)> {
    let Some(path) = known_hosts() else { return Vec::new() };

    let lines: Vec<String> = recorded_keys(&path, host, port)
        .into_iter()
        .map(|material| format!("{host} {material}"))
        .collect();

    fingerprints(&lines)
}

/// Removes every recorded key for a host. Used only when the user accepts a
/// changed key, so the new one does not collide with the old.
pub fn forget(host: &str, port: u16) -> Result<(), String> {
    let path = known_hosts().ok_or_else(|| "~/.ssh not found".to_string())?;

    let status = std::process::Command::new("ssh-keygen")
        .arg("-R")
        .arg(lookup_name(host, port))
        .arg("-f")
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not run ssh-keygen: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("ssh-keygen could not remove the old key".into())
    }
}

/// `ssh-keyscan` output: one key per line.
pub fn scan(host: &str, port: u16) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("ssh-keyscan")
        .arg("-T")
        .arg("5")
        .arg("-p")
        .arg(port.to_string())
        .arg(host)
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
pub fn trust(keys: &[String]) -> Result<(), String> {
    let path = known_hosts().ok_or_else(|| "~/.ssh not found".to_string())?;
    trust_into(&path, keys)
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

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("could not open known_hosts: {error}"))?;

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

    #[test]
    fn port_is_part_of_the_lookup_name() {
        assert_eq!(lookup_name("10.0.0.1", 22), "10.0.0.1");
        assert_eq!(lookup_name("10.0.0.1", 2222), "[10.0.0.1]:2222");
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
}
