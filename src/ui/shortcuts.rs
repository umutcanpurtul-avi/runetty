//! The keyboard shortcut reference.
//!
//! Written by hand rather than with `GtkShortcutsWindow`: that widget brings
//! its own styling, and it is deprecated from GTK 4.18 while the build floor
//! is 4.14. A plain dialog keeps one look across every supported version.

use adw::prelude::*;

/// A group of bindings. `keys` is false for the mouse section, whose right
/// column is a description rather than something to press.
struct Group {
    title: &'static str,
    keys: bool,
    entries: &'static [(&'static str, &'static str)],
}

/// Everything the window and the terminal bind, grouped the way someone
/// looking for a key would scan it.
const GROUPS: &[Group] = &[
    Group {
        title: "General",
        keys: true,
        entries: &[
            ("Keyboard shortcuts", "Super K"),
            ("Command palette", "Ctrl K"),
            ("Search servers", "Ctrl Shift K"),
            ("Show or hide the sidebar", "Ctrl B"),
            ("Preferences", "Ctrl ,"),
            ("Quit", "Ctrl Q"),
        ],
    },
    Group {
        title: "Sessions",
        keys: true,
        entries: &[
            ("New server", "Ctrl N"),
            ("Reconnect the current tab", "Ctrl R"),
            ("Close the current tab", "Ctrl W"),
            ("Next tab", "Ctrl PgDn"),
            ("Previous tab", "Ctrl PgUp"),
            ("Jump to tab 1-9", "Alt 1 … 9"),
        ],
    },
    Group {
        title: "Terminal",
        keys: true,
        entries: &[
            ("Copy", "Ctrl Shift C · Ctrl Insert"),
            ("Paste", "Ctrl Shift V · Shift Insert"),
            ("Select all", "Ctrl Shift A"),
        ],
    },
    Group {
        title: "Mouse",
        keys: false,
        entries: &[
            ("Connect, or focus an open session", "Click a server"),
            ("Open an extra session", "Ctrl-click or middle-click"),
            ("Server actions", "Right-click"),
            ("Move a server into a folder", "Drag onto the folder"),
            ("Expand a Monitor card", "Click the card"),
        ],
    },
];

pub fn present(parent: &impl IsA<gtk::Widget>) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.set_margin_top(12);
    page.set_margin_bottom(18);
    page.set_margin_start(18);
    page.set_margin_end(18);

    for section in GROUPS {
        let group = adw::PreferencesGroup::new();
        group.set_title(section.title);

        for (what, keys) in section.entries {
            let row = adw::ActionRow::new();
            row.set_title(what);

            if section.keys {
                row.add_suffix(&keycaps(keys));
            } else {
                let label = gtk::Label::new(Some(keys));
                label.set_valign(gtk::Align::Center);
                label.add_css_class("row-meta");
                row.add_suffix(&label);
            }

            group.add(&row);
        }

        page.append(&group);
    }

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_propagate_natural_height(true);
    scroller.set_max_content_height(620);
    scroller.set_child(Some(&page));

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroller));

    let dialog = adw::Dialog::new();
    dialog.set_title("Keyboard Shortcuts");
    dialog.set_content_width(520);
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// Splits on spaces so each key gets its own cap, keeping the separators
/// between alternatives as plain text.
fn keycaps(keys: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.set_valign(gtk::Align::Center);

    for part in keys.split_whitespace() {
        let label = gtk::Label::new(Some(part));

        if part == "·" || part == "…" {
            label.add_css_class("dimmer");
        } else {
            label.add_css_class("keycap");
        }

        row.append(&label);
    }

    row
}
