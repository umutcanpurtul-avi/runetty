mod askpass;
mod capture;
mod config;
mod data;
mod ui;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

use config::{APP_ID, MIN_ADW, MIN_GTK, RESOURCE_PATH, VERSION};

fn main() -> glib::ExitCode {
    // First: the askpass window uses the stylesheet too.
    gio::resources_register_include!("runetty.gresource")
        .expect("could not load embedded resources");

    // Invoked as ssh's SSH_ASKPASS: read the password from the keyring and
    // print it. Before GTK starts, since there is no window here.
    if std::env::args().any(|arg| arg == "--askpass")
        || std::env::var_os("RUNETTY_ASKPASS_ALIAS").is_some()
    {
        return askpass::run();
    }

    // Version queries and checks are safe once GTK is initialised.
    if let Err(error) = gtk::init() {
        eprintln!("runetty: could not initialise GTK: {error}");
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
        eprintln!("Fix: use the Flatpak build for your distribution (see BUILDING.md).");
        return glib::ExitCode::FAILURE;
    }

    let screenshot = argument_value("--screenshot");
    let exec = argument_value("--exec");
    let autoconnect = argument_value("--connect");
    let context_menu = argument_value("--context-menu");
    let edit = argument_value("--edit");
    let action = argument_value("--action");
    let demo_metrics = std::env::args().any(|arg| arg == "--demo-metrics");
    let expand = argument_value("--expand");

    // Development flags must work even with a window already open; the
    // single-instance behaviour would hand them to the running process.
    let flags = if screenshot.is_some()
        || exec.is_some()
        || autoconnect.is_some()
        || context_menu.is_some()
        || edit.is_some()
        || action.is_some()
        || demo_metrics
        || expand.is_some()
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
                action.as_deref(),
                demo_metrics,
                expand.as_deref(),
            );
        window.present();
        if let Some(path) = screenshot.clone() {
            capture::schedule(&window, path);
        }
    });

    // Authenticated channels outlive their sessions by ControlPersist; once
    // runetty is gone nobody expects them to. A development instance runs
    // alongside the real one and shares its channels, so it leaves them be.
    if !flags.contains(gio::ApplicationFlags::NON_UNIQUE) {
        app.connect_shutdown(|_| {
            let channels: Vec<std::process::Child> = data::store::Store::load()
                .hosts()
                .iter()
                .filter_map(|host| host.release_channel("exit"))
                .collect();
            for mut channel in channels {
                let _ = channel.wait();
            }
        });
    }

    // We parse arguments ourselves; GApplication must not see unknown flags.
    app.run_with_args::<&str>(&[])
}

/// We link dynamically, so a version mismatch surfaces at run time.
/// Exiting with a clear message beats an unresolved-symbol error.
fn check_runtime_versions() -> Result<(), String> {
    let gtk = (gtk::major_version(), gtk::minor_version());
    if gtk < MIN_GTK {
        return Err(format!(
            "GTK {}.{} is required, the system has {}.{}.",
            MIN_GTK.0, MIN_GTK.1, gtk.0, gtk.1
        ));
    }

    let adw = (adw::major_version(), adw::minor_version());
    if adw < MIN_ADW {
        return Err(format!(
            "libadwaita {}.{} is required, the system has {}.{}.",
            MIN_ADW.0, MIN_ADW.1, adw.0, adw.1
        ));
    }

    Ok(())
}


/// Reads an argument of the form `--flag value`.
fn argument_value(flag: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

thread_local! {
    /// Light palette. style.css holds the dark one; this provider is added
    /// only for the light theme and redefines the colours.
    static LIGHT_CSS: gtk::CssProvider = gtk::CssProvider::new();

    /// Font layer from the settings. It sits above the palette, so design
    /// tokens stay in the file while preferences can still override them.
    static USER_CSS: gtk::CssProvider = gtk::CssProvider::new();
}

/// Adds or removes the light palette. The caller watches libadwaita's `dark`
/// state; this only manages the provider.
pub fn apply_palette(dark: bool) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };

    LIGHT_CSS.with(|provider| {
        if dark {
            gtk::style_context_remove_provider_for_display(&display, provider);
            return;
        }

        provider.load_from_resource(&format!("{RESOURCE_PATH}/light.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    });
}

pub fn apply_user_css(css: &str) {
    USER_CSS.with(|provider| {
        provider.load_from_string(css);

        if let Some(display) = gtk::gdk::Display::default() {
            // Above the palette: the font preference holds in both themes.
            gtk::style_context_add_provider_for_display(
                &display,
                provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
            );
        }
    });
}

pub fn load_stylesheet() {
    let provider = gtk::CssProvider::new();
    provider.load_from_resource(&format!("{RESOURCE_PATH}/style.css"));

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("ekran bulunamadi"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Our own symbolic icons come from gresource; the Adwaita set is the fallback.
fn register_icons() {
    let display = gtk::gdk::Display::default().expect("ekran bulunamadi");
    gtk::IconTheme::for_display(&display)
        .add_resource_path(&format!("{RESOURCE_PATH}/icons"));
}

fn setup_accels(app: &adw::Application) {
    // Super+K is what was asked for; Ctrl+? and F1 are the conventional
    // fallbacks, and some desktops reserve Super for themselves.
    app.set_accels_for_action("win.shortcuts", &["<Super>k", "<Control>question", "F1"]);

    app.set_accels_for_action("win.global-search", &["<Shift><Control>k"]);
    app.set_accels_for_action("win.command-palette", &["<Control>k"]);
    app.set_accels_for_action("win.toggle-sidebar", &["<Control>b"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.host-new", &["<Control>n"]);
    app.set_accels_for_action("win.reconnect", &["<Control>r"]);

    // Ctrl+W closed the window before, not the tab, despite what the hint said.
    app.set_accels_for_action("win.close-tab", &["<Control>w"]);
    app.set_accels_for_action("win.next-tab", &["<Control>Page_Down"]);
    app.set_accels_for_action("win.previous-tab", &["<Control>Page_Up"]);

    for index in 1..=9 {
        app.set_accels_for_action(
            &format!("win.select-tab({})", index - 1),
            &[&format!("<Alt>{index}")],
        );
    }

    app.set_accels_for_action("app.quit", &["<Control>q"]);
}
