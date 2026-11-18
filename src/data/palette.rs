//! What the command palette can offer, and how a query narrows it.
//!
//! Kept apart from the widget so the matching can be tested without a window.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

/// Where an entry came from, which decides its badge and its tie-break order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A stored server: activating it connects.
    Server,
    /// A window action: preferences, new server, and so on.
    Command,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Command => "command",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: String,
    /// Second line: `user@host` for a server, the shortcut for a command.
    pub detail: String,
    pub kind: Kind,
    /// The window action to activate, e.g. `win.host-connect`.
    pub action: String,
    /// Target for actions that take one; the alias for a server.
    pub target: Option<String>,
    /// Recently used servers sort first among equal matches.
    pub last_used: Option<u64>,
}

impl Entry {
    /// Both lines are searched, so "web prod" finds a server by its address.
    fn haystack(&self) -> String {
        format!("{} {}", self.title, self.detail)
    }
}

/// The fixed commands. Servers are added by the caller from the store.
pub fn commands() -> Vec<Entry> {
    let entry = |title: &str, detail: &str, action: &str| Entry {
        title: title.to_string(),
        detail: detail.to_string(),
        kind: Kind::Command,
        action: action.to_string(),
        target: None,
        last_used: None,
    };

    vec![
        entry("New Server", "Ctrl N", "win.host-new"),
        entry("New Folder", "", "win.folder-new"),
        entry("Reconnect", "Ctrl R", "win.reconnect"),
        entry("Close Tab", "Ctrl W", "win.close-tab"),
        entry("Preferences", "Ctrl ,", "win.preferences"),
        entry("Keyboard Shortcuts", "Super K", "win.shortcuts"),
        entry("Reload ~/.ssh/config", "", "win.reload-ssh-config"),
        entry("Restore Hidden Records", "", "win.restore-hidden"),
        entry("Toggle Sidebar", "Ctrl B", "win.toggle-sidebar"),
        entry("About runetty", "", "win.about"),
    ]
}

/// Ranks entries against a query.
///
/// An empty query is not "no matches" but "nothing typed yet": recent servers
/// first, then the rest, which is what a palette should open showing.
pub fn search(entries: &[Entry], query: &str) -> Vec<Entry> {
    let query = query.trim();

    if query.is_empty() {
        let mut listed: Vec<Entry> = entries.to_vec();
        listed.sort_by(|a, b| {
            b.last_used
                .cmp(&a.last_used)
                .then_with(|| a.kind.cmp_order().cmp(&b.kind.cmp_order()))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        return listed;
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut scored: Vec<(u32, Entry)> = entries
        .iter()
        .filter_map(|entry| {
            let haystack = entry.haystack();
            let mut buffer = Vec::new();
            let text = nucleo_matcher::Utf32Str::new(&haystack, &mut buffer);
            pattern.score(text, &mut matcher).map(|score| (score, entry.clone()))
        })
        .collect();

    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.last_used.cmp(&left.last_used))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });

    scored.into_iter().map(|(_, entry)| entry).collect()
}

impl Kind {
    /// Servers before commands when nothing else separates them.
    fn cmp_order(self) -> u8 {
        match self {
            Self::Server => 0,
            Self::Command => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(title: &str, detail: &str, last_used: Option<u64>) -> Entry {
        Entry {
            title: title.into(),
            detail: detail.into(),
            kind: Kind::Server,
            action: "win.host-connect".into(),
            target: Some(title.into()),
            last_used,
        }
    }

    fn entries() -> Vec<Entry> {
        let mut all = vec![
            server("web-a", "deploy@web-a.prod", Some(200)),
            server("web-b", "deploy@web-b.prod", Some(100)),
            server("db-main", "psql@db.prod", None),
        ];
        all.extend(commands());
        all
    }

    /// An empty query opens on recent servers, not on nothing.
    #[test]
    fn empty_query_lists_recent_first() {
        let found = search(&entries(), "");
        assert_eq!(found[0].title, "web-a");
        assert_eq!(found[1].title, "web-b");
        assert_eq!(found[2].kind, Kind::Server, "unused server before commands");
    }

    /// Matching is fuzzy: scattered letters still find the entry.
    #[test]
    fn matches_scattered_letters() {
        let found = search(&entries(), "wba");
        assert_eq!(found.first().map(|e| e.title.as_str()), Some("web-a"));
    }

    /// The second line is searched too, so an address finds its server.
    #[test]
    fn searches_the_detail_line() {
        let found = search(&entries(), "psql");
        assert_eq!(found.first().map(|e| e.title.as_str()), Some("db-main"));
    }

    #[test]
    fn finds_commands_by_name() {
        let found = search(&entries(), "prefer");
        assert_eq!(found.first().map(|e| e.title.as_str()), Some("Preferences"));
        assert_eq!(found[0].action, "win.preferences");
    }

    /// Equal scores break towards the server used more recently.
    #[test]
    fn recent_wins_a_tie() {
        let found = search(&entries(), "web-");
        assert_eq!(found[0].title, "web-a");
        assert_eq!(found[1].title, "web-b");
    }

    #[test]
    fn no_match_is_empty() {
        assert!(search(&entries(), "zzzznothing").is_empty());
    }
}
