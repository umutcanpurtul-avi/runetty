//! VTE-based terminal page.
//! Disabled with `--no-default-features` on distributions without vte-2.91-gtk4.

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use vte::prelude::*;

const PALETTE: [&str; 16] = [
    "#0d0d12", "#f4766d", "#4ade80", "#e8a33d",
    "#4a7dff", "#b283f0", "#4fd6d2", "#9aa0ab",
    "#3a3f4b", "#ff9b94", "#86efac", "#f3c175",
    "#7fa2ff", "#cbaaf7", "#8ae8e4", "#f5f6f8",
];

/// With no `command` the user's shell opens.
/// `environment` is added to the child's, normally from
/// `data::askpass_environment` so the keyring can answer ssh's prompts.
/// `on_status` fires with the exit code when the command ends; the status dot
/// reads from it.
pub fn page<F>(
    command: Option<&str>,
    environment: &[(String, String)],
    font: &str,
    on_status: F,
) -> gtk::Widget
where
    F: Fn(i32) + 'static,
{
    let term = vte::Terminal::new();
    term.add_css_class("terminal-page");
    term.set_hexpand(true);
    term.set_vexpand(true);
    term.set_scrollback_lines(10_000);
    term.set_font(Some(&pango::FontDescription::from_string(font)));

    apply_colors(&term);

    let shell = glib::getenv("SHELL")
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "/bin/bash".to_string());

    let argv: Vec<String> = match command {
        // Rather than silently dropping the user into a local shell when the
        // command ends, say how it ended and exit with the same status.
        //
        // The status reaches the dot through child-exited, the wrapper's own
        // exit. It used to travel through the terminal title, but anything
        // printed in the session can set the title, so the far side could
        // turn a red dot green. The tab stays open after the exit, with its
        // output readable, until it is closed.
        Some(cmd) => vec![
            "/bin/sh".into(),
            "-c".into(),
            format!(
                "{cmd}; status=$?; \
                 printf '\\n\\033[2m[runetty] exit code %s\\033[0m\\n' \"$status\"; \
                 exit \"$status\"",
                cmd = cmd
            ),
        ],
        None => vec![shell, "-l".into()],
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let env: Vec<String> = environment
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let env: Vec<&str> = env.iter().map(String::as_str).collect();

    let home = glib::home_dir();

    term.spawn_async(
        vte::PtyFlags::DEFAULT,
        home.to_str(),
        &argv,
        &env,
        glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(error) = result {
                eprintln!("runetty: could not start terminal: {error}");
            }
        },
    );

    term.connect_child_exited(move |_, status| on_status(status));

    attach_clipboard(&term);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_child(Some(&term));
    scroller.upcast()
}

/// Copy and paste inside the terminal.
///
/// Plain Ctrl+C and Ctrl+V belong to the shell: Ctrl+C interrupts, Ctrl+V
/// quotes the next character. So the clipboard uses the shifted pair every
/// terminal emulator uses, plus the Insert variants some people reach for,
/// plus a context menu for anyone who reaches for neither.
fn attach_clipboard(term: &vte::Terminal) {
    let keys = gtk::EventControllerKey::new();

    keys.connect_key_pressed(|controller, key, _, state| {
        let Some(term) = controller.widget().and_downcast::<vte::Terminal>() else {
            return glib::Propagation::Proceed;
        };

        let control = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);

        match key {
            gdk::Key::C | gdk::Key::c if control && shift => {
                copy(&term);
                glib::Propagation::Stop
            }
            gdk::Key::V | gdk::Key::v if control && shift => {
                term.paste_clipboard();
                glib::Propagation::Stop
            }
            gdk::Key::Insert if control => {
                copy(&term);
                glib::Propagation::Stop
            }
            gdk::Key::Insert if shift => {
                term.paste_clipboard();
                glib::Propagation::Stop
            }
            gdk::Key::A | gdk::Key::a if control && shift => {
                term.select_all();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });

    term.add_controller(keys);
    attach_context_menu(term);
}

/// Copying with nothing selected would clear the clipboard, which is never
/// what the keystroke meant.
fn copy(term: &vte::Terminal) {
    if term.has_selection() {
        term.copy_clipboard_format(vte::Format::Text);
    }
}

fn attach_context_menu(term: &vte::Terminal) {
    let menu = gio::Menu::new();
    menu.append(Some("Copy"), Some("term.copy"));
    menu.append(Some("Paste"), Some("term.paste"));
    menu.append(Some("Select All"), Some("term.select-all"));

    let actions = gio::SimpleActionGroup::new();

    let entries: [(&str, fn(&vte::Terminal)); 3] = [
        ("copy", copy),
        ("paste", |term| term.paste_clipboard()),
        ("select-all", |term| term.select_all()),
    ];

    for (name, action) in entries {
        let entry = gio::SimpleAction::new(name, None);
        let term = term.clone();
        entry.connect_activate(move |_, _| action(&term));
        actions.add_action(&entry);
    }

    term.insert_action_group("term", Some(&actions));

    // Built per click and unparented on close. A popover kept alive on the
    // terminal would outlive the tab it belongs to and be left orphaned when
    // the tab closes.
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    gesture.connect_pressed(move |gesture, _, x, y| {
        let Some(term) = gesture.widget() else { return };

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&term);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| popover.unparent());
        popover.popup();
    });
    term.add_controller(gesture);
}

/// Changes the font of open tabs in place.
pub fn restyle(root: &gtk::Widget, font: &str) {
    if let Ok(term) = root.clone().downcast::<vte::Terminal>() {
        term.set_font(Some(&pango::FontDescription::from_string(font)));
        return;
    }

    let mut child = root.first_child();
    while let Some(widget) = child {
        restyle(&widget, font);
        child = widget.next_sibling();
    }
}

fn apply_colors(term: &vte::Terminal) {
    let parse = |hex: &str| gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::BLACK);
    let palette: Vec<gdk::RGBA> = PALETTE.iter().map(|hex| parse(hex)).collect();
    let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();

    term.set_colors(
        Some(&parse("#f5f6f8")),
        Some(&parse("#0d0d12")),
        &palette_refs,
    );
}
