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
pub mod palette;
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

    /// Options for `ssh`, without the program name and without the target.
    ///
    /// Terminal sessions, Monitor probes and agent attach all start from here.
    /// A single source matters: if a field like port or key is missed on one
    /// side, Monitor connects to the wrong host or with the wrong identity.
    pub fn ssh_options(&self) -> Vec<String> {
        let mut options = control_options();

        if self.source == Source::SshConfig {
            // Port, key, jump host: all already defined in ssh config.
            return options;
        }

        if let Some(port) = self.port {
            if port != 22 {
                options.push("-p".into());
                options.push(port.to_string());
            }
        }

        if let Some(identity) = &self.identity_file {
            options.push("-i".into());
            options.push(identity.clone());
        }

        options
    }

    /// The destination as ssh should receive it.
    pub fn ssh_target(&self) -> String {
        match self.source {
            Source::SshConfig => self.alias.clone(),
            Source::Local => self.user_host_plain(),
        }
    }

    /// `ssh <options> <extra> -- <target> <remote...>`.
    ///
    /// The `--` is what keeps a target such as `-oProxyCommand=...` from being
    /// read as an option: without it a crafted record in hosts.toml could run
    /// a command locally. Callers put their own options in `extra`, never after
    /// the target, and the remote command last.
    pub fn ssh_invocation(&self, extra: &[&str], remote: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = vec!["ssh".into()];
        args.extend(self.ssh_options());
        args.extend(extra.iter().map(|value| value.to_string()));
        args.push("--".into());
        args.push(self.ssh_target());
        args.extend(remote.iter().map(|value| value.to_string()));
        args
    }

    /// Starts `ssh -O <command>` on this host's shared channel, if it has one.
    ///
    /// `stop` refuses new sessions but lets open ones finish: what should
    /// happen when a password is forgotten or a record deleted, so the next
    /// connection has to authenticate again. `exit` ends the channel and
    /// everything on it, for when runetty itself quits.
    pub fn release_channel(&self, command: &str) -> Option<std::process::Child> {
        if control_options().is_empty() {
            return None;
        }

        let args = self.ssh_invocation(&["-O", command], &[]);
        std::process::Command::new(&args[0])
            .args(&args[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()
    }

    /// Every `ssh` argument needed for an interactive session.
    pub fn ssh_args(&self) -> Vec<String> {
        self.ssh_invocation(&[], &[])
    }

    /// Hosts and users may not start with `-`. The `--` in every invocation
    /// already stops them being read as options; refusing them at the door
    /// means a crafted record never gets that far.
    pub fn is_well_formed(&self) -> bool {
        let starts_with_dash = |value: &str| value.trim_start().starts_with('-');

        !starts_with_dash(&self.alias)
            && !starts_with_dash(&self.hostname)
            && !self.user.as_deref().is_some_and(starts_with_dash)
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
        // Keep the channel alive briefly after the terminal closes, so a second
        // session opened right after need not re-authenticate. Monitor probes
        // keep it in use while a host is watched; runetty closes it on quit.
        "ControlPersist=120".into(),
    ]
}

/// The socket path must stay short; `%C` is already a short hash.
pub fn control_dir() -> Option<std::path::PathBuf> {
    dirs::runtime_dir().or_else(dirs::cache_dir).map(|dir| dir.join("runetty"))
}

/// Environment for an ssh process that may be answered from the keyring.
///
/// Besides the keyring alias, askpass is told which connection this is: the
/// names the target may appear under, its user, and the key files that may
/// legitimately ask for a passphrase. Without that it could only judge a
/// prompt by its wording, and both the wording and - for keyboard-interactive
/// prompts - the text itself can come from a server, or from a jump host
/// that is not the machine the password belongs to.
///
/// Empty when nothing is saved for this host: ssh then asks in the terminal.
pub fn askpass_environment(host: &Host) -> Vec<(String, String)> {
    if !secrets::has(&host.alias) {
        return Vec::new();
    }

    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };

    let mut names = vec![host.alias.clone()];
    if !host.hostname.is_empty() && host.hostname != host.alias {
        names.push(host.hostname.clone());
    }

    let mut environment = vec![
        ("SSH_ASKPASS".to_string(), exe.to_string_lossy().into_owned()),
        ("SSH_ASKPASS_REQUIRE".to_string(), "force".to_string()),
        ("RUNETTY_ASKPASS_ALIAS".to_string(), host.alias.clone()),
        ("RUNETTY_ASKPASS_HOSTS".to_string(), names.join(",")),
        ("RUNETTY_ASKPASS_KEYS".to_string(), expected_keys(host).join("\n")),
    ];

    if let Some(user) = &host.user {
        environment.push(("RUNETTY_ASKPASS_USER".to_string(), user.clone()));
    }

    environment
}

