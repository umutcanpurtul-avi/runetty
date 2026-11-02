//! Empty state: centred logo, wide command field, "Jump back in" chips and
//! a keyboard hint row underneath.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::data::store::Store;

pub struct EmptyState {
    pub widget: gtk::Box,
    chips_label: gtk::Label,
    chips: gtk::Box,
    store: Rc<RefCell<Store>>,
    connect: Rc<dyn Fn(&str)>,
}

impl EmptyState {
    /// Chips follow the recents list.
    pub fn refresh(&self) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }

        let recents = self.store.borrow().recents(3);
        let visible = !recents.is_empty();
        self.chips_label.set_visible(visible);
        self.chips.set_visible(visible);

        for host in recents {
            let chip = gtk::Button::new();
            chip.add_css_class("chip");

            let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            inner.append(&gtk::Label::new(Some(&host.alias)));

            let when = gtk::Label::new(Some(&crate::data::relative(host.last_used)));
            when.add_css_class("dimmer");
            inner.append(&when);
            chip.set_child(Some(&inner));

            let connect = Rc::clone(&self.connect);
            let alias = host.alias.clone();
            chip.connect_clicked(move |_| connect(&alias));

            self.chips.append(&chip);
        }
    }
}

pub fn build(
    store: Rc<RefCell<Store>>,
    connect: Rc<dyn Fn(&str)>,
    run: Rc<dyn Fn(&str)>,
) -> Rc<EmptyState> {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    widget.set_valign(gtk::Align::Center);
    widget.set_halign(gtk::Align::Center);
    widget.set_margin_bottom(48);

    let icon = gtk::Image::from_icon_name("dev.runetty.Runetty");
    icon.set_pixel_size(48);
    icon.set_margin_bottom(14);
    widget.append(&icon);

    let title = gtk::Label::new(Some("Connect to a server"));
    title.add_css_class("empty-title");
    title.set_margin_bottom(18);
    widget.append(&title);

    let entry = gtk::Entry::new();
    entry.add_css_class("command-field");
    entry.set_placeholder_text(Some("ssh user@host  or type a command"));
    entry.set_margin_bottom(20);
    {
        let run = Rc::clone(&run);
        entry.connect_activate(move |entry| {
            let text = entry.text();
            let text = text.trim();
            if !text.is_empty() {
                run(text);
                entry.set_text("");
            }
        });
    }
    widget.append(&entry);

    let chips_label = gtk::Label::new(Some("Jump back in"));
    chips_label.add_css_class("section-header");
    chips_label.set_halign(gtk::Align::Center);
    widget.append(&chips_label);

    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    chips.set_halign(gtk::Align::Center);
    chips.set_margin_bottom(24);
    widget.append(&chips);

    widget.append(&hint_row());

    let state = Rc::new(EmptyState { widget, chips_label, chips, store, connect });
    state.refresh();
    state
}

fn hint_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::Center);
    row.add_css_class("hint-row");

    // Super K first: it is the way to find everything else.
    let hints: &[(&str, &str)] = &[
        ("Super K", "all shortcuts"),
        ("Ctrl K", "command palette"),
        ("Ctrl B", "sidebar"),
    ];

    for (index, (keys, what)) in hints.iter().enumerate() {
        if index > 0 {
            let separator = gtk::Label::new(Some("·"));
            separator.add_css_class("dimmer");
            row.append(&separator);
        }

        let cap = gtk::Label::new(Some(keys));
        cap.add_css_class("keycap");
        row.append(&cap);

        row.append(&gtk::Label::new(Some(what)));
    }

    row
}
