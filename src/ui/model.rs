//! Sample data for the agent panel. Server records now come from
//! `crate::data`; agents will be wired to a real source later.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Blocked,
    Working,
    Done,
    Idle,
}

impl AgentState {
    /// CSS class name; the colour decision lives in style.css.
    pub fn css(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Working => "working",
            Self::Done => "done",
            Self::Idle => "idle",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Blocked => "needs you",
            Self::Working => "working",
            Self::Done => "done",
            Self::Idle => "idle",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Agent {
    pub name: &'static str,
    pub host: &'static str,
    pub state: AgentState,
}

pub const AGENTS: &[Agent] = &[
    Agent { name: "refactor-ui",   host: "build-arm", state: AgentState::Blocked },
    Agent { name: "log-triage",    host: "edge-01",   state: AgentState::Working },
    Agent { name: "nightly-tests", host: "web-a",     state: AgentState::Done },
    Agent { name: "backup-check",  host: "nas",       state: AgentState::Idle },
];