/// The key files ssh may legitimately ask a passphrase for on this connection:
/// the one the record names, plus ssh's own defaults.
fn expected_keys(host: &Host) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();

    if let Some(identity) = &host.identity_file {
        keys.push(identity.clone());
    }

    if let Some(ssh) = dirs::home_dir().map(|home| home.join(".ssh")) {
        for name in ["id_ed25519", "id_ecdsa", "id_rsa", "id_ed25519_sk", "id_ecdsa_sk"] {
            keys.push(ssh.join(name).to_string_lossy().into_owned());
        }
    }

    keys
}

/// Writes a file only the owner can read.
///
/// These files list which machines exist, who logs in to them and on which
/// port. That is an inventory worth keeping to oneself on a shared machine,
/// and it costs nothing to match what ssh does with its own config.
pub(crate) fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;

    file.write_all(contents.as_bytes())
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

#[cfg(test)]
mod injection_tests {
    use super::*;

    fn host(hostname: &str, user: Option<&str>, identity: Option<&str>) -> Host {
        Host {
            alias: "evil".into(),
            hostname: hostname.into(),
            user: user.map(str::to_string),
            port: None,
            folder: None,
            identity_file: identity.map(str::to_string),
            last_used: None,
            monitored: false,
            source: Source::Local,
        }
    }

    /// Record fields reach a shell through the terminal, so a hostname that
    /// looks like shell syntax must stay one argument.
    #[test]
    fn metacharacters_in_a_hostname_stay_quoted() {
        let command = host("box; rm -rf ~", None, None).command();
        assert!(command.ends_with("'box; rm -rf ~'"), "got {command}");
    }

    #[test]
    fn substitution_in_a_hostname_stays_quoted() {
        for nasty in ["$(id)", "`id`", "a|b", "a&b", "x\nrm -rf /"] {
            let command = host(nasty, None, None).command();
            let quoted = shell_quote(nasty);
            assert!(command.ends_with(&quoted), "{nasty:?} -> {command}");
        }
    }


    /// Reading the quoting by eye proves little, so ask a real shell: the
    /// value must come back out byte for byte, and nothing inside it may run.
    fn round_trip(value: &str) -> String {
        let script = format!("printf %s {}", shell_quote(value));
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .output()
            .expect("sh should run");

        assert!(output.status.success(), "sh rejected: {script}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn quoting_survives_a_real_shell() {
        for nasty in [
            "a'; id; echo '",
            "$(id)",
            "`id`",
            "; rm -rf ~",
            "a|b",
            "x\ny",
            "--option=value",
        ] {
            assert_eq!(round_trip(nasty), nasty, "mangled: {nasty:?}");
        }
    }

    #[test]
    fn key_paths_with_spaces_stay_one_argument() {
        let command = host("example.test", Some("root"), Some("/home/u/my keys/id")).command();
        assert!(command.contains("-i '/home/u/my keys/id'"), "got {command}");
    }

    /// ssh_args goes to execv, where no shell is involved: values must be
    /// passed through untouched rather than carrying quotes into argv.
    #[test]
    fn exec_arguments_are_not_quoted() {
        let args = host("box; rm -rf ~", Some("root"), None).ssh_args();
        assert_eq!(args.last().map(String::as_str), Some("root@box; rm -rf ~"));
    }

    /// A target that looks like an option must be read as a target. Without
    /// `--`, `-oProxyCommand=...` in hosts.toml would run a command locally.
    #[test]
    fn a_dash_target_comes_after_the_option_terminator() {
        let evil = host("-oProxyCommand=touch /tmp/pwned", None, None);
        let args = evil.ssh_invocation(&["-t"], &["uptime"]);

        let dashes = args.iter().position(|arg| arg == "--").expect("-- present");
        assert_eq!(args[dashes + 1], "-oProxyCommand=touch /tmp/pwned");
        assert!(
            args[..dashes].iter().all(|arg| !arg.contains("ProxyCommand")),
            "the target leaked into the options: {args:?}"
        );
        assert_eq!(args.last().map(String::as_str), Some("uptime"));
    }

    #[test]
    fn dash_hosts_and_users_are_rejected() {
        assert!(host("example.test", Some("root"), None).is_well_formed());
        assert!(!host("-oProxyCommand=x", None, None).is_well_formed());
        assert!(!host("example.test", Some("-oProxyCommand=x"), None).is_well_formed());
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// The record files list which machines exist and who logs in to them.
    /// On a shared machine that should not be world-readable.
    #[test]
    fn records_are_written_owner_only() {
        let dir = std::env::temp_dir().join(format!("runetty-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hosts.toml");

        write_private(&path, "hidden = []\n").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");

        // Rewriting must not loosen it either.
        write_private(&path, "hidden = [\"x\"]\n").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o} after rewrite");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
