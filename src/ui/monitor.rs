//! Monitor sayfasi: sunucu basina bir kart, kartta uc halka.
//!
//! Olcum turu sunucu basina tek `ssh` cagrisi; kanal `ControlMaster` ile
//! acik kaliyor. Tur, sayfa gorunur degilken hic calismiyor — kapali bir
//! sekmenin arka planda ag ve CPU yakmasi icin bir sebep yok.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use super::ring::Ring;
use crate::data::metrics::{self, CpuTimes};
use crate::data::store::Store;
use crate::data::Host;

const INTERVAL: Duration = Duration::from_secs(2);

struct Card {
    /// Dis kutu: ustte ozet satiri, altinda acilir detay.
    widget: gtk::Box,
    detail_command: Vec<String>,
    detail: gtk::Revealer,
    detail_body: gtk::Box,
    expanded: Cell<bool>,
    detail_busy: Cell<bool>,
    alias: String,
    command: Vec<String>,
    rings: [Ring; 3],
    readings: [gtk::Label; 3],
    summary: gtk::Label,
    error: gtk::Box,
    error_text: gtk::Label,
    gauges: gtk::Box,
    previous: Cell<Option<CpuTimes>>,
    busy: Cell<bool>,
}

pub struct Monitor {
    pub widget: gtk::Widget,
    /// Sagda sabit panel olarak mi duruyor, sekmede mi?
    pinned: Cell<bool>,
    pin_button: gtk::Button,
    list: gtk::Box,
    empty: gtk::Label,
    store: Rc<RefCell<Store>>,
    cards: RefCell<HashMap<String, Rc<Card>>>,
    tick: RefCell<Option<glib::SourceId>>,
}

impl Monitor {
    /// Sayfa gorunur oldugunda tur baslar, gorunmez oldugunda durur.
    pub fn set_active(self: &Rc<Self>, active: bool) {
        if active {
            self.rebuild();
            if self.tick.borrow().is_some() {
                return;
            }

            self.poll();

            let this = Rc::clone(self);
            let id = glib::timeout_add_local(INTERVAL, move || {
                this.poll();
                glib::ControlFlow::Continue
            });
            *self.tick.borrow_mut() = Some(id);
        } else if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned.get()
    }

    pub fn set_pinned(&self, pinned: bool) {
        self.pinned.set(pinned);
        self.pin_button.set_icon_name(if pinned {
            "unpin-symbolic"
        } else {
            "pin-right-symbolic"
        });
        self.pin_button.set_tooltip_text(Some(if pinned {
            "Sekmeye geri al"
        } else {
            "Saga sabitle"
        }));

        if pinned {
            self.widget.add_css_class("pinned");
        } else {
            self.widget.remove_css_class("pinned");
        }
    }

    /// Dogrulama icin: bir kartin detayini programatik acar.
    pub fn expand(self: &Rc<Self>, alias: &str) {
        self.rebuild();
        let card = self.cards.borrow().get(alias).cloned();
        let Some(card) = card else { return };

        card.expanded.set(true);
        card.detail.set_reveal_child(true);
        card.widget.add_css_class("expanded");
        load_detail(&card);
    }

    /// Izleme listesi degisince disaridan cagrilir.
    pub fn rebuild_now(self: &Rc<Self>) {
        self.rebuild();
    }

    /// Kayit listesi degisince kartlari yeniden kurar.
    fn rebuild(self: &Rc<Self>) {
        let hosts = self.store.borrow().monitored();
        let aliases: Vec<String> = hosts.iter().map(|host| host.alias.clone()).collect();

        self.cards
            .borrow_mut()
            .retain(|alias, card| {
                let keep = aliases.contains(alias);
                if !keep {
                    self.list.remove(&card.widget);
                }
                keep
            });

        for host in &hosts {
            if self.cards.borrow().contains_key(&host.alias) {
                continue;
            }

            let card = Rc::new(build_card(host));
            attach_expander(&card);
            self.list.append(&card.widget);
            self.cards.borrow_mut().insert(host.alias.clone(), card);
        }

        self.empty.set_visible(hosts.is_empty());
    }

