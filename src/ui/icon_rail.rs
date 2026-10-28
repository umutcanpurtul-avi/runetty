//! Mod secici: uc mod, hep ayni yerde, isim kaplamadan.
//! Kenar cubugunun ust barinda, listenin tam ustunde duruyor.

use gtk::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sessions,
    Agents,
    Monitor,
}

impl Mode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sessions" => Some(Self::Sessions),
            "agents" => Some(Self::Agents),
            "monitor" => Some(Self::Monitor),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Agents => "agents",
            Self::Monitor => "monitor",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Sessions => "rail-sessions-symbolic",
            Self::Agents => "rail-agents-symbolic",
            Self::Monitor => "rail-monitor-symbolic",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Sessions => "Sessions",
            Self::Agents => "Agents",
            Self::Monitor => "Monitor",
        }
    }
}

pub fn build<F>(initial: Mode, on_change: F) -> gtk::Box
where
    F: Fn(Mode) + Clone + 'static,
{
    let rail = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    rail.add_css_class("icon-rail");

    let modes = [Mode::Sessions, Mode::Agents, Mode::Monitor];
    let mut group: Option<gtk::ToggleButton> = None;

    for mode in modes {
        let button = gtk::ToggleButton::new();
        button.set_child(Some(&gtk::Image::from_icon_name(mode.icon())));
        button.set_tooltip_text(Some(mode.tooltip()));

        match &group {
            Some(first) => button.set_group(Some(first)),
            None => group = Some(button.clone()),
        }
        button.set_active(mode == initial);

        let on_change = on_change.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                on_change(mode);
            }
        });

        rail.append(&button);
    }

    rail
}
