//! Appearance preferences: `~/.config/runetty/settings.toml`.
//! Kept apart from server records: one is shareable configuration, the other
//! is a matter of taste belonging to this machine alone.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Let the desktop decide.
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn all() -> [Theme; 3] {
        [Theme::System, Theme::Light, Theme::Dark]
    }

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }
}

/// An empty field means "whatever the theme picks".
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    pub interface_font: String,
    pub interface_size: f64,
    pub terminal_font: String,
    pub terminal_size: f64,
}

impl Default for Appearance {
    fn default() -> Self {
        // Family left empty so the fallback chain in style.css stays in force.
        // If Inter is not installed the chain already moves on; hardcoding a
        // name here would impose a font that may not exist.
        Self {
            theme: Theme::default(),
            interface_font: String::new(),
            interface_size: 13.0,
            terminal_font: String::new(),
            terminal_size: 11.0,
        }
    }
}

impl Appearance {
    /// Pango description, e.g. "Inter 13". An empty family defers to the theme.
    pub fn interface_pango(&self) -> String {
        format!("{} {}", self.family(&self.interface_font, "sans"), self.interface_size)
    }

    pub fn terminal_pango(&self) -> String {
        format!("{} {}", self.family(&self.terminal_font, "monospace"), self.terminal_size)
    }

    fn family<'a>(&self, value: &'a str, fallback: &'a str) -> &'a str {
        let value = value.trim();
        if value.is_empty() {
            fallback
        } else {
            value
        }
    }

    /// A thin layer over the constants in style.css.
    /// When the family is empty we omit the rule entirely so the stylesheet's
    /// fallback chain (Inter -> Adwaita Sans -> Cantarell) stays intact.
    pub fn css(&self) -> String {
        let mut css = String::new();

        css.push_str(&match self.interface_font.trim() {
            "" => format!("window.runetty {{ font-size: {}px; }}\n", self.interface_size),
            family => format!(
                "window.runetty {{ font-family: {}; font-size: {}px; }}\n",
                quote_family(family),
                self.interface_size
            ),
        });

        if let family @ 1.. = self.terminal_font.trim().len() {
            let _ = family;
            css.push_str(&format!(
                ".mono, .row-subtitle {{ font-family: {}; }}\n",
                quote_family(&self.terminal_font)
            ));
        }

        css
    }
}

/// Quote the family name if it contains spaces; keep the fallback chain.
fn quote_family(value: &str) -> String {
    format!("\"{}\", \"Adwaita Sans\", Cantarell, sans-serif", value.trim().replace('"', ""))
}

/// Window layout: preferences preserved across sessions.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Is Monitor pinned to the right-hand pane?
    pub monitor_pinned: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub layout: Layout,
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };

        toml::from_str(&text).unwrap_or_else(|error| {
            eprintln!("runetty: could not read settings.toml: {error}");
            Self::default()
        })
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };

        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("runetty: could not create {}: {error}", parent.display());
                return;
            }
        }

        match toml::to_string_pretty(self) {
            Ok(text) => {
                let temporary = path.with_extension("toml.tmp");
                if let Err(error) = std::fs::write(&temporary, text)
                    .and_then(|()| std::fs::rename(&temporary, &path))
                {
                    eprintln!("runetty: could not write settings.toml: {error}");
                }
            }
            Err(error) => eprintln!("runetty: could not serialise settings.toml: {error}"),
        }
    }
}

fn path() -> Option<PathBuf> {
    super::store::config_dir().map(|dir| dir.join("settings.toml"))
}
