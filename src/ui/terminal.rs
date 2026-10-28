//! VTE-based terminal page.
//! Disabled with `--no-default-features` on distributions without vte-2.91-gtk4.

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use vte::prelude::*;

/// Title prefix the wrapper uses to report the exit status.
const STATUS_PREFIX: &str = "runetty-status:";

const PALETTE: [&str; 16] = [
    "#0d0d12", "#f4766d", "#4ade80", "#e8a33d",
    "#4a7dff", "#b283f0", "#4fd6d2", "#9aa0ab",
    "#3a3f4b", "#ff9b94", "#86efac", "#f3c175",
    "#7fa2ff", "#cbaaf7", "#8ae8e4", "#f5f6f8",
];

/// With no `command` the user's shell opens.
/// Given an `alias`, the password is fed from the keyring via SSH_ASKPASS.
/// `on_status` fires with the exit code when the command ends; the status dot
/// reads from it.
pub fn page<F>(
    command: Option<&str>,
    alias: Option<&str>,
    font: &str,
    on_status: F,
) -> gtk::Widget
where
    F: Fn(i32) + 'static,
{
    use std::rc::Rc;

    let on_status = Rc::new(on_status);
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
        // command ends, show the exit code and keep the tab open.
        // The exit code comes back through the window title: the wrapper
        // blocks on `read`, so the process never dies and child-exited alone
        // is not enough.
        Some(cmd) => vec![
            "/bin/sh".into(),
            "-c".into(),
            format!(
                "{cmd}; status=$?; \
                 printf '\\033]0;{STATUS_PREFIX}%s\\007' \"$status\"; \
                 printf '\\n\\033[2m[runetty] exit code %s — \
                 press Enter to close\\033[0m\\n' \"$status\"; read _",
                cmd = cmd
            ),
        ],
        None => vec![shell, "-l".into()],
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    // When ssh needs a password it runs SSH_ASKPASS and reads its stdout.
    // We point that at our own binary, which prints what the keyring holds.
    let env: Vec<String> = match alias {
        Some(alias) if crate::data::secrets::has(alias) => match std::env::current_exe() {
            Ok(exe) => vec![
                format!("SSH_ASKPASS={}", exe.display()),
                "SSH_ASKPASS_REQUIRE=force".to_string(),
                format!("RUNETTY_ASKPASS_ALIAS={alias}"),
            ],
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    };
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

    {
        let on_status = Rc::clone(&on_status);
        term.connect_window_title_changed(move |term| {
            let Some(title) = term.window_title() else { return };
            let Some(rest) = title.strip_prefix(STATUS_PREFIX) else { return };
            if let Ok(status) = rest.trim().parse::<i32>() {
                on_status(status);
            }
        });
    }

    term.connect_child_exited(move |_, status| on_status(status));

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_child(Some(&term));
    scroller.upcast()
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