    /// Halka cizimini ag olmadan dogrulamak icin: her karta artan degerler.
    pub fn demo(self: &Rc<Self>) {
        self.rebuild();

        let samples: [f64; 4] = [18.0, 64.0, 92.0, 100.0];
        for (index, card) in self.cards.borrow().values().enumerate() {
            let base = samples[index % samples.len()];
            let values = [base, (base + 20.0).min(100.0), (base - 15.0).max(0.0)];
            for ((ring, reading), value) in
                card.rings.iter().zip(card.readings.iter()).zip(values)
            {
                ring.set_value(Some(value));
                reading.set_text(&format!("{value:.0}"));
            }
            card.summary.set_text("up 5d 13:52 · 4c · 5.8 GB");
            card.error.set_visible(false);
            card.gauges.set_visible(true);
        }
    }

    fn poll(self: &Rc<Self>) {
        let cards: Vec<Rc<Card>> = self.cards.borrow().values().cloned().collect();

        for card in cards {
            // Onceki tur hala suruyorsa yenisini kuyruga ekleme: yavas bir
            // sunucu butun turu geriletmesin.
            if card.busy.get() {
                continue;
            }
            card.busy.set(true);

            glib::spawn_future_local(async move {
                let result = run(&card.command, &card.alias).await;
                card.busy.set(false);

                match result {
                    Ok(output) => match metrics::parse(&output) {
                        Some(sample) => card.apply(sample),
                        None => card.fail("unexpected response"),
                    },
                    Err(message) => card.fail(&message),
                }
            });
        }
    }
}

impl Card {
    fn apply(&self, sample: metrics::Sample) {
        self.error.set_visible(false);
        self.gauges.set_visible(true);

        let cpu = metrics::cpu_percent(self.previous.get(), sample.cpu);
        self.previous.set(Some(sample.cpu));

        let values = [cpu, Some(sample.memory_percent()), Some(sample.disk_used_percent)];
        for ((ring, reading), value) in
            self.rings.iter().zip(self.readings.iter()).zip(values)
        {
            ring.set_value(value);
            reading.set_text(&match value {
                Some(value) => format!("{value:.0}"),
                None => "–".into(),
            });
        }

        self.summary.set_text(&sample.summary());
    }

    fn fail(&self, reason: &str) {
        self.previous.set(None);
        for (ring, reading) in self.rings.iter().zip(self.readings.iter()) {
            ring.set_value(None);
            reading.set_text("");
        }

        self.summary.set_text("");
        self.gauges.set_visible(false);
        self.error_text.set_text(&format!("[!] {reason}"));
        self.error.set_visible(true);
    }
}

async fn run(command: &[String], alias: &str) -> Result<String, String> {
    let args: Vec<&std::ffi::OsStr> =
        command.iter().map(|part| std::ffi::OsStr::new(part)).collect();

    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    );

    // Kanal henuz acik degilse parola keyring'den gelsin; terminalde
    // oldugu gibi komut satirinda hic gorunmuyor.
    if crate::data::secrets::has(alias) {
        if let Ok(exe) = std::env::current_exe() {
            launcher.setenv("SSH_ASKPASS", exe.as_os_str(), true);
            launcher.setenv(
                std::ffi::OsStr::new("SSH_ASKPASS_REQUIRE"),
                std::ffi::OsStr::new("force"),
                true,
            );
            launcher.setenv(
                std::ffi::OsStr::new("RUNETTY_ASKPASS_ALIAS"),
                std::ffi::OsStr::new(alias),
                true,
            );
        }
    }

    let process = launcher
        .spawn(&args)
        .map_err(|error| error.message().to_string())?;

    let (stdout, stderr) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|error| error.message().to_string())?;

    if process.is_successful() {
        return Ok(stdout.map(|text| text.to_string()).unwrap_or_default());
    }

    // ssh'in kendi mesaji kullanicinin isine yarar; ilk satiri yeterli.
    let message = stderr
        .map(|text| text.to_string())
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("connection failed")
        .trim()
        .to_string();

    Err(if message.is_empty() { "connection failed".into() } else { message })
}

