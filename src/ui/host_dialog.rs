//! Sunucu ekleme / duzenleme diyalogu.
//! Plandaki karar geregi yalniz runetty'nin kendi kaydina yazar;
//! ~/.ssh/config'ten gelen bir kayit duzenlenirse yerel kopya olusur.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

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

    let group = adw::PreferencesGroup::new();

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

    if let Some(host) = &existing {
        name.set_text(&host.alias);
        hostname.set_text(&host.hostname);
        user.set_text(host.user.as_deref().unwrap_or_default());
        folder.set_text(host.folder.as_deref().unwrap_or_default());
        port.set_value(host.port.unwrap_or(22) as f64);

        // ssh config'teki alias kimligi oradaki blokla eslesmeli.
        if host.source == Source::SshConfig {
            name.set_editable(false);
            name.set_tooltip_text(Some("~/.ssh/config icinde tanimli; adi orada degisir"));
        }
    }

    for row in [&name, &hostname, &user, &folder] {
        group.add(row);
    }
    group.add(&port);

    let dialog = adw::AlertDialog::new(
        Some(if existing.is_some() { "Edit Server" } else { "New Server" }),
        None,
    );
    dialog.set_extra_child(Some(&group));
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
        let host = hostname.text().trim().to_string();
        if alias.is_empty() {
            dialog.set_body("Name bos birakilamaz.");
            return;
        }

        let input = LocalInput {
            hostname: if host.is_empty() { alias.clone() } else { host },
            alias,
            user: non_empty(&user.text()),
            port: match port.value() as u16 {
                22 => None,
                other => Some(other),
            },
            folder: non_empty(&folder.text()),
        };

        store.borrow_mut().upsert(input);
        on_saved();
    });

    dialog.present(Some(parent));
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
