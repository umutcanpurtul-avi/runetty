//! `~/.config/runetty/hosts.toml` — runetty'nin kendi kayitlari.
//!
//! ssh config'e hicbir zaman yazmayiz; oradan gelen bir kaydi "silmek"
//! burada gizleme olarak kaydedilir. Boylece `ssh`, `scp`, `rsync` etkilenmez.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{now, ssh_config, Host, Source};

#[derive(Default, Serialize, Deserialize)]
struct File {
    /// ssh config'ten gelip listede gosterilmeyen alias'lar.
    #[serde(default)]
    hidden: Vec<String>,
    /// alias -> son kullanim (unix saniye)
    #[serde(default)]
    last_used: HashMap<String, u64>,
    /// ssh config'ten gelen kayitlara runetty'nin ekledigi klasor.
    #[serde(default)]
    folders: HashMap<String, String>,
    #[serde(default, rename = "host")]
    hosts: Vec<LocalHost>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LocalHost {
    alias: String,
    hostname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
}

pub struct Store {
    file: File,
    path: Option<PathBuf>,
    /// Her yuklemede ssh config'ten tazelenir; bellekte tutulur, yazilmaz.
    from_ssh: Vec<Host>,
}

impl Store {
    pub fn load() -> Self {
        let path = config_path();
        let file = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| match toml::from_str(&text) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    eprintln!("runetty: hosts.toml okunamadi: {error}");
                    None
                }
            })
            .unwrap_or_default();

        Store { file, path, from_ssh: ssh_config::load() }
    }

    /// Iki kaynagin birlesimi: gizlenenler dusulur, klasor ve zaman eklenir.
    pub fn hosts(&self) -> Vec<Host> {
        let mut hosts: Vec<Host> = Vec::new();

        for host in &self.from_ssh {
            if self.file.hidden.iter().any(|alias| alias == &host.alias) {
                continue;
            }
            let mut host = host.clone();
            host.folder = self.file.folders.get(&host.alias).cloned();
            host.last_used = self.file.last_used.get(&host.alias).copied();
            hosts.push(host);
        }

        for local in &self.file.hosts {
            // Ayni alias ssh config'te de varsa yerel kayit kazanir.
            hosts.retain(|existing| existing.alias != local.alias);
            hosts.push(Host {
                alias: local.alias.clone(),
                hostname: local.hostname.clone(),
                user: local.user.clone(),
                port: local.port,
                folder: local.folder.clone(),
                last_used: self.file.last_used.get(&local.alias).copied(),
                source: Source::Local,
            });
        }

        hosts.sort_by(|a, b| a.alias.to_lowercase().cmp(&b.alias.to_lowercase()));
        hosts
    }

    /// Son kullanilanlar, yeniden eskiye.
    pub fn recents(&self, limit: usize) -> Vec<Host> {
        let mut used: Vec<Host> =
            self.hosts().into_iter().filter(|host| host.last_used.is_some()).collect();
        used.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        used.truncate(limit);
        used
    }


    pub fn touch(&mut self, alias: &str) {
        self.file.last_used.insert(alias.to_string(), now());
        self.save();
    }

    pub fn upsert(&mut self, host: LocalInput) {
        let entry = LocalHost {
            alias: host.alias.clone(),
            hostname: host.hostname,
            user: host.user,
            port: host.port,
            folder: host.folder,
        };

        match self.file.hosts.iter_mut().find(|h| h.alias == entry.alias) {
            Some(existing) => *existing = entry,
            None => self.file.hosts.push(entry),
        }

        // Yerel kayit ayni adli gizlemeyi gecersiz kilar.
        self.file.hidden.retain(|alias| alias != &host.alias);
        self.save();
    }

    /// Yerel kayitta gercek silme, ssh config kaydinda gizleme.
    /// Donus: gercekten silindi mi (false ise gizlendi).
    pub fn remove(&mut self, alias: &str) -> bool {
        let before = self.file.hosts.len();
        self.file.hosts.retain(|host| host.alias != alias);
        let deleted = self.file.hosts.len() != before;

        if !deleted && self.from_ssh.iter().any(|host| host.alias == alias) {
            if !self.file.hidden.iter().any(|hidden| hidden == alias) {
                self.file.hidden.push(alias.to_string());
            }
        }

        self.file.last_used.remove(alias);
        self.file.folders.remove(alias);
        self.save();
        deleted
    }


    /// Gizlenenleri geri getirir; "silmeyi geri al" icin.
    pub fn unhide_all(&mut self) {
        self.file.hidden.clear();
        self.save();
    }


    pub fn reload_ssh_config(&mut self) {
        self.from_ssh = ssh_config::load();
    }


    fn save(&self) {
        let Some(path) = &self.path else { return };

        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("runetty: {} olusturulamadi: {error}", parent.display());
                return;
            }
        }

        match toml::to_string_pretty(&self.file) {
            Ok(text) => {
                // Yazma yarida kalirsa eski dosya bozulmasin.
                let temporary = path.with_extension("toml.tmp");
                if let Err(error) =
                    std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path))
                {
                    eprintln!("runetty: hosts.toml yazilamadi: {error}");
                }
            }
            Err(error) => eprintln!("runetty: hosts.toml serilestirilemedi: {error}"),
        }
    }
}

/// Diyalogdan gelen ham girdi.
pub struct LocalInput {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub folder: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("runetty/hosts.toml"))
}
