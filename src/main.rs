mod capture;
mod config;
mod data;
mod ui;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

use config::{APP_ID, MIN_ADW, MIN_GTK, RESOURCE_PATH, VERSION};

fn main() -> glib::ExitCode {
    // ssh'in SSH_ASKPASS'i olarak cagrildik: parolayi keyring'den okuyup bas.
    // GTK baslatmadan once, cunku burada pencere yok.
    if std::env::args().any(|arg| arg == "--askpass")
        || std::env::var_os("RUNETTY_ASKPASS_ALIAS").is_some()
    {
        return askpass();
    }

    // Surum sorgulari ve kontrolu GTK baslatildiktan sonra guvenli.
    if let Err(error) = gtk::init() {
        eprintln!("runetty: GTK baslatilamadi: {error}");
        return glib::ExitCode::FAILURE;
    }

    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("runetty {VERSION}");
        println!(
            "gtk {}.{}.{} · libadwaita {}.{}.{}",
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version(),
            adw::major_version(),
            adw::minor_version(),
            adw::micro_version(),
        );
        return glib::ExitCode::SUCCESS;
    }

    if let Err(msg) = check_runtime_versions() {
        eprintln!("runetty: {msg}");
        eprintln!("Cozum: dagitiminiz icin Flatpak surumunu kullanin (bkz. BUILDING.md).");
        return glib::ExitCode::FAILURE;
    }

    gio::resources_register_include!("runetty.gresource")
        .expect("gomulu kaynaklar yuklenemedi");

    let screenshot = argument_value("--screenshot");
    let exec = argument_value("--exec");
    let autoconnect = argument_value("--connect");
    let context_menu = argument_value("--context-menu");
    let edit = argument_value("--edit");

    // Gelistirme bayraklari acik bir pencere varken de ise yaramali;
    // tek-instance davranisi bunlari calisan surece devredip yutuyor.
    let flags = if screenshot.is_some()
        || exec.is_some()
        || autoconnect.is_some()
        || context_menu.is_some()
        || edit.is_some()
    {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .resource_base_path(RESOURCE_PATH)
        .flags(flags)
        .build();

    app.connect_startup(|app| {
        load_stylesheet();
        register_icons();
        gtk::Window::set_default_icon_name(APP_ID);
        setup_accels(app);
    });

    let mode = argument_value("--mode")
        .and_then(|value| ui::icon_rail::Mode::parse(&value))
        .unwrap_or(ui::icon_rail::Mode::Sessions);
    app.connect_activate(move |app| {
        let window = window::build(
                app,
                mode,
                exec.as_deref(),
                autoconnect.as_deref(),
                context_menu.as_deref(),
                edit.as_deref(),
            );
        window.present();
        if let Some(path) = screenshot.clone() {
            capture::schedule(&window, path);
        }
    });

    // Argumanlari kendimiz okuyoruz; GApplication'a bilinmeyen bayrak gitmesin.
    app.run_with_args::<&str>(&[])
}

/// Dinamik bagladigimiz icin surum uyusmazligi calistirma aninda ortaya cikar.
/// Anlasilir bir mesajla cikmak, cozulmemis sembol hatasindan iyidir.
fn check_runtime_versions() -> Result<(), String> {
    let gtk = (gtk::major_version(), gtk::minor_version());
    if gtk < MIN_GTK {
        return Err(format!(
            "GTK {}.{} gerekiyor, sistemde {}.{} var.",
            MIN_GTK.0, MIN_GTK.1, gtk.0, gtk.1
        ));
    }

    let adw = (adw::major_version(), adw::minor_version());
    if adw < MIN_ADW {
        return Err(format!(
            "libadwaita {}.{} gerekiyor, sistemde {}.{} var.",
            MIN_ADW.0, MIN_ADW.1, adw.0, adw.1
        ));
    }

    Ok(())
}

/// SSH_ASKPASS protokolu: parolayi stdout'a bas, baska hicbir sey yazma.
fn askpass() -> glib::ExitCode {
    let Ok(alias) = std::env::var("RUNETTY_ASKPASS_ALIAS") else {
        return glib::ExitCode::FAILURE;
    };

    match data::secrets::lookup(&alias) {
        Some(password) => {
            println!("{password}");
            glib::ExitCode::SUCCESS
        }
        None => glib::ExitCode::FAILURE,
    }
}

/// `--bayrak deger` seklindeki argumani okur.
fn argument_value(flag: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

fn load_stylesheet() {
    let provider = gtk::CssProvider::new();
    provider.load_from_resource(&format!("{RESOURCE_PATH}/style.css"));

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("ekran bulunamadi"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Kendi symbolic ikonlarimiz gresource'tan gelir; Adwaita seti yedek kalir.
fn register_icons() {
    let display = gtk::gdk::Display::default().expect("ekran bulunamadi");
    gtk::IconTheme::for_display(&display)
        .add_resource_path(&format!("{RESOURCE_PATH}/icons"));
}

fn setup_accels(app: &adw::Application) {
    app.set_accels_for_action("win.global-search", &["<Shift><Control>k"]);
    app.set_accels_for_action("win.command-palette", &["<Control>k"]);
    app.set_accels_for_action("win.toggle-sidebar", &["<Control>b"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
}
