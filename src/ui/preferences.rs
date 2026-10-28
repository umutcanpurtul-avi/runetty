//! Ayar diyalogu. Su an yalnizca gorunum; baglanti alanlari sunucu basina
//! tanimli oldugu icin orada degil, kayit diyalogunda duruyor.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::pango;

use crate::data::settings::{Settings, Theme};

pub fn present<F>(parent: &impl IsA<gtk::Widget>, settings: Rc<RefCell<Settings>>, apply: F)
where
    F: Fn() + 'static,
{
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");

    let page = adw::PreferencesPage::new();
    page.set_title("Appearance");
    page.set_icon_name(Some("preferences-desktop-font-symbolic"));

    let apply_early = Rc::new(apply);
    let apply = Rc::clone(&apply_early);

    let appearance = adw::PreferencesGroup::new();
    appearance.set_title("Tema");

    let theme_row = adw::ComboRow::new();
    theme_row.set_title("Renk semasi");
    theme_row.set_subtitle("Sistem secenegi masaustunun ayarini takip eder");

    let names: Vec<&str> = Theme::all().iter().map(|theme| theme.label()).collect();
    let model = gtk::StringList::new(&names);
    theme_row.set_model(Some(&model));
    theme_row.set_selected(
        Theme::all()
            .iter()
            .position(|theme| *theme == settings.borrow().appearance.theme)
            .unwrap_or(0) as u32,
    );

    {
        let settings = Rc::clone(&settings);
        theme_row.connect_selected_notify(move |row| {
            let Some(theme) = Theme::all().get(row.selected() as usize).copied() else {
                return;
            };

            let mut settings = settings.borrow_mut();
            if settings.appearance.theme == theme {
                return;
            }
            settings.appearance.theme = theme;
            settings.save();
            drop(settings);

            crate::window::apply_theme(theme);
        });
    }

    appearance.add(&theme_row);

    let fonts = adw::PreferencesGroup::new();
    fonts.set_title("Fonts");
    fonts.set_description(Some(
        "Degisiklik aninda uygulanir ve acik terminallere de yansir.",
    ));

    // Aile ve boyut ayri kontroller: font diyalogu yalnizca aile secimi
    // icin aciliyordu ve "sadece boyutu degistir" diye bir yol yoktu.
    let interface_family = family_row("Arayuz yazi tipi", &settings.borrow().appearance.interface_font, {
        let settings = Rc::clone(&settings);
        let apply = Rc::clone(&apply);
        move |name| {
            {
                let mut settings = settings.borrow_mut();
                settings.appearance.interface_font = name;
                settings.save();
            }
            apply();
        }
    });

    let interface_size = size_row("Arayuz boyutu", settings.borrow().appearance.interface_size, {
        let settings = Rc::clone(&settings);
        let apply = Rc::clone(&apply);
        move |value| {
            {
                let mut settings = settings.borrow_mut();
                settings.appearance.interface_size = value;
                settings.save();
            }
            apply();
        }
    });

    let terminal_family = family_row("Terminal yazi tipi", &settings.borrow().appearance.terminal_font, {
        let settings = Rc::clone(&settings);
        let apply = Rc::clone(&apply);
        move |name| {
            {
                let mut settings = settings.borrow_mut();
                settings.appearance.terminal_font = name;
                settings.save();
            }
            apply();
        }
    });

    let terminal_size = size_row("Terminal boyutu", settings.borrow().appearance.terminal_size, {
        let settings = Rc::clone(&settings);
        let apply = Rc::clone(&apply);
        move |value| {
            {
                let mut settings = settings.borrow_mut();
                settings.appearance.terminal_size = value;
                settings.save();
            }
            apply();
        }
    });

    fonts.add(&interface_family);
    fonts.add(&interface_size);
    fonts.add(&terminal_family);
    fonts.add(&terminal_size);

    let reset = adw::PreferencesGroup::new();
    let button = gtk::Button::with_label("Varsayilanlara don");
    button.add_css_class("pill");
    button.set_halign(gtk::Align::Center);
    {
        let settings = Rc::clone(&settings);
        let apply = Rc::clone(&apply);
        let interface_family = interface_family.clone();
        let interface_size = interface_size.clone();
        let terminal_family = terminal_family.clone();
        let terminal_size = terminal_size.clone();
        button.connect_clicked(move |_| {
            let theme = settings.borrow().appearance.theme;
            {
                let mut settings = settings.borrow_mut();
                settings.appearance = Default::default();
                // Tema ayri bir tercih; font sifirlamasi onu bozmasin.
                settings.appearance.theme = theme;
                settings.save();
            }

            let current = settings.borrow().appearance.clone();
            set_family(&interface_family, &current.interface_font);
            interface_size.set_value(current.interface_size);
            set_family(&terminal_family, &current.terminal_font);
            terminal_size.set_value(current.terminal_size);
            apply();
        });
    }
    reset.add(&button);

    page.add(&appearance);
    page.add(&fonts);
    page.add(&reset);
    dialog.add(&page);
    dialog.present(Some(parent));
}

fn family_row<F>(title: &str, initial: &str, on_change: F) -> adw::ActionRow
where
    F: Fn(String) + 'static,
{
    let row = adw::ActionRow::new();
    row.set_title(title);

    let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    button.set_valign(gtk::Align::Center);
    // Once tanim, sonra seviye: ters sirada dugme "None" gosteriyordu.
    button.set_font_desc(&family_desc(initial));
    button.set_level(gtk::FontLevel::Family);
    button.set_use_size(false);

    button.connect_font_desc_notify(move |button| {
        let Some(font) = button.font_desc() else { return };
        let Some(name) = font.family() else { return };
        on_change(name.to_string());
    });

    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    row
}

fn size_row<F>(title: &str, initial: f64, on_change: F) -> adw::SpinRow
where
    F: Fn(f64) + 'static,
{
    let row = adw::SpinRow::with_range(7.0, 28.0, 1.0);
    row.set_title(title);
    row.set_value(initial);
    row.connect_value_notify(move |row| on_change(row.value()));
    row
}

/// Aile adindan gecerli bir tanim: bos ise temanin varsayilani.
fn family_desc(name: &str) -> pango::FontDescription {
    let name = name.trim();
    let text = if name.is_empty() { "Sans 12".to_string() } else { format!("{name} 12") };
    pango::FontDescription::from_string(&text)
}

fn set_family(row: &adw::ActionRow, name: &str) {
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(button) = find_button(&widget) {
            button.set_font_desc(&family_desc(name));
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



