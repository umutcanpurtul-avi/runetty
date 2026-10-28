//! Parolalar sistem keyring'inde (Secret Service: GNOME Keyring / KWallet)
//! tutulur. hosts.toml'a ya da baska bir dosyaya asla duz metin yazilmaz.
//!
//! Parolayi ssh'a verme yolu `SSH_ASKPASS`: ssh, parola sordugunda bu programi
//! calistirip stdout'undan okur. Boylece `sshpass` gibi komut satirinda parola
//! tasiyan bir araca ihtiyac kalmiyor.

use libsecret::{Schema, SchemaAttributeType, SchemaFlags};
use std::collections::HashMap;

const SCHEMA_NAME: &str = "dev.runetty.Runetty.Password";
const ATTR_ALIAS: &str = "alias";

fn schema() -> Schema {
    let mut attributes = HashMap::new();
    attributes.insert(ATTR_ALIAS, SchemaAttributeType::String);
    Schema::new(SCHEMA_NAME, SchemaFlags::NONE, attributes)
}

fn attributes(alias: &str) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    map.insert(ATTR_ALIAS, alias);
    map
}

pub fn store(alias: &str, password: &str) -> Result<(), String> {
    libsecret::password_store_sync(
        Some(&schema()),
        attributes(alias),
        Some(libsecret::COLLECTION_DEFAULT),
        &format!("runetty: {alias}"),
        password,
        gtk::gio::Cancellable::NONE,
    )
    .map_err(|error| error.to_string())
}

pub fn lookup(alias: &str) -> Option<String> {
    libsecret::password_lookup_sync(
        Some(&schema()),
        attributes(alias),
        gtk::gio::Cancellable::NONE,
    )
    .ok()
    .flatten()
    .map(|password| password.to_string())
}

pub fn clear(alias: &str) -> Result<(), String> {
    libsecret::password_clear_sync(
        Some(&schema()),
        attributes(alias),
        gtk::gio::Cancellable::NONE,
    )
    .map_err(|error| error.to_string())
}

pub fn has(alias: &str) -> bool {
    lookup(alias).is_some()
}
