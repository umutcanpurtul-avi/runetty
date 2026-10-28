//! Gorunum tercihleri: `~/.config/runetty/settings.toml`.
//! Sunucu kayitlarindan ayri duruyor cunku biri paylasilabilir yapilandirma,
//! digeri tamamen bu makineye ait zevk meselesi.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Masaustunun karar vermesine birak.
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
            Theme::System => "Sistem",
            Theme::Light => "Acik",
            Theme::Dark => "Koyu",
        }
    }
}

/// Bos birakilan alan "temanin kendi secimi" demek.
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
        Self {
            theme: Theme::default(),
            interface_font: "Inter".into(),
            interface_size: 13.0,
            terminal_font: "JetBrains Mono".into(),
            terminal_size: 11.0,
        }
    }
}

impl Appearance {
    /// Pango tanimi: "Inter 13". Bos aile temanin fontuna birakir.
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

    /// style.css'teki sabitleri ezen kucuk bir katman.
    pub fn css(&self) -> String {
        let interface = quote_family(&self.interface_font);
        let terminal = quote_family(&self.terminal_font);

        format!(
            "window.runetty {{ font-family: {interface}; font-size: {}px; }}\n\
             .mono, .row-subtitle {{ font-family: {terminal}; }}\n",
            self.interface_size
        )
    }
}

/// Aile adi bosluk iceriyorsa tirnakla; yedek zinciri koru.
fn quote_family(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "\"Adwaita Sans\", Cantarell, sans-serif".into();
    }

    format!("\"{}\", \"Adwaita Sans\", Cantarell, sans-serif", value.replace('"', ""))
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };

        toml::from_str(&text).unwrap_or_else(|error| {
            eprintln!("runetty: settings.toml okunamadi: {error}");
            Self::default()
        })
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };

        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("runetty: {} olusturulamadi: {error}", parent.display());
                return;
            }
        }

        match toml::to_string_pretty(self) {
            Ok(text) => {
                let temporary = path.with_extension("toml.tmp");
                if let Err(error) = std::fs::write(&temporary, text)
                    .and_then(|()| std::fs::rename(&temporary, &path))
                {
                    eprintln!("runetty: settings.toml yazilamadi: {error}");
                }
            }
            Err(error) => eprintln!("runetty: settings.toml serilestirilemedi: {error}"),
        }
    }
}

fn path() -> Option<PathBuf> {
    super::store::config_dir().map(|dir| dir.join("settings.toml"))
}
