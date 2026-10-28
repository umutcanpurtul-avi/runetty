//! VTE tabanli terminal sayfasi.
//! vte-2.91-gtk4 bulunmayan dagitimlarda `--no-default-features` ile devre disi.

use gtk::prelude::*;
use vte::prelude::*;
use gtk::{gdk, gio, glib, pango};

const PALETTE: [&str; 16] = [
    "#0d0d12", "#f4766d", "#4ade80", "#e8a33d",
    "#4a7dff", "#b283f0", "#4fd6d2", "#9aa0ab",
    "#3a3f4b", "#ff9b94", "#86efac", "#f3c175",
    "#7fa2ff", "#cbaaf7", "#8ae8e4", "#f5f6f8",
];

/// `command` bos ise kullanicinin kabugu acilir.
pub fn page(command: Option<&str>) -> gtk::Widget {
    let term = vte::Terminal::new();
    term.add_css_class("terminal-page");
    term.set_hexpand(true);
    term.set_vexpand(true);
    term.set_scrollback_lines(10_000);
    term.set_font(Some(&pango::FontDescription::from_string(
        "JetBrains Mono 11",
    )));

    apply_colors(&term);

    let shell = glib::getenv("SHELL")
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "/bin/bash".to_string());

    let argv: Vec<String> = match command {
        Some(cmd) => vec![shell, "-lc".into(), format!("{cmd}; exec $SHELL")],
        None => vec![shell, "-l".into()],
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let home = glib::home_dir();

    term.spawn_async(
        vte::PtyFlags::DEFAULT,
        home.to_str(),
        &argv,
        &[],
        glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(error) = result {
                eprintln!("runetty: terminal baslatilamadi: {error}");
            }
        },
    );

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_child(Some(&term));
    scroller.upcast()
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
