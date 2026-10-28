//! `~/.config/runetty/hosts.toml` - runetty's own records.
//!
//! We never write to ssh config; "deleting" a record that came from there is
//! recorded here as hiding it, so `ssh`, `scp` and `rsync` stay untouched.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{now, ssh_config, Host, Source};

#[derive(Default, Serialize, Deserialize)]
struct File {
    /// Aliases from ssh config that are not shown in the list.
    #[serde(default)]
    hidden: Vec<String>,
    /// alias -> last used (unix seconds)
    #[serde(default)]
    last_used: HashMap<String, u64>,
    /// Folder runetty assigns to records that come from ssh config.
    #[serde(default)]
    folders: HashMap<String, String>,
    /// Records watched by Monitor. Both ssh config and local records collect
    /// here; watching is a presentation choice, not a connection setting.
    #[serde(default)]
    monitored: Vec<String>,
    /// Folders that are still empty. They stay listed once hosts move in, so
    /// a folder does not vanish when its last host is moved away.
    #[serde(default)]
    empty_folders: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_file: Option<String>,
}

pub struct Store {
    file: File,
    path: Option<PathBuf>,
    /// Refreshed from ssh config on each load; kept in memory, never written.
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
                    eprintln!("runetty: could not read hosts.toml: {error}");
                    None
                }
            })
            .unwrap_or_default();

        Store { file, path, from_ssh: ssh_config::load() }
    }

    /// Union of both sources: hidden entries dropped, folder and time added.
    pub fn hosts(&self) -> Vec<Host> {
        let mut hosts: Vec<Host> = Vec::new();

        for host in &self.from_ssh {
            if self.file.hidden.iter().any(|alias| alias == &host.alias) {
                continue;
            }
            let mut host = host.clone();
            host.folder = self.file.folders.get(&host.alias).cloned();
            host.last_used = self.file.last_used.get(&host.alias).copied();
            host.monitored = self.file.monitored.contains(&host.alias);
            hosts.push(host);
        }

        for local in &self.file.hosts {
            // If the same alias also exists in ssh config, the local record wins.
            hosts.retain(|existing| existing.alias != local.alias);
            hosts.push(Host {
                alias: local.alias.clone(),
                hostname: local.hostname.clone(),
                user: local.user.clone(),
                port: local.port,
                folder: local.folder.clone(),
                identity_file: local.identity_file.clone(),
                last_used: self.file.last_used.get(&local.alias).copied(),
                monitored: self.file.monitored.contains(&local.alias),
                source: Source::Local,
            });
        }

        hosts.sort_by(|a, b| a.alias.to_lowercase().cmp(&b.alias.to_lowercase()));
        hosts
    }

    /// Most recently used first.
    pub fn recents(&self, limit: usize) -> Vec<Host> {
        let mut used: Vec<Host> =
            self.hosts().into_iter().filter(|host| host.last_used.is_some()).collect();
        used.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        used.truncate(limit);
        used
    }


    /// Known folder names: those in use plus manually created empty ones.
    pub fn folders(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.hosts().into_iter().filter_map(|host| host.folder).collect();
        names.extend(self.file.empty_folders.iter().cloned());
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        names
    }

    pub fn add_folder(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.folders().iter().any(|known| known == name) {
            return;
        }

        self.file.empty_folders.push(name.to_string());
        self.save();
    }

    /// Moves a host into a folder, or ungroups it with `None`.
    /// For records from ssh config the folder lives in a separate map,
    /// because we do not write to that file.
    pub fn set_folder(&mut self, alias: &str, folder: Option<&str>) {
        match self.file.hosts.iter_mut().find(|host| host.alias == alias) {
            Some(local) => local.folder = folder.map(str::to_string),
            None => match folder {
                Some(name) => {
                    self.file.folders.insert(alias.to_string(), name.to_string());
                }
                None => {
                    self.file.folders.remove(alias);
                }
            },
        }

        self.save();
    }

    /// Renames a folder; the records inside take the new name.
    pub fn rename_folder(&mut self, old: &str, new: &str) {
        let new = new.trim();
        if new.is_empty() || new == old {
            return;
        }

        for host in self.file.hosts.iter_mut() {
            if host.folder.as_deref() == Some(old) {
                host.folder = Some(new.to_string());
            }
        }

        for folder in self.file.folders.values_mut() {
            if folder == old {
                *folder = new.to_string();
            }
        }

        for folder in self.file.empty_folders.iter_mut() {
            if folder == old {
                *folder = new.to_string();
            }
        }

        self.save();
    }

    pub fn remove_folder(&mut self, name: &str) {
        self.file.empty_folders.retain(|folder| folder != name);
        for host in self.file.hosts.iter_mut() {
            if host.folder.as_deref() == Some(name) {
                host.folder = None;
            }
        }
        self.file.folders.retain(|_, folder| folder != name);
        self.save();
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
            identity_file: host.identity_file,
        };

        match self.file.hosts.iter_mut().find(|h| h.alias == entry.alias) {
            Some(existing) => *existing = entry,
            None => self.file.hosts.push(entry),
        }

        // A local record overrides a hide with the same name.
        self.file.hidden.retain(|alias| alias != &host.alias);
        self.save();
    }

    /// Real deletion for a local record, hiding for an ssh config one.
    /// Returns whether it was actually deleted (false means hidden).
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
        self.file.monitored.retain(|name| name != alias);
        // If the record goes, its password should not linger in the keyring.
        // With no persistent storage (tests) we leave the keyring alone.
        if self.path.is_some() {
            let _ = super::secrets::clear(alias);
        }
        self.save();
        deleted
    }


    /// Records watched by Monitor.
    pub fn monitored(&self) -> Vec<Host> {
        self.hosts().into_iter().filter(|host| host.monitored).collect()
    }

    pub fn is_monitored(&self, alias: &str) -> bool {
        self.file.monitored.iter().any(|name| name == alias)
    }

    pub fn set_monitored(&mut self, alias: &str, monitored: bool) {
        let already = self.is_monitored(alias);
        if already == monitored {
            return;
        }

        if monitored {
            self.file.monitored.push(alias.to_string());
        } else {
            self.file.monitored.retain(|name| name != alias);
        }

        self.save();
    }

    pub fn hidden_count(&self) -> usize {
        self.file.hidden.len()
    }

    /// Brings hidden records back; the "undo delete" path.
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
                // If the write is cut short, the old file must stay intact.
                let temporary = path.with_extension("toml.tmp");
                if let Err(error) =
                    std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path))
                {
                    eprintln!("runetty: could not write hosts.toml: {error}");
                }
            }
            Err(error) => eprintln!("runetty: could not serialise hosts.toml: {error}"),
        }
    }
}

