//! Long-running sessions on a host, read through tmux.
//!
//! "Agent" here means any detached piece of work: a build, a capture, an AI
//! coding agent. They nearly always live in tmux already, so nothing has to
//! be installed on the far side - one `tmux list-sessions` per host, over the
//! same shared SSH channel the terminal and Monitor use.

use super::Host;

/// One command, formatted so parsing never depends on tmux's default layout.
/// `|` separates fields; session names may contain spaces but not `|` often,
/// and the parser rejoins anything extra into the name anyway.
pub const PROBE: &str = "\
tmux list-sessions -F \
'#{session_name}|#{session_attached}|#{session_windows}|#{session_activity}|#{session_alerts}' \
2>/dev/null || true";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Something wants attention: a bell, or a window flagged by tmux.
    Blocked,
    /// Produced output recently, or someone is attached and watching.
    Working,
    /// Alive but quiet.
    Idle,
}

impl State {
    pub fn css(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Working => "working",
            Self::Idle => "idle",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Blocked => "needs you",
            Self::Working => "working",
            Self::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub name: String,
    /// Which host it runs on; the row's second layer.
    pub host: String,
    pub attached: bool,
    pub windows: u32,
    /// Unix seconds of the last activity in the session.
    pub activity: u64,
    /// tmux's own alert list for the session: bell, activity, silence.
    /// Empty on tmux versions that do not report it.
    pub alerts: String,
}

impl Agent {
    /// A session that moved in the last minute counts as working; one someone
    /// is attached to counts as working too, since it is being watched.
    pub fn state(&self, now: u64) -> State {
        // tmux raises an alert when a session rang a bell or went quiet after
        // activity; that is the closest thing to "this wants you".
        if !self.alerts.trim().is_empty() {
            return State::Blocked;
        }

        if self.attached {
            return State::Working;
        }

        match now.saturating_sub(self.activity) {
            0..=60 => State::Working,
            _ => State::Idle,
        }
    }

    /// Command that attaches to this session in a terminal tab.
    pub fn attach_command(&self, host: &Host) -> String {
        let mut args = host.ssh_args();
        let target = args.pop().unwrap_or_default();

        // A tty is required: tmux refuses to attach without one.
        args.push("-t".into());
        args.push(target);

        let command = args
            .iter()
            .map(|arg| super::quote_if_needed(arg))
            .collect::<Vec<String>>()
            .join(" ");

        format!("{command} tmux attach -t {}", shell_single_quote(&self.name))
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Parses `tmux list-sessions` output. Unknown or malformed lines are skipped
/// rather than failing the whole read: one odd session should not blank the
/// panel.
pub fn parse(output: &str, host: &str) -> Vec<Agent> {
    output
        .lines()
        .filter_map(|line| parse_line(line.trim(), host))
        .collect()
}

fn parse_line(line: &str, host: &str) -> Option<Agent> {
    if line.is_empty() {
        return None;
    }

    let fields: Vec<&str> = line.split('|').collect();
    if fields.len() < 5 {
        return None;
    }

    // Everything before the last four fields is the name, so names containing
    // `|` survive.
    let split = fields.len() - 4;
    let name = fields[..split].join("|");
    if name.is_empty() {
        return None;
    }

    Some(Agent {
        name,
        host: host.to_string(),
        attached: fields[split] != "0",
        windows: fields[split + 1].parse().unwrap_or(1),
        activity: fields[split + 2].parse().unwrap_or(0),
        alerts: fields[split + 3].to_string(),
    })
}

/// `ssh` arguments that list sessions over the shared channel.
pub fn probe_args(host: &Host) -> Vec<String> {
    let mut args = host.ssh_args();
    let target = args.pop().unwrap_or_default();

    args.push("-o".into());
    args.push("StrictHostKeyChecking=yes".into());
    args.push("-o".into());
    args.push("ConnectTimeout=5".into());
    args.push("-o".into());
    args.push("NumberOfPasswordPrompts=1".into());

    args.push(target);
    args.push(PROBE.into());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
build|1|3|1700000500|
deploy|0|1|1700000100|
my|session|0|2|1700000480|
";

    #[test]
    fn parses_sessions() {
        let agents = parse(OUTPUT, "edge-01");
        assert_eq!(agents.len(), 3);

        assert_eq!(agents[0].name, "build");
        assert!(agents[0].attached);
        assert_eq!(agents[0].windows, 3);
        assert_eq!(agents[0].host, "edge-01");

        assert!(!agents[1].attached);
        assert_eq!(agents[1].windows, 1);
    }

    /// A session name containing `|` must not shift the other fields.
    #[test]
    fn name_may_contain_the_separator() {
        let agents = parse(OUTPUT, "edge-01");
        assert_eq!(agents[2].name, "my|session");
        assert_eq!(agents[2].windows, 2);
        assert_eq!(agents[2].activity, 1_700_000_480);
        assert!(agents[2].alerts.is_empty());
    }

    /// No tmux, or no sessions, is an empty list rather than an error.
    #[test]
    fn empty_output_is_not_a_failure() {
        assert!(parse("", "edge-01").is_empty());
        assert!(parse("\n\n", "edge-01").is_empty());
        assert!(parse("garbage", "edge-01").is_empty());
    }

    #[test]
    fn state_follows_attachment_and_activity() {
        let agents = parse(OUTPUT, "edge-01");
        let now = 1_700_000_520;

        assert_eq!(agents[0].state(now), State::Working, "attached");
        assert_eq!(agents[1].state(now), State::Idle, "quiet for a while");
        assert_eq!(agents[2].state(now), State::Working, "recent activity");
    }

    /// An alert outranks both attachment and quietness.
    #[test]
    fn an_alert_means_it_wants_attention() {
        let agents = parse("waiting|0|1|1700000000|bell", "edge-01");
        assert_eq!(agents[0].alerts, "bell");
        assert_eq!(agents[0].state(1_700_999_999), State::Blocked);
    }

    /// The attach command needs a tty and must quote the session name.
    #[test]
    fn attach_command_requests_a_tty() {
        let host = Host {
            alias: "edge".into(),
            hostname: "10.0.0.1".into(),
            user: Some("root".into()),
            port: None,
            folder: None,
            identity_file: None,
            last_used: None,
            monitored: true,
            source: super::super::Source::Local,
        };

        let agent = Agent {
            name: "my build".into(),
            host: "edge".into(),
            attached: false,
            windows: 1,
            activity: 0,
            alerts: String::new(),
        };

        let command = agent.attach_command(&host);
        assert!(command.contains(" -t root@10.0.0.1 "), "got {command}");
        assert!(command.ends_with("tmux attach -t 'my build'"), "got {command}");
    }
}
