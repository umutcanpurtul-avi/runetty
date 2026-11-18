//! The command palette: one field that reaches every server and every action.
//!
//! Opens on recent servers rather than an empty list, so Ctrl+K followed by
//! Enter reconnects to the last host without typing anything.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::data::palette::{self, Entry};
use crate::data::store::Store;

struct Palette {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    /// Everything on offer.
    entries: RefCell<Vec<Entry>>,
    /// What the current query left on screen, in the order shown, so a row
    /// index maps straight back to an entry.
    visible: RefCell<Vec<Entry>>,
    parent: gtk::Widget,
}

impl Palette {
    fn show(self: &Rc<Self>, query: &str) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }

        let found = palette::search(&self.entries.borrow(), query);

        if found.is_empty() {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            row.set_child(Some(&note("No matches")));
            self.list.append(&row);
            return;
        }

        for entry in &found {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&entry_row(entry)));
            self.list.append(&row);
        }

        // Enter runs the top hit, so it has to start selected.
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
        }

        self.visible.replace(found);
    }

    fn activate(&self, index: i32) {
        let visible = self.visible.borrow();
        let Some(entry) = visible.get(index as usize) else { return };

        let action = entry.action.clone();
        let target = entry.target.clone();
        self.dialog.close();

        // After the dialog is gone, so a dialog opened by the action has a
        // clear parent and does not fight the palette for focus.
        let parent = self.parent.clone();
        glib::idle_add_local_once(move || {
            let target = target.as_ref().map(|value| value.to_variant());
            let _ = gtk::prelude::WidgetExt::activate_action(
                &parent,
                &action,
                target.as_ref(),
            );
        });
    }
}

pub fn present(parent: &impl IsA<gtk::Widget>, store: &Rc<RefCell<Store>>) {
    let mut entries: Vec<Entry> = store
        .borrow()
        .hosts()
        .into_iter()
        .map(|host| Entry {
            title: host.alias.clone(),
            detail: host.user_host(),
            kind: palette::Kind::Server,
            action: "win.host-connect".into(),
            target: Some(host.alias),
            last_used: host.last_used,
        })
        .collect();
    entries.extend(palette::commands());

    let search = gtk::SearchEntry::new();
    search.add_css_class("command-field");
    search.set_placeholder_text(Some("Connect to a server, or run a command"));

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("palette-list");

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_min_content_height(260);
    scroller.set_max_content_height(420);
    scroller.set_propagate_natural_height(true);
    scroller.set_child(Some(&list));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.add_css_class("palette");
    body.append(&search);
    body.append(&scroller);

    let dialog = adw::Dialog::new();
    dialog.set_content_width(560);
    dialog.set_child(Some(&body));
    dialog.set_presentation_mode(adw::DialogPresentationMode::Floating);

    let palette = Rc::new(Palette {
        dialog: dialog.clone(),
        list: list.clone(),
        entries: RefCell::new(entries),
        visible: RefCell::new(Vec::new()),
        parent: parent.clone().upcast(),
    });

    palette.show("");

    {
        let palette = Rc::clone(&palette);
        search.connect_search_changed(move |entry| palette.show(&entry.text()));
    }

    {
        let palette = Rc::clone(&palette);
        let list = list.clone();
        search.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                palette.activate(row.index());
            }
        });
    }

    {
        let palette = Rc::clone(&palette);
        list.connect_row_activated(move |_, row| palette.activate(row.index()));
    }

    // Arrow keys move the selection while the field keeps the text cursor.
    let keys = gtk::EventControllerKey::new();
    {
        let list = list.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Down => {
                step(&list, 1);
                glib::Propagation::Stop
            }
            gdk::Key::Up => {
                step(&list, -1);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    search.add_controller(keys);

    dialog.present(Some(parent));
    search.grab_focus();
}

fn step(list: &gtk::ListBox, delta: i32) {
    let current = list.selected_row().map(|row| row.index()).unwrap_or(0);
    let next = current + delta;

    if next < 0 {
        return;
    }

    if let Some(row) = list.row_at_index(next) {
        list.select_row(Some(&row));
        row.grab_focus();
    }
}

fn entry_row(entry: &Entry) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("session-row");

    let stack = gtk::Box::new(gtk::Orientation::Vertical, 1);
    stack.set_hexpand(true);
    stack.set_valign(gtk::Align::Center);

    let title = gtk::Label::new(Some(&entry.title));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("row-title");
    stack.append(&title);

    if !entry.detail.is_empty() {
        let detail = gtk::Label::new(Some(&entry.detail));
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        detail.add_css_class("row-subtitle");
        if entry.kind == palette::Kind::Server {
            detail.add_css_class("mono");
        }
        stack.append(&detail);
    }

    row.append(&stack);

    let badge = gtk::Label::new(Some(entry.kind.label()));
    badge.add_css_class("badge");
    badge.set_valign(gtk::Align::Center);
    row.append(&badge);

    row
}

fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_margin_start(12);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.add_css_class("row-meta");
    label
}
