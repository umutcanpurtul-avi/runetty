//! The "do you trust this host?" dialog shown on first connect.
//! The UI counterpart of the question ssh asks in a terminal.

use adw::prelude::*;

use crate::data::hostkey;

/// Shows the dialog when the key is unknown and runs `proceed` on approval.
/// Known or undecidable keys go straight to `proceed`.
pub fn ensure_trusted<F>(
    parent: &impl IsA<gtk::Widget>,
    host: &str,
    port: u16,
    proceed: F,
) where
    F: Fn() + 'static,
{
    if hostkey::status(host, port) != hostkey::Known::No {
        proceed();
        return;
    }

    let keys = match hostkey::scan(host, port) {
        Ok(keys) => keys,
        Err(reason) => {
            // No key available: leave it to ssh, whose error will show up there.
            eprintln!("runetty: could not fetch key for {host}: {reason}");
            proceed();
            return;
        }
    };

    let dialog = adw::AlertDialog::new(
        Some("Unknown host"),
        Some(&format!(
            "This is the first connection to {host}. Compare the fingerprint \
             with the value you expect before trusting it."
        )),
    );

    dialog.set_extra_child(Some(&fingerprint_list(&keys)));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("trust", "Trust and connect");
    dialog.set_response_appearance("trust", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response != "trust" {
            return;
        }

        if let Err(error) = hostkey::trust(&keys) {
            eprintln!("runetty: could not store key: {error}");
            return;
        }

        proceed();
    });

    dialog.present(Some(parent));
}

fn fingerprint_list(keys: &[String]) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    group.set_title("Fingerprint");

    let fingerprints = hostkey::fingerprints(keys);
    if fingerprints.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("Could not compute fingerprint");
        row.set_subtitle("ssh-keygen not found");
        group.add(&row);
        return group.upcast();
    }

    for (kind, fingerprint) in fingerprints {
        let row = adw::ActionRow::new();
        row.set_title(&kind);
        row.set_subtitle(&fingerprint);
        row.set_subtitle_selectable(true);
        row.add_css_class("mono");
        group.add(&row);
    }

    group.upcast()
}