/// Raw input from the dialog.
pub struct LocalInput {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub folder: Option<String>,
    pub identity_file: Option<String>,
}

/// Inside Flatpak, XDG_CONFIG_HOME points at the sandbox's own directory.
/// To share settings with a native install we use the host's ~/.config
/// explicitly; the manifest grants access to that path.
pub fn config_dir() -> Option<PathBuf> {
    let base = if std::path::Path::new("/.flatpak-info").exists() {
        dirs::home_dir()?.join(".config")
    } else {
        dirs::config_dir()?
    };

    Some(base.join("runetty"))
}

fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("hosts.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With `path: None` save() returns early, so tests never touch the disk.
    fn store(hosts: Vec<LocalHost>, from_ssh: Vec<Host>) -> Store {
        Store {
            file: File { hosts, ..Default::default() },
            path: None,
            from_ssh,
        }
    }

    fn local(alias: &str) -> LocalHost {
        LocalHost {
            alias: alias.into(),
            hostname: "example.test".into(),
            user: None,
            port: None,
            folder: None,
            identity_file: None,
        }
    }

    fn from_config(alias: &str) -> Host {
        Host {
            alias: alias.into(),
            hostname: "example.test".into(),
            user: None,
            port: None,
            folder: None,
            identity_file: None,
            last_used: None,
            monitored: false,
            source: Source::SshConfig,
        }
    }

    /// Watching must be off by default, or ssh runs every two seconds for
    /// every single record.
    #[test]
    fn monitoring_is_opt_in() {
        let mut store = store(vec![local("box")], Vec::new());
        assert!(store.monitored().is_empty());

        store.set_monitored("box", true);
        assert_eq!(store.monitored().len(), 1);

        store.set_monitored("box", false);
        assert!(store.monitored().is_empty());
    }

    #[test]
    fn deleting_a_host_stops_monitoring_it() {
        let mut store = store(vec![local("box")], Vec::new());
        store.set_monitored("box", true);

        store.remove("box");

        assert!(store.monitored().is_empty());
        assert!(!store.is_monitored("box"));
    }

    #[test]
    fn moves_local_host_into_folder_and_out() {
        let mut store = store(vec![local("box")], Vec::new());

        store.set_folder("box", Some("lab"));
        assert_eq!(store.hosts()[0].folder.as_deref(), Some("lab"));

        store.set_folder("box", None);
        assert_eq!(store.hosts()[0].folder, None);
    }

    /// The folder for an ssh config record is kept in a separate map;
    /// the record is not turned into a local copy.
    #[test]
    fn folder_for_ssh_config_host_is_kept_aside() {
        let mut store = store(Vec::new(), vec![from_config("edge")]);

        store.set_folder("edge", Some("prod"));

        let hosts = store.hosts();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].source, Source::SshConfig);
        assert_eq!(hosts[0].folder.as_deref(), Some("prod"));
        assert!(store.file.hosts.is_empty(), "no local copy should be created");
    }

    #[test]
    fn deleting_ssh_config_host_hides_it_instead() {
        let mut store = store(Vec::new(), vec![from_config("edge")]);

        let deleted = store.remove("edge");

        assert!(!deleted, "an ssh config record must not report as deleted");
        assert!(store.hosts().is_empty(), "it should drop off the list");

        store.unhide_all();
        assert_eq!(store.hosts().len(), 1, "it should be restorable");
    }

    #[test]
    fn deleting_local_host_really_removes_it() {
        let mut store = store(vec![local("box")], Vec::new());

        assert!(store.remove("box"));
        assert!(store.hosts().is_empty());
    }

    #[test]
    fn renaming_a_folder_moves_its_hosts() {
        let mut local = local("box");
        local.folder = Some("lab".into());
        let mut store = store(vec![local], Vec::new());
        store.add_folder("lab");

        store.rename_folder("lab", "yeni-lab");

        assert_eq!(store.hosts()[0].folder.as_deref(), Some("yeni-lab"));
        assert_eq!(store.folders(), vec!["yeni-lab".to_string()]);
    }

    #[test]
    fn empty_folder_survives_until_removed() {
        let mut store = store(Vec::new(), Vec::new());

        store.add_folder("lab");
        assert_eq!(store.folders(), vec!["lab".to_string()]);

        store.remove_folder("lab");
        assert!(store.folders().is_empty());
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    /// If the record file silently breaks, the watch list and folders reset
    /// on every start. The round trip is pinned down here.
    #[test]
    fn file_survives_a_round_trip() {
        let mut file = File::default();
        file.hidden.push("gizli".into());
        file.monitored.push("izlenen".into());
        file.empty_folders.push("bos-klasor".into());
        file.last_used.insert("izlenen".into(), 1_700_000_000);
        file.folders.insert("gizli".into(), "lab".into());
        file.hosts.push(LocalHost {
            alias: "box".into(),
            hostname: "example.test".into(),
            user: Some("root".into()),
            port: Some(2222),
            folder: Some("lab".into()),
            identity_file: Some("/home/u/.ssh/id_ed25519".into()),
        });

        let text = toml::to_string_pretty(&file).expect("should serialise");
        let parsed: File = toml::from_str(&text).expect("should parse");

        assert_eq!(parsed.hidden, vec!["gizli".to_string()]);
        assert_eq!(parsed.monitored, vec!["izlenen".to_string()]);
        assert_eq!(parsed.empty_folders, vec!["bos-klasor".to_string()]);
        assert_eq!(parsed.last_used.get("izlenen"), Some(&1_700_000_000));
        assert_eq!(parsed.folders.get("gizli").map(String::as_str), Some("lab"));
        assert_eq!(parsed.hosts.len(), 1);
        assert_eq!(parsed.hosts[0].port, Some(2222));
        assert_eq!(parsed.hosts[0].identity_file.as_deref(), Some("/home/u/.ssh/id_ed25519"));
    }

    /// Fields absent from an older file must default, or an upgrade would
    /// drop every record.
    #[test]
    fn older_files_without_new_fields_still_load() {
        let text = "hidden = []\n\n[[host]]\nalias = \"box\"\nhostname = \"example.test\"\n";
        let parsed: File = toml::from_str(text).expect("should parse");

        assert!(parsed.monitored.is_empty());
        assert!(parsed.empty_folders.is_empty());
        assert_eq!(parsed.hosts.len(), 1);
    }
}
