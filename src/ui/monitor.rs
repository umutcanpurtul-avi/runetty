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
    widget: gtk::Box,
    command: Vec<String>,
    rings: [Ring; 3],
    summary: gtk::Label,
    error: gtk::Box,
    error_text: gtk::Label,
    gauges: gtk::Box,
    previous: Cell<Option<CpuTimes>>,
    busy: Cell<bool>,
}

pub struct Monitor {
    pub widget: gtk::Widget,
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

    /// Kayit listesi degisince kartlari yeniden kurar.
    fn rebuild(self: &Rc<Self>) {
        let hosts = self.store.borrow().hosts();
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
            self.list.append(&card.widget);
            self.cards.borrow_mut().insert(host.alias.clone(), card);
        }

        self.empty.set_visible(hosts.is_empty());
    }

    /// Halka cizimini ag olmadan dogrulamak icin: her karta artan degerler.
    pub fn demo(self: &Rc<Self>) {
        self.rebuild();

        let samples = [18.0, 64.0, 92.0, 100.0];
        for (index, card) in self.cards.borrow().values().enumerate() {
            let base = samples[index % samples.len()];
            card.rings[0].set_value(Some(base));
            card.rings[1].set_value(Some((base + 20.0).min(100.0)));
            card.rings[2].set_value(Some((base - 15.0).max(0.0)));
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
                let result = run(&card.command).await;
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

        self.rings[0].set_value(cpu);
        self.rings[1].set_value(Some(sample.memory_percent()));
        self.rings[2].set_value(Some(sample.disk_used_percent));
        self.summary.set_text(&sample.summary());
    }

    fn fail(&self, reason: &str) {
        self.previous.set(None);
        for ring in &self.rings {
            ring.set_value(None);
        }

        self.summary.set_text("");
        self.gauges.set_visible(false);
        self.error_text.set_text(&format!("[!] {reason}"));
        self.error.set_visible(true);
    }
}

async fn run(command: &[String]) -> Result<String, String> {
    let args: Vec<&std::ffi::OsStr> =
        command.iter().map(|part| std::ffi::OsStr::new(part)).collect();

    let process = gio::Subprocess::newv(
        &args,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
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
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    widget.add_css_class("monitor-card");

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

    let rings = [Ring::new(), Ring::new(), Ring::new()];
    for (ring, label) in rings.iter().zip(["CPU", "MEM", "DISK"]) {
        gauges.append(&gauge(ring, label));
    }

    // Hata, kartin icinde ve cozum eyleminin yaninda duruyor.
    let error = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    error.set_valign(gtk::Align::Center);
    error.set_visible(false);

    let error_text = gtk::Label::new(None);
    error_text.add_css_class("monitor-error");
    error_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    error_text.set_max_width_chars(42);

    let retry = gtk::Button::with_label("Retry");
    retry.add_css_class("flat");
    retry.add_css_class("retry");

    error.append(&error_text);
    error.append(&retry);

    widget.append(&identity);
    widget.append(&gauges);
    widget.append(&error);

    let card = Card {
        widget,
        command: metrics::probe_args(&ssh_target(host)),
        rings,
        summary,
        error,
        error_text,
        gauges,
        previous: Cell::new(None),
        busy: Cell::new(false),
    };

    let command = card.command.clone();
    let error_box = card.error.clone();
    let error_label = card.error_text.clone();
    retry.connect_clicked(move |button| {
        button.set_sensitive(false);
        error_label.set_text("[!] baglaniyor...");

        let command = command.clone();
        let button = button.clone();
        let error_box = error_box.clone();
        glib::spawn_future_local(async move {
            let result = run(&command).await;
            button.set_sensitive(true);
            if result.is_ok() {
                error_box.set_visible(false);
            }
        });
    });

    card
}

fn gauge(ring: &Ring, label: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_halign(gtk::Align::Center);

    ring.set_size_request(44, 44);
    column.append(ring);

    let caption = gtk::Label::new(Some(label));
    caption.add_css_class("gauge-label");
    column.append(&caption);

    column
}

/// ssh config kaydinda alias yeterli; yerel kayitta user@host kuruyoruz.
fn ssh_target(host: &Host) -> String {
    match host.source {
        crate::data::Source::SshConfig => host.alias.clone(),
        crate::data::Source::Local => match &host.user {
            Some(user) => format!("{user}@{}", host.hostname),
            None => host.hostname.clone(),
        },
    }
}

pub fn build(store: Rc<RefCell<Store>>) -> Rc<Monitor> {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    list.set_margin_top(16);
    list.set_margin_bottom(16);
    list.set_margin_start(16);
    list.set_margin_end(16);

    let empty = gtk::Label::new(Some("Izlenecek sunucu yok."));
    empty.add_css_class("row-meta");
    empty.set_margin_top(24);
    list.append(&empty);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_child(Some(&list));

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("content-bg");
    page.append(&scroller);

    Rc::new(Monitor {
        widget: page.upcast(),
        list,
        empty,
        store,
        cards: RefCell::new(HashMap::new()),
        tick: RefCell::new(None),
    })
}
