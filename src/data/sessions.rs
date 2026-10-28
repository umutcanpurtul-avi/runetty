//! Which hosts have a live session? The sidebar status dot reads from here.
//! Updated as tabs open and close; never written to disk.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Tab is open and its process is running.
    Open,
    /// Tab is still open but the process exited non-zero.
    Trouble,
    /// No open session.
    Closed,
}

impl State {
    pub fn css(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Trouble => "trouble",
            Self::Closed => "closed",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Open => "Connected",
            Self::Trouble => "Connection dropped",
            Self::Closed => "No open session",
        }
    }
}

#[derive(Default)]
pub struct Sessions {
    states: RefCell<HashMap<String, State>>,
}

impl Sessions {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    pub fn state(&self, alias: &str) -> State {
        self.states.borrow().get(alias).copied().unwrap_or(State::Closed)
    }

    pub fn set(&self, alias: &str, state: State) {
        match state {
            State::Closed => {
                self.states.borrow_mut().remove(alias);
            }
            other => {
                self.states.borrow_mut().insert(alias.to_string(), other);
            }
        }
    }
}
