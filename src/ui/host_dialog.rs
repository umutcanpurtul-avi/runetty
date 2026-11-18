//! Dialog for adding and editing a server.
//!
//! Connection fields are written to runetty's own record; ~/.ssh/config is
//! left alone. The password goes to the system keyring, never to a file.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

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
    present_in_folder(parent, store, alias, None, on_saved)
}

/// `prefill_folder`: a new record started from the + beside a folder should
/// begin in that folder.
pub fn present_in_folder<F>(
    parent: &impl IsA<gtk::Widget>,
    store: Rc<RefCell<Store>>,
    alias: Option<String>,
    prefill_folder: Option<String>,
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

    // Existing folders can be picked from a menu; typing a new name is free.
    let known = store.borrow().folders();
    if !known.is_empty() {
        let menu = gio::Menu::new();
        for name in &known {
            let item = gio::MenuItem::new(Some(name), None);
            item.set_action_and_target_value(
                Some("folder.pick"),
                Some(&name.to_variant()),
            );
            menu.append_item(&item);
        }

        let pick = gio::SimpleAction::new("pick", Some(glib::VariantTy::STRING));
        {
            let folder = folder.clone();
            pick.connect_activate(move |_, target| {
                if let Some(name) = target.and_then(glib::Variant::str) {
                    folder.set_text(name);
                }
            });
        }

        let actions = gio::SimpleActionGroup::new();
        actions.add_action(&pick);
        folder.insert_action_group("folder", Some(&actions));

        let button = gtk::MenuButton::new();
        button.set_icon_name("pan-down-symbolic");
        button.add_css_class("flat");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some("Existing folders"));
        button.set_menu_model(Some(&menu));
        folder.add_suffix(&button);
    }

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
        "If a key is set, ssh tries it first. The password is stored in the system \
         keyring and never written to any file in plain text.",
    ));

    let key = adw::EntryRow::new();
    key.set_title("SSH Key");

    // Left empty: the user picks the key, not the app.
    // Keys in ~/.ssh are offered in a menu; any other file can be chosen too.
    let keys = available_keys();
    if !keys.is_empty() {
        let menu = gio::Menu::new();
        for path in &keys {
            let label = path.rsplit('/').next().unwrap_or(path);
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some("key.pick"), Some(&path.to_variant()));
            menu.append_item(&item);
        }

        let pick = gio::SimpleAction::new("pick", Some(glib::VariantTy::STRING));
        {
            let key = key.clone();
            pick.connect_activate(move |_, target| {
                if let Some(path) = target.and_then(glib::Variant::str) {
                    key.set_text(path);
                }
            });
        }

        let actions = gio::SimpleActionGroup::new();
        actions.add_action(&pick);
        key.insert_action_group("key", Some(&actions));

        let list = gtk::MenuButton::new();
        list.set_icon_name("pan-down-symbolic");
        list.add_css_class("flat");
        list.set_valign(gtk::Align::Center);
        list.set_tooltip_text(Some("Keys in ~/.ssh"));
        list.set_menu_model(Some(&menu));
        key.add_suffix(&list);
    }

    let browse = gtk::Button::from_icon_name("document-open-symbolic");
    browse.add_css_class("flat");
    browse.set_valign(gtk::Align::Center);
    browse.set_tooltip_text(Some("Choose another file"));
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

    if let Some(folder_name) = &prefill_folder {
        folder.set_text(folder_name);
    }

    if let Some(host) = &existing {
        name.set_text(&host.alias);
        hostname.set_text(&host.hostname);
        user.set_text(host.user.as_deref().unwrap_or_default());
        folder.set_text(host.folder.as_deref().unwrap_or_default());
        key.set_text(host.identity_file.as_deref().unwrap_or_default());
        port.set_value(host.port.unwrap_or(22) as f64);

        if secrets::has(&host.alias) {
            password.set_text("");
            password.set_tooltip_text(Some("A password is stored in the keyring"));
            forget.set_visible(true);
        }

        if host.source == Source::SshConfig {
            name.set_editable(false);
            name.set_tooltip_text(Some(
                "Defined in ~/.ssh/config; rename it there",
            ));
        }
    }

    {
        let alias = existing.as_ref().map(|host| host.alias.clone());
        let password = password.clone();
        forget.connect_clicked(move |button| {
            if let Some(alias) = &alias {
                if let Err(error) = secrets::clear(alias) {
                    eprintln!("runetty: could not clear password: {error}");
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
            dialog.set_body("Name cannot be empty.");
            return;
        }

        let starts_with_dash = |value: &str| value.trim_start().starts_with('-');
        if starts_with_dash(&alias)
            || starts_with_dash(&hostname.text())
            || starts_with_dash(&user.text())
        {
            dialog.set_body("Name, hostname and user cannot start with '-'.");
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
                eprintln!("runetty: could not store password in keyring: {error}");
            }
        }

        on_saved();
    });

    dialog.present(Some(parent));
}

fn choose_key(parent: &adw::AlertDialog, target: &adw::EntryRow) {
    let chooser = gtk::FileDialog::new();
    chooser.set_title("Choose an SSH key");

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

/// Private keys under ~/.ssh, identified by having a matching `.pub` file.
/// That naturally filters out `config`, `known_hosts` and the `.pub` files.
fn available_keys() -> Vec<String> {
    let Some(ssh) = dirs::home_dir().map(|home| home.join(".ssh")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&ssh) else {
        return Vec::new();
    };

    let mut keys: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.extension().is_none_or(|ext| ext != "pub"))
        .filter(|path| ssh.join(format!("{}.pub", file_name(path))).is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();

    keys.sort();
    keys
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
