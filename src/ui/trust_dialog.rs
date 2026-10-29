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
    let status = hostkey::status(host, port);
    if matches!(status, hostkey::Known::Yes | hostkey::Known::Unclear) {
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

    if status == hostkey::Known::Changed {
        present_changed(parent, host, port, keys, proceed);
        return;
    }

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

/// A recorded key no longer matches. This is the one case where ssh refuses
/// outright, and the one case that can mean an attack, so the dialog says so
/// plainly, shows both fingerprints, and defaults to cancelling.
fn present_changed<F>(
    parent: &impl IsA<gtk::Widget>,
    host: &str,
    port: u16,
    keys: Vec<String>,
    proceed: F,
) where
    F: Fn() + 'static,
{
    let dialog = adw::AlertDialog::new(
        Some("Host key has changed"),
        Some(&format!(
            "The key recorded for {host} no longer matches the one it now \
             presents.\n\nThis happens when a host is rebuilt or an address is \
             reused — but it is also what a man-in-the-middle attack looks \
             like. Only continue if you know why it changed."
        )),
    );

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);

    let recorded = hostkey::recorded_fingerprints(host, port);
    if !recorded.is_empty() {
        body.append(&fingerprint_group("Recorded until now", &recorded));
    }

    body.append(&fingerprint_group("Offered now", &hostkey::fingerprints(&keys)));
    dialog.set_extra_child(Some(&body));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("replace", "Replace key and connect");
    dialog.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let host = host.to_string();
    dialog.connect_response(None, move |_, response| {
        if response != "replace" {
            return;
        }

        // The old entry has to go first, or ssh keeps refusing on the stale one.
        if let Err(error) = hostkey::forget(&host, port) {
            eprintln!("runetty: could not remove the old key: {error}");
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

fn fingerprint_group(title: &str, fingerprints: &[(String, String)]) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    group.set_title(title);

    if fingerprints.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("Could not compute fingerprint");
        group.add(&row);
        return group.upcast();
    }

    for (kind, fingerprint) in fingerprints {
        let row = adw::ActionRow::new();
        row.set_title(kind);
        row.set_subtitle(fingerprint);
        row.set_subtitle_selectable(true);
        row.add_css_class("mono");
        group.add(&row);
    }

    group.upcast()
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
