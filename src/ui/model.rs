//! Agent paneli icin ornek veri. Sunucu kayitlari artik `crate::data`'dan
//! geliyor; agent'lar faz 3'te gercek kaynaga baglanacak.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Blocked,
    Working,
    Done,
    Idle,
}

impl AgentState {
    /// CSS sinif adi; renk karari style.css'te.
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
