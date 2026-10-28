//! Ayar diyalogu. Su an yalnizca gorunum; baglanti alanlari sunucu basina
//! tanimli oldugu icin orada degil, kayit diyalogunda duruyor.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::pango;

use crate::data::settings::Settings;

pub fn present<F>(parent: &impl IsA<gtk::Widget>, settings: Rc<RefCell<Settings>>, apply: F)
where
    F: Fn() + 'static,
{
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");

    let page = adw::PreferencesPage::new();
    page.set_title("Appearance");
    page.set_icon_name(Some("preferences-desktop-font-symbolic"));

    let fonts = adw::PreferencesGroup::new();
    fonts.set_title("Fonts");
    fonts.set_description(Some(
        "Degisiklik aninda uygulanir ve acik terminallere de yansir.",
    ));

    let apply = Rc::new(apply);

    let interface = font_row(
        "Interface",
        &settings.borrow().appearance.interface_pango(),
        {
            let settings = Rc::clone(&settings);
            let apply = Rc::clone(&apply);
            move |font| {
                {
                    let mut settings = settings.borrow_mut();
                    settings.appearance.interface_font = family(&font);
                    settings.appearance.interface_size = size(&font);
                    settings.save();
                }
                apply();
            }
        },
    );

    let terminal = font_row(
        "Terminal",
        &settings.borrow().appearance.terminal_pango(),
        {
            let settings = Rc::clone(&settings);
            let apply = Rc::clone(&apply);
            move |font| {
                {
                    let mut settings = settings.borrow_mut();
                    settings.appearance.terminal_font = family(&font);
                    settings.appearance.terminal_size = size(&font);
                    settings.save();
                }
                apply();
            }
        },
    );

    fonts.add(&interface);
    fonts.add(&terminal);

    let reset = adw::PreferencesGroup::new();
    let button = gtk::Button::with_label("Varsayilanlara don");
    button.add_css_class("pill");
    button.set_halign(gtk::Align::Center);
    {
        let settings = Rc::clone(&settings);
        let apply = Rc::clone(&apply);
        let interface = interface.clone();
        let terminal = terminal.clone();
        button.connect_clicked(move |_| {
            {
                let mut settings = settings.borrow_mut();
                settings.appearance = Default::default();
                settings.save();
            }

            let current = settings.borrow().appearance.clone();
            set_row_font(&interface, &current.interface_pango());
            set_row_font(&terminal, &current.terminal_pango());
            apply();
        });
    }
    reset.add(&button);

    page.add(&fonts);
    page.add(&reset);
    dialog.add(&page);
    dialog.present(Some(parent));
}

fn font_row<F>(title: &str, initial: &str, on_change: F) -> adw::ActionRow
where
    F: Fn(pango::FontDescription) + 'static,
{
    let row = adw::ActionRow::new();
    row.set_title(title);

    let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    button.set_valign(gtk::Align::Center);
    button.set_font_desc(&pango::FontDescription::from_string(initial));
    button.connect_font_desc_notify(move |button| {
        if let Some(font) = button.font_desc() {
            on_change(font);
        }
    });

    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    row
}

fn set_row_font(row: &adw::ActionRow, font: &str) {
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(button) = find_button(&widget) {
            button.set_font_desc(&pango::FontDescription::from_string(font));
            return;
        }
        child = widget.next_sibling();
    }
}

/// ActionRow suffix'i birkac kutu derinde kalabiliyor.
fn find_button(widget: &gtk::Widget) -> Option<gtk::FontDialogButton> {
    if let Ok(button) = widget.clone().downcast::<gtk::FontDialogButton>() {
        return Some(button);
    }

    let mut child = widget.first_child();
    while let Some(candidate) = child {
        if let Some(found) = find_button(&candidate) {
            return Some(found);
        }
        child = candidate.next_sibling();
    }

    None
}

fn family(font: &pango::FontDescription) -> String {
    font.family().map(|f| f.to_string()).unwrap_or_default()
}

fn size(font: &pango::FontDescription) -> f64 {
    let size = font.size();
    if size <= 0 {
        return 13.0;
    }
    f64::from(size) / f64::from(pango::SCALE)
}
