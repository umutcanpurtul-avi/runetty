//! Sunucu ekleme / duzenleme diyalogu.
//!
//! Baglanti alanlari runetty'nin kendi kaydina yazilir; ~/.ssh/config'e
//! dokunulmaz. Parola dosyaya degil sistem keyring'ine gider.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::data::secrets;
use crate::data::store::{LocalInput, Store};
use crate::data::Source;

pub fn present<F>(
    parent: &impl IsA<gtk::Widget>,
    store: Rc<RefCell<Store>>,
    alias: Option<String>,
    on_saved: F,
) where
    F: Fn() + 'static,
{
    let existing = alias.as_ref().and_then(|alias| {
        store.borrow().hosts().into_iter().find(|host| &host.alias == alias)
    });

    let connection = adw::PreferencesGroup::new();
    connection.set_title("Connection");

    let name = adw::EntryRow::new();
    name.set_title("Name");
    let hostname = adw::EntryRow::new();
    hostname.set_title("Hostname");
    let user = adw::EntryRow::new();
    user.set_title("User");
    let folder = adw::EntryRow::new();
    folder.set_title("Folder");

    let port = adw::SpinRow::with_range(1.0, 65535.0, 1.0);
    port.set_title("Port");
    port.set_value(22.0);

    for row in [&name, &hostname, &user] {
        connection.add(row);
    }
    connection.add(&port);
    connection.add(&folder);

    let auth = adw::PreferencesGroup::new();
    auth.set_title("Authentication");
    auth.set_description(Some(
        "Anahtar varsa ssh once onu dener. Parola sistem keyring'ine kaydedilir, \
         hicbir dosyaya duz metin yazilmaz.",
    ));

    let key = adw::EntryRow::new();
    key.set_title("SSH Key");
    let browse = gtk::Button::from_icon_name("document-open-symbolic");
    browse.add_css_class("flat");
    browse.set_valign(gtk::Align::Center);
    browse.set_tooltip_text(Some("Anahtar dosyasi sec"));
    key.add_suffix(&browse);
    auth.add(&key);

    let password = adw::PasswordEntryRow::new();
    password.set_title("Password");
    auth.add(&password);

    let forget = gtk::Button::with_label("Forget");
    forget.add_css_class("flat");
    forget.set_valign(gtk::Align::Center);
    forget.set_visible(false);
    password.add_suffix(&forget);

    if let Some(host) = &existing {
        name.set_text(&host.alias);
        hostname.set_text(&host.hostname);
        user.set_text(host.user.as_deref().unwrap_or_default());
        folder.set_text(host.folder.as_deref().unwrap_or_default());
        key.set_text(host.identity_file.as_deref().unwrap_or_default());
        port.set_value(host.port.unwrap_or(22) as f64);

        if secrets::has(&host.alias) {
            password.set_text("");
            password.set_tooltip_text(Some("Keyring'de kayitli parola var"));
            forget.set_visible(true);
        }

        if host.source == Source::SshConfig {
            name.set_editable(false);
            name.set_tooltip_text(Some(
                "~/.ssh/config icinde tanimli; adi orada degisir",
            ));
        }
    } else {
        // Yeni kayitta en yaygin anahtari hazir onerelim.
        if let Some(default_key) = default_identity() {
            key.set_text(&default_key);
        }
    }

    {
        let alias = existing.as_ref().map(|host| host.alias.clone());
        let password = password.clone();
        forget.connect_clicked(move |button| {
            if let Some(alias) = &alias {
                if let Err(error) = secrets::clear(alias) {
                    eprintln!("runetty: parola silinemedi: {error}");
                    return;
                }
            }
            password.set_text("");
            password.set_tooltip_text(None);
            button.set_visible(false);
        });
    }

    let dialog = adw::AlertDialog::new(
        Some(if existing.is_some() { "Edit Server" } else { "New Server" }),
        None,
    );

    {
        let key = key.clone();
        let dialog = dialog.clone();
        browse.connect_clicked(move |_| choose_key(&dialog, &key));
    }

    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.append(&connection);
    form.append(&auth);

    dialog.set_extra_child(Some(&form));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |dialog, response| {
        if response != "save" {
            return;
        }

        let alias = name.text().trim().to_string();
        if alias.is_empty() {
            dialog.set_body("Name bos birakilamaz.");
            return;
        }

        let host = hostname.text().trim().to_string();
        let input = LocalInput {
            hostname: if host.is_empty() { alias.clone() } else { host },
            alias: alias.clone(),
            user: non_empty(&user.text()),
            port: match port.value() as u16 {
                22 => None,
                other => Some(other),
            },
            folder: non_empty(&folder.text()),
            identity_file: non_empty(&key.text()),
        };

        store.borrow_mut().upsert(input);

        let secret = password.text();
        if !secret.is_empty() {
            if let Err(error) = secrets::store(&alias, &secret) {
                eprintln!("runetty: parola keyring'e yazilamadi: {error}");
            }
        }

        on_saved();
    });

    dialog.present(Some(parent));
}

fn choose_key(parent: &adw::AlertDialog, target: &adw::EntryRow) {
    let chooser = gtk::FileDialog::new();
    chooser.set_title("SSH anahtari sec");

    if let Some(home) = dirs::home_dir() {
        let ssh = home.join(".ssh");
        if ssh.is_dir() {
            chooser.set_initial_folder(Some(&gio::File::for_path(&ssh)));
        }
    }

    let target = target.clone();
    let window = parent.root().and_downcast::<gtk::Window>();
    chooser.open(window.as_ref(), gio::Cancellable::NONE, move |result| {
        if let Ok(file) = result {
            if let Some(path) = file.path() {
                target.set_text(&path.to_string_lossy());
            }
        }
    });
}

/// ~/.ssh altindaki en yaygin varsayilan anahtar.
fn default_identity() -> Option<String> {
    let ssh = dirs::home_dir()?.join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| ssh.join(name))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
