//! Sunucu kayitlarinin veri katmani.
//!
//! Iki kaynak var ve ayrimi bilerek koruyoruz:
//!   - `~/.ssh/config`  : `ssh`, `scp`, `rsync` ile paylasilan gercek ayar.
//!                        Buradan gelen kayitlari runetty silmez, yalniz gizler.
//!   - `hosts.toml`     : runetty'nin kendi ekledikleri + klasor, etiket, zaman.
//!
//! Plandaki karar: baglanti alanlari ssh config'e, gorunum alanlari TOML'a.

pub mod secrets;
pub mod sessions;
pub mod ssh_config;
pub mod store;

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// ~/.ssh/config icinden okundu.
    SshConfig,
    /// runetty'nin kendi kaydi.
    Local,
}

#[derive(Clone, Debug)]
pub struct Host {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub folder: Option<String>,
    /// `-i` ile gecilen ozel anahtar. ssh config'te `IdentityFile`.
    pub identity_file: Option<String>,
    pub last_used: Option<u64>,
    pub source: Source,
}

impl Host {
    /// Liste satirinin ikinci katmani: `user@host[:port]`
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

    /// ssh config'ten gelen kayitta alias'i kullan: Jump host, anahtar, keep-alive
    /// gibi ayarlar zaten orada tanimli, komut satirinda tekrarlamayalim.
    pub fn command(&self) -> String {
        match self.source {
            Source::SshConfig => format!("ssh {}", self.alias),
            Source::Local => {
                let mut cmd = String::from("ssh ");
                if let Some(port) = self.port {
                    if port != 22 {
                        cmd.push_str(&format!("-p {port} "));
                    }
                }
                if let Some(identity) = &self.identity_file {
                    cmd.push_str(&format!("-i {} ", shell_quote(identity)));
                }
                cmd.push_str(&self.user_host_plain());
                cmd
            }
        }
    }

    fn user_host_plain(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.hostname),
            None => self.hostname.clone(),
        }
    }
}

/// Dosya yolu bosluk ya da ozel karakter icerebilir; kabuga tek tirnakla gecir.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "2m", "18m", "3h", "6d" — sagda duran goreli zaman.
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
