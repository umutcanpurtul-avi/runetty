//! Sunucu kayitlarinin veri katmani.
//!
//! Iki kaynak var ve ayrimi bilerek koruyoruz:
//!   - `~/.ssh/config`  : `ssh`, `scp`, `rsync` ile paylasilan gercek ayar.
//!                        Buradan gelen kayitlari runetty silmez, yalniz gizler.
//!   - `hosts.toml`     : runetty'nin kendi ekledikleri + klasor, etiket, zaman.
//!
//! Plandaki karar: baglanti alanlari ssh config'e, gorunum alanlari TOML'a.

pub mod hostkey;
pub mod metrics;
pub mod secrets;
pub mod settings;
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

    /// Baglanti icin gereken butun `ssh` argumanlari — hedef dahil.
    ///
    /// Terminal oturumu da Monitor olcumu de buradan besleniyor. Tek kaynak
    /// olmasi onemli: port ve anahtar gibi alanlar bir tarafta unutulursa
    /// Monitor yanlis sunucuya ya da yanlis kimlikle baglanir.
    pub fn ssh_args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec!["ssh".into()];
        args.extend(control_options());

        if self.source == Source::SshConfig {
            // Port, anahtar, jump host: hepsi ssh config'te tanimli.
            args.push(self.alias.clone());
            return args;
        }

        if let Some(port) = self.port {
            if port != 22 {
                args.push("-p".into());
                args.push(port.to_string());
            }
        }

        if let Some(identity) = &self.identity_file {
            args.push("-i".into());
            args.push(identity.clone());
        }

        args.push(self.user_host_plain());
        args
    }

    /// Terminalde calistirilacak kabuk komutu.
    pub fn command(&self) -> String {
        self.ssh_args()
            .iter()
            .map(|arg| if needs_quoting(arg) { shell_quote(arg) } else { arg.clone() })
            .collect::<Vec<String>>()
            .join(" ")
    }

    fn user_host_plain(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.hostname),
            None => self.hostname.clone(),
        }
    }
}

fn needs_quoting(value: &str) -> bool {
    value.contains(|c: char| c.is_whitespace() || "'\"$`\\;&|<>()*?[]{}".contains(c))
}

/// Dosya yolu bosluk ya da ozel karakter icerebilir; kabuga tek tirnakla gecir.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Terminal oturumu ve Monitor turu ayni kalici kanali paylasiyor.
/// Boylece kimlik dogrulama sunucu basina bir kez yapiliyor: ilk oturumda
/// parola/anahtar verilince sonraki her baglanti ayni kanaldan geciyor.
pub fn control_options() -> Vec<String> {
    let Some(dir) = control_dir() else {
        return Vec::new();
    };
    control_options_in(&dir)
}

fn control_options_in(dir: &std::path::Path) -> Vec<String> {
    let path = format!("{}/cm-%C", dir.display());

    // Unix soket yolu 108 bayt ile sinirli ve `%C` acilinca daha da uzuyor.
    // Sinira yaklasiyorsa paylasimdan vazgec: yavas ama calisir, aksi halde
    // ssh "ControlPath too long" deyip hic baglanmaz.
    if path.len() + 24 >= 108 {
        return Vec::new();
    }

    let _ = std::fs::create_dir_all(&dir);

    vec![
        "-o".into(),
        format!("ControlPath={path}"),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        // Terminal kapansa bile kanal biraz yasasin: Monitor turu ve
        // hemen ardindan acilan ikinci oturum yeniden parola istemesin.
        "ControlPersist=600".into(),
    ]
}

/// Unix soket yolu kisa olmali; `%C` zaten kisa bir hash.
pub fn control_dir() -> Option<std::path::PathBuf> {
    dirs::runtime_dir().or_else(dirs::cache_dir).map(|dir| dir.join("runetty"))
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

#[cfg(test)]
mod control_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn short_path_enables_sharing() {
        let options = control_options_in(Path::new("/run/user/1000/runetty"));
        assert!(options.iter().any(|option| option.starts_with("ControlPath=")));
        assert!(options.iter().any(|option| option == "ControlMaster=auto"));
    }

    /// Unix soketi 108 bayt ile sinirli. "too long" deyip hic baglanamamaktansa
    /// kanal paylasimindan vazgecip calismaya devam etmek daha iyi.
    #[test]
    fn overlong_path_disables_sharing_instead_of_failing() {
        let deep = format!("/tmp/{}/runetty", "x".repeat(120));
        assert!(control_options_in(Path::new(&deep)).is_empty());
    }
}
