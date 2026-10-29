//! Reader for `~/.ssh/config`. Read-only: we never write to that file.

use std::path::{Path, PathBuf};

use super::{Host, Source};

pub fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh/config"))
}

pub fn load() -> Vec<Host> {
    default_path().map(|path| parse_file(&path, 0)).unwrap_or_default()
}

fn parse_file(path: &Path, depth: u8) -> Vec<Host> {
    // A hard limit against include cycles and deep chains.
    if depth > 8 {
        return Vec::new();
    }

    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    parse(&text, path, depth)
}

fn parse(text: &str, origin: &Path, depth: u8) -> Vec<Host> {
    let mut hosts: Vec<Host> = Vec::new();
    let mut current: Option<Host> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((keyword, value)) = split_directive(line) else {
            continue;
        };
        let lowered = keyword.to_ascii_lowercase();

        match lowered.as_str() {
            "host" => {
                if let Some(host) = current.take() {
                    push(&mut hosts, host);
                }
                // `Host a b` and wildcards: take the first literal name, skip patterns.
                let alias = value.split_whitespace().find(|p| !is_pattern(p));
                current = alias.map(|alias| Host {
                    alias: alias.to_string(),
                    hostname: String::new(),
                    user: None,
                    port: None,
                    folder: None,
                    identity_file: None,
                    last_used: None,
                    monitored: false,
                    source: Source::SshConfig,
                });
            }
            "hostname" => {
                if let Some(host) = current.as_mut() {
                    host.hostname = value.to_string();
                }
            }
            "user" => {
                if let Some(host) = current.as_mut() {
                    host.user = Some(value.to_string());
                }
            }
            "identityfile" => {
                if let Some(host) = current.as_mut() {
                    host.identity_file = Some(expand_tilde(value));
                }
            }
            "port" => {
                if let Some(host) = current.as_mut() {
                    host.port = value.parse().ok();
                }
            }
            "include" => {
                for included in expand_include(value, origin) {
                    for host in parse_file(&included, depth + 1) {
                        push(&mut hosts, host);
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(host) = current.take() {
        push(&mut hosts, host);
    }

    hosts
}

fn push(hosts: &mut Vec<Host>, mut host: Host) {
    if host.hostname.is_empty() {
        host.hostname = host.alias.clone();
    }
    // If an alias is defined more than once the first wins, as ssh does.
    if !hosts.iter().any(|existing| existing.alias == host.alias) {
        hosts.push(host);
    }
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    // Both `Key value` and `Key=value` are valid.
    let (key, rest) = match line.find(['=', ' ', '\t']) {
        Some(index) => (&line[..index], line[index + 1..].trim_start_matches(['=', ' ', '\t'])),
        None => return None,
    };

    let rest = rest.trim();
    if key.is_empty() || rest.is_empty() {
        None
    } else {
        Some((key, rest))
    }
}

fn expand_tilde(value: &str) -> String {
    match value.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|home| home.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| value.to_string()),
        None => value.to_string(),
    }
}

fn is_pattern(value: &str) -> bool {
    value.contains('*') || value.contains('?') || value.starts_with('!')
}

fn expand_include(value: &str, origin: &Path) -> Vec<PathBuf> {
    let base = origin.parent().map(Path::to_path_buf).unwrap_or_default();

    value
        .split_whitespace()
        .filter(|pattern| !pattern.contains(['*', '?']))
        .map(|pattern| {
            if let Some(rest) = pattern.strip_prefix("~/") {
                dirs::home_dir().unwrap_or_default().join(rest)
            } else if pattern.starts_with('/') {
                PathBuf::from(pattern)
            } else {
                base.join(pattern)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_host_blocks() {
        let text = "\
# comment
Host edge
    HostName 192.0.2.21
    User root
    Port 2222

Host *.internal
    User admin

Host nas
    HostName=192.168.1.4
";
        let hosts = parse(text, Path::new("/tmp/config"), 0);
        assert_eq!(hosts.len(), 2);

        assert_eq!(hosts[0].alias, "edge");
        assert_eq!(hosts[0].hostname, "192.0.2.21");
        assert_eq!(hosts[0].user.as_deref(), Some("root"));
        assert_eq!(hosts[0].port, Some(2222));

        assert_eq!(hosts[1].alias, "nas");
        assert_eq!(hosts[1].hostname, "192.168.1.4");
        assert_eq!(hosts[1].user, None);
    }

    #[test]
    fn alias_without_hostname_falls_back() {
        let hosts = parse("Host box\n  User u\n", Path::new("/tmp/config"), 0);
        assert_eq!(hosts[0].hostname, "box");
        // The command carries the shared-channel options; what matters is that
        // the target is the alias, because ssh config knows the rest.
        let command = hosts[0].command();
        assert!(command.ends_with(" box"), "got {command}");
        assert!(command.starts_with("ssh "));
    }
}
