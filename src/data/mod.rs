//! Data layer for server records.
//!
//! There are two sources and the split is deliberate:
//!   - `~/.ssh/config`: the real configuration shared with `ssh`, `scp`, `rsync`.
//!                      runetty never deletes those records, only hides them.
//!   - `hosts.toml`:    runetty's own entries plus folder, label and timestamps.
//!
//! Per the plan: connection fields belong in ssh config, presentation in TOML.

pub mod agents;
pub mod hostkey;
pub mod metrics;
pub mod secrets;
pub mod settings;
pub mod sessions;
pub mod ssh_config;
pub mod store;

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Read from ~/.ssh/config.
    SshConfig,
    /// runetty's own record.
    Local,
}

#[derive(Clone, Debug)]
pub struct Host {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub folder: Option<String>,
    /// Private key passed with `-i`; `IdentityFile` in ssh config.
    pub identity_file: Option<String>,
    pub last_used: Option<u64>,
    /// Whether Monitor watches this host. Off by default: running ssh every
    /// two seconds for every record is a cost nobody asked for.
    pub monitored: bool,
    pub source: Source,
}

impl Host {
    /// Second layer of a list row: `user@host[:port]`
    pub fn user_host(&self) -> String {
        let host = if self.hostname.is_empty() {
            self.alias.as_str()
        } else {
            self.hostname.as_str()
        };

        let base = match &self.user {
            Some(user) => format!("{user}@{host}"),
            None => host.to_string(),
        };

        match self.port {
            Some(port) if port != 22 => format!("{base}:{port}"),
            _ => base,
        }
    }

    /// Every `ssh` argument needed to connect, target included.
    ///
    /// Both terminal sessions and Monitor probes come from here. A single
    /// source matters: if a field like port or key is missed on one side,
    /// Monitor connects to the wrong host or with the wrong identity.
    pub fn ssh_args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec!["ssh".into()];
        args.extend(control_options());

        if self.source == Source::SshConfig {
            // Port, key, jump host: all already defined in ssh config.
            args.push(self.alias.clone());
            return args;
        }

        if let Some(port) = self.port {
            if port != 22 {
                args.push("-p".into());
                args.push(port.to_string());
            }
        }

        if let Some(identity) = &self.identity_file {
            args.push("-i".into());
            args.push(identity.clone());
        }

        args.push(self.user_host_plain());
        args
    }

    /// Shell command to run in the terminal.
    pub fn command(&self) -> String {
        self.ssh_args()
            .iter()
            .map(|arg| if needs_quoting(arg) { shell_quote(arg) } else { arg.clone() })
            .collect::<Vec<String>>()
            .join(" ")
    }

    fn user_host_plain(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.hostname),
            None => self.hostname.clone(),
        }
    }
}

pub(crate) fn quote_if_needed(value: &str) -> String {
    if needs_quoting(value) {
        shell_quote(value)
    } else {
        value.to_string()
    }
}

fn needs_quoting(value: &str) -> bool {
    value.contains(|c: char| c.is_whitespace() || "'\"$`\\;&|<>()*?[]{}".contains(c))
}

/// A path may contain spaces or shell metacharacters; single-quote it.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Terminal sessions and Monitor probes share one persistent channel, so
/// authentication happens once per host: after the first session supplies the
/// password or key, every later connection rides the same channel.
pub fn control_options() -> Vec<String> {
    let Some(dir) = control_dir() else {
        return Vec::new();
    };
    control_options_in(&dir)
}

fn control_options_in(dir: &std::path::Path) -> Vec<String> {
    let path = format!("{}/cm-%C", dir.display());

    // A unix socket path is capped at 108 bytes and `%C` expands further.
    // Near the limit, give up sharing: slower but working, since otherwise
    // ssh refuses with "ControlPath too long" and never connects.
    if path.len() + 24 >= 108 {
        return Vec::new();
    }

    let _ = std::fs::create_dir_all(&dir);

    vec![
        "-o".into(),
        format!("ControlPath={path}"),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        // Keep the channel alive briefly after the terminal closes, so a Monitor
        // probe or a second session opened right after need not re-authenticate.
        "ControlPersist=600".into(),
    ]
}

/// The socket path must stay short; `%C` is already a short hash.
pub fn control_dir() -> Option<std::path::PathBuf> {
    dirs::runtime_dir().or_else(dirs::cache_dir).map(|dir| dir.join("runetty"))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "2m", "18m", "3h", "6d" - the relative time shown on the right.
pub fn relative(timestamp: Option<u64>) -> String {
    let Some(timestamp) = timestamp else {
        return String::new();
    };

    let delta = now().saturating_sub(timestamp);
    match delta {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", delta / 60),
        3600..=86_399 => format!("{}h", delta / 3600),
        86_400..=2_591_999 => format!("{}d", delta / 86_400),
        _ => format!("{}w", delta / 604_800),
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn short_path_enables_sharing() {
        let options = control_options_in(Path::new("/run/user/1000/runetty"));
        assert!(options.iter().any(|option| option.starts_with("ControlPath=")));
        assert!(options.iter().any(|option| option == "ControlMaster=auto"));
    }

    /// A unix socket is capped at 108 bytes. Dropping channel sharing and
    /// still working beats failing outright with "too long".
    #[test]
    fn overlong_path_disables_sharing_instead_of_failing() {
        let deep = format!("/tmp/{}/runetty", "x".repeat(120));
        assert!(control_options_in(Path::new(&deep)).is_empty());
    }
}