fn build_card(host: &Host) -> Card {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    widget.add_css_class("monitor-card");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    header.add_css_class("monitor-card-header");

    let identity = gtk::Box::new(gtk::Orientation::Vertical, 2);
    identity.set_hexpand(true);
    identity.set_valign(gtk::Align::Center);

    let name = gtk::Label::new(Some(&host.alias));
    name.set_xalign(0.0);
    name.add_css_class("row-title");

    let target = gtk::Label::new(Some(&host.user_host()));
    target.set_xalign(0.0);
    target.add_css_class("row-subtitle");
    target.add_css_class("mono");

    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.add_css_class("row-meta");

    identity.append(&name);
    identity.append(&target);
    identity.append(&summary);

    let gauges = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    gauges.set_valign(gtk::Align::Center);
    gauges.set_halign(gtk::Align::End);

    let rings = [Ring::new(), Ring::new(), Ring::new()];
    let readings = [gtk::Label::new(None), gtk::Label::new(None), gtk::Label::new(None)];
    for ((ring, reading), label) in rings.iter().zip(readings.iter()).zip(["CPU", "MEM", "DISK"]) {
        gauges.append(&gauge(ring, reading, label));
    }

    // Hata, kartin icinde ve cozum eyleminin yaninda duruyor.
    let error = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    error.set_valign(gtk::Align::Center);
    error.set_halign(gtk::Align::End);
    error.set_visible(false);

    // Sabit genislik: mesaj uzunlugu degisince Retry'in yeri kaymasin,
    // kartlar alt alta hizali dursun.
    let error_text = gtk::Label::new(None);
    error_text.add_css_class("monitor-error");
    error_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    error_text.set_width_chars(44);
    error_text.set_max_width_chars(44);
    error_text.set_xalign(1.0);

    let retry = gtk::Button::with_label("Retry");
    retry.add_css_class("flat");
    retry.add_css_class("retry");
    retry.set_valign(gtk::Align::Center);

    error.append(&error_text);
    error.append(&retry);

    header.append(&identity);
    header.append(&gauges);
    header.append(&error);

    let detail_body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    detail_body.add_css_class("monitor-detail");

    let detail = gtk::Revealer::new();
    detail.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    detail.set_transition_duration(160);
    detail.set_child(Some(&detail_body));

    widget.append(&header);
    widget.append(&detail);

    let card = Card {
        widget,
        detail_command: metrics::detail_args(host),
        detail,
        detail_body,
        expanded: Cell::new(false),
        detail_busy: Cell::new(false),
        alias: host.alias.clone(),
        command: metrics::probe_args(host),
        rings,
        readings,
        summary,
        error,
        error_text,
        gauges,
        previous: Cell::new(None),
        busy: Cell::new(false),
    };

    let command = card.command.clone();
    let alias = card.alias.clone();
    let error_box = card.error.clone();
    let error_label = card.error_text.clone();
    retry.connect_clicked(move |button| {
        button.set_sensitive(false);
        error_label.set_text("[!] baglaniyor...");

        let command = command.clone();
        let alias = alias.clone();
        let button = button.clone();
        let error_box = error_box.clone();
        glib::spawn_future_local(async move {
            let result = run(&command, &alias).await;
            button.set_sensitive(true);
            if result.is_ok() {
                error_box.set_visible(false);
            }
        });
    });

    card
}

