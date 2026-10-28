//! VTE tabanli terminal sayfasi.
//! vte-2.91-gtk4 bulunmayan dagitimlarda `--no-default-features` ile devre disi.

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use vte::prelude::*;

/// Sarmalayicinin cikis kodunu bildirdigi baslik oneki.
const STATUS_PREFIX: &str = "runetty-status:";

const PALETTE: [&str; 16] = [
    "#0d0d12", "#f4766d", "#4ade80", "#e8a33d",
    "#4a7dff", "#b283f0", "#4fd6d2", "#9aa0ab",
    "#3a3f4b", "#ff9b94", "#86efac", "#f3c175",
    "#7fa2ff", "#cbaaf7", "#8ae8e4", "#f5f6f8",
];

/// `command` bos ise kullanicinin kabugu acilir.
/// `alias` verilirse parola keyring'den SSH_ASKPASS uzerinden beslenir.
/// `on_status` komut bitince cikis koduyla cagrilir; durum noktasi buna bakar.
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
        // Komut bitince kullaniciyi sessizce yerel kabuga dusurmek yerine
        // cikis kodunu goster ve sekmeyi acik tut.
        // Cikis kodu pencere basligi uzerinden geri geliyor: sarmalayici
        // `read` ile bekledigi icin surec olmuyor, yani child-exited yetmiyor.
        Some(cmd) => vec![
            "/bin/sh".into(),
            "-c".into(),
            format!(
                "{cmd}; status=$?; \
                 printf '\\033]0;{STATUS_PREFIX}%s\\007' \"$status\"; \
                 printf '\\n\\033[2m[runetty] cikis kodu %s — \
                 kapatmak icin Enter\\033[0m\\n' \"$status\"; read _",
                cmd = cmd
            ),
        ],
        None => vec![shell, "-l".into()],
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    // ssh parola sordugunda SSH_ASKPASS'i calistirir ve stdout'undan okur.
    // Kendi binary'mizi askpass olarak veriyoruz; keyring'den okuyup basiyor.
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
                eprintln!("runetty: terminal baslatilamadi: {error}");
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

/// Acik sekmelerin fontunu yerinde degistirir.
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
