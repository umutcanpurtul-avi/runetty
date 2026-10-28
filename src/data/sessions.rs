//! Hangi sunucunun acik oturumu var? Kenar cubugundaki durum noktasi buradan
//! besleniyor. Sekme acilip kapandikca guncellenir, diske yazilmaz.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Sekmesi acik ve sureci calisiyor.
    Open,
    /// Sekme hala acik ama surec sifirdan farkli kodla bitti.
    Trouble,
    /// Acik oturum yok.
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
            Self::Open => "Bagli",
            Self::Trouble => "Baglanti dustu",
            Self::Closed => "Acik oturum yok",
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