/// Karta tiklandiginda detayi acar/kapatir. Detay yalnizca acikken
/// sorgulaniyor; kapali kartlar icin fazladan tur yok.
fn attach_expander(card: &Rc<Card>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gtk::gdk::BUTTON_PRIMARY);

    card.widget.add_controller(gesture.clone());

    let card = Rc::clone(card);
    gesture.connect_released(move |_, _, _, _| {
        let open = !card.expanded.get();
        card.expanded.set(open);
        card.detail.set_reveal_child(open);

        if open {
            card.widget.add_css_class("expanded");
            load_detail(&card);
        } else {
            card.widget.remove_css_class("expanded");
        }
    });
}

fn load_detail(card: &Rc<Card>) {
    if card.detail_busy.get() {
        return;
    }
    card.detail_busy.set(true);

    let card = Rc::clone(card);
    glib::spawn_future_local(async move {
        let result = run(&card.detail_command, &card.alias).await;
        card.detail_busy.set(false);

        let body = &card.detail_body;
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        match result.ok().and_then(|output| metrics::parse_detail(&output)) {
            Some(detail) => {
                for panel in detail_panels(&detail) {
                    body.append(&panel);
                }
            }
            None => {
                let label = gtk::Label::new(Some("Detay alinamadi."));
                label.set_xalign(0.0);
                label.add_css_class("row-meta");
                body.append(&label);
            }
        }
    });
}

fn detail_panels(detail: &metrics::Detail) -> Vec<gtk::Widget> {
    let sample = &detail.sample;

    let cpu = panel(
        "CPU",
        &[
            ("Cekirdek", format!("{}", sample.cores)),
            ("Yuk (1/5/15)", format!(
                "{:.2} · {:.2} · {:.2}",
                detail.load[0], detail.load[1], detail.load[2]
            )),
            ("Calisma suresi", sample.uptime_text()),
        ],
    );

    let swap = if detail.swap_total_kb == 0 {
        "yok".to_string()
    } else {
        format!(
            "{} / {} ({:.0}%)",
            metrics::human_kb(detail.swap_total_kb - detail.swap_free_kb),
            metrics::human_kb(detail.swap_total_kb),
            detail.swap_percent()
        )
    };

    let memory = panel(
        "MEMORY",
        &[
            ("Kullanilan", format!(
                "{} / {} ({:.0}%)",
                metrics::human_kb(sample.mem_total_kb - sample.mem_available_kb),
                metrics::human_kb(sample.mem_total_kb),
                sample.memory_percent()
            )),
            ("Swap", swap),
        ],
    );

    let disks: Vec<(String, String, String)> = detail
        .disks
        .iter()
        .map(|disk| {
            (
                disk.mount.clone(),
                format!(
                    "{} / {}",
                    metrics::human_kb(disk.used_kb),
                    metrics::human_kb(disk.total_kb)
                ),
                format!("{:.0}%", disk.used_percent),
            )
        })
        .collect();

    let network: Vec<(String, String, String)> = detail
        .interfaces
        .iter()
        .map(|interface| {
            (
                interface.name.clone(),
                format!("↓ {}", metrics::human_kb(interface.rx_bytes / 1024)),
                format!("↑ {}", metrics::human_kb(interface.tx_bytes / 1024)),
            )
        })
        .collect();

    let processes: Vec<(String, String, String)> = detail
        .processes
        .iter()
        .map(|process| {
            (
                format!("{} ({})", process.name, process.pid),
                format!("{:.1}% cpu", process.cpu),
                format!("{:.1}% mem", process.mem),
            )
        })
        .collect();

    vec![
        cpu,
        memory,
        table("DISK", &["Baglanti noktasi", "Kullanim", ""], &disks),
        table("NETWORK", &["Arayuz", "Alinan", "Gonderilen"], &network),
        table("TOP PROCESSES", &["Surec", "CPU", "Bellek"], &processes),
    ]
}

fn panel(title: &str, rows: &[(&str, String)]) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.append(&super::rows::section_header(title));

    for (label, value) in rows {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);

        let name = gtk::Label::new(Some(label));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.add_css_class("row-subtitle");

        let data = gtk::Label::new(Some(value));
        data.set_xalign(1.0);
        data.add_css_class("row-subtitle");
        data.add_css_class("mono");

        line.append(&name);
        line.append(&data);
        column.append(&line);
    }

    column.upcast()
}

/// Ucuncu sutun bos birakilabiliyor; disk tablosunda basliksiz yuzde duruyor.
fn table(title: &str, headers: &[&str; 3], rows: &[(String, String, String)]) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.append(&super::rows::section_header(title));

    if rows.is_empty() {
        let empty = gtk::Label::new(Some("veri yok"));
        empty.set_xalign(0.0);
        empty.add_css_class("row-meta");
        column.append(&empty);
        return column.upcast();
    }

    let grid = gtk::Grid::new();
    grid.set_column_spacing(16);
    grid.set_row_spacing(2);
    grid.set_hexpand(true);

    for (index, header) in headers.iter().enumerate() {
        if header.is_empty() {
            continue;
        }
        let label = gtk::Label::new(Some(header));
        label.set_xalign(if index == 0 { 0.0 } else { 1.0 });
        label.add_css_class("row-meta");
        if index == 0 {
            label.set_hexpand(true);
        }
        grid.attach(&label, index as i32, 0, 1, 1);
    }

    for (row, (first, second, third)) in rows.iter().enumerate() {
        for (index, value) in [first, second, third].iter().enumerate() {
            let label = gtk::Label::new(Some(value));
            label.set_xalign(if index == 0 { 0.0 } else { 1.0 });
            label.add_css_class("row-subtitle");
            label.add_css_class("mono");
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            if index == 0 {
                label.set_hexpand(true);
            }
            grid.attach(&label, index as i32, row as i32 + 1, 1, 1);
        }
    }

    column.append(&grid);
    column.upcast()
}

fn gauge(ring: &Ring, reading: &gtk::Label, label: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_halign(gtk::Align::Center);

    // Yuzde halkanin icinde: renk esigi uzaktan, sayi yakindan okunuyor.
    ring.set_size_request(50, 50);
    reading.add_css_class("gauge-value");

    let dial = gtk::Overlay::new();
    dial.set_child(Some(ring));
    dial.add_overlay(reading);
    column.append(&dial);

    let caption = gtk::Label::new(Some(label));
    caption.add_css_class("gauge-label");
    column.append(&caption);

    column
}


pub fn build(store: Rc<RefCell<Store>>) -> Rc<Monitor> {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    list.set_margin_top(16);
    list.set_margin_bottom(16);
    list.set_margin_start(16);
    list.set_margin_end(16);

    let empty = gtk::Label::new(Some(
        "Izlenen sunucu yok. Listeden bir sunucuya sag tiklayip\n\
         \"Monitor'e ekle\" diyebilirsin.",
    ));
    empty.add_css_class("row-meta");
    empty.set_margin_top(24);
    list.append(&empty);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_child(Some(&list));

    // Baslik satiri: sabitleme dugmesi burada dursun ki sekmede de
    // sabit panelde de ayni yerde olsun.
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("monitor-bar");

    let title = gtk::Label::new(Some("Monitor"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("section-header");

    let pin = gtk::Button::from_icon_name("pin-right-symbolic");
    pin.add_css_class("flat");
    pin.set_tooltip_text(Some("Saga sabitle"));
    pin.set_valign(gtk::Align::Center);

    bar.append(&title);
    bar.append(&pin);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("content-bg");
    page.append(&bar);
    page.append(&scroller);

    let monitor = Rc::new(Monitor {
        widget: page.upcast(),
        pinned: Cell::new(false),
        pin_button: pin,
        list,
        empty,
        store,
        cards: RefCell::new(HashMap::new()),
        tick: RefCell::new(None),
    });

    monitor.pin_button.set_action_name(Some("win.toggle-monitor-pin"));
    monitor
}
