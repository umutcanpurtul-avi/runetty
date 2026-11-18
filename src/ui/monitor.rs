//! Monitor page: one card per host, three rings per card.
//!
//! A round is one `ssh` call per host; the channel stays open through
//! `ControlMaster`. Rounds stop entirely when the page is not visible - there
//! is no reason for a hidden tab to burn network and CPU.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};

use super::ring::Ring;
use crate::data::metrics::{self, CpuTimes};
use crate::data::store::Store;
use crate::data::Host;

const INTERVAL: Duration = Duration::from_secs(2);

struct Card {
    /// Outer box: summary row on top, expandable detail below.
    widget: gtk::Box,
    detail_command: Vec<String>,
    detail: gtk::Revealer,
    detail_body: gtk::Box,
    expanded: Cell<bool>,
    stacked: Rc<Cell<bool>>,
    /// Previous interface counters and when they were read, so rates can be
    /// derived. Counters alone only say "how much since boot".
    traffic: RefCell<Option<(Vec<metrics::Interface>, Instant)>>,
    network: gtk::Label,
    /// Tells askpass which connection this is; see `data::askpass_environment`.
    environment: Vec<(String, String)>,
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
    /// Pinned as a right-hand pane, or living in a tab?
    /// Shared with the cards: a pinned pane is narrow, so detail panels
    /// stack instead of sitting in two columns.
    pinned: Rc<Cell<bool>>,
    pin_button: gtk::Button,
    list: gtk::Box,
    empty: gtk::Label,
    store: Rc<RefCell<Store>>,
    cards: RefCell<HashMap<String, Rc<Card>>>,
    tick: RefCell<Option<glib::SourceId>>,
}

impl Monitor {
    /// Rounds start when the page becomes visible and stop when it hides.
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

    pub fn set_pinned(self: &Rc<Self>, pinned: bool) {
        let changed = self.pinned.get() != pinned;
        self.pinned.set(pinned);
        self.pin_button.set_icon_name(if pinned {
            "unpin-symbolic"
        } else {
            "pin-right-symbolic"
        });
        self.pin_button.set_tooltip_text(Some(if pinned {
            "Move back to a tab"
        } else {
            "Pin to the right"
        }));

        if pinned {
            self.widget.add_css_class("pinned");
        } else {
            self.widget.remove_css_class("pinned");
        }

        // Open details were laid out for the old width; rebuild them.
        if changed {
            for card in self.cards.borrow().values() {
                if card.expanded.get() {
                    refresh_detail(card);
                }
            }
        }
    }

    /// For verification: expands a card's detail programmatically.
    pub fn expand(self: &Rc<Self>, alias: &str) {
        self.rebuild();
        let card = self.cards.borrow().get(alias).cloned();
        let Some(card) = card else { return };

        card.expanded.set(true);
        card.detail.set_reveal_child(true);
        card.widget.add_css_class("expanded");
        refresh_detail(&card);
    }

    /// Called from outside when the watch list changes.
    pub fn rebuild_now(self: &Rc<Self>) {
        self.rebuild();
    }

    /// Rebuilds the cards when the record list changes.
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

            let card = Rc::new(build_card(host, Rc::clone(&self.pinned)));
            attach_expander(&card);
            self.list.append(&card.widget);
            self.cards.borrow_mut().insert(host.alias.clone(), card);
        }

        self.empty.set_visible(hosts.is_empty());
    }

    /// To verify ring drawing without a network: rising values per card.
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
                reading.set_text(&format!("{value:.0}%"));
            }
            card.summary.set_text("up 5d 13:52 · 4c · 5.8 GB");
            card.error.set_visible(false);
            card.gauges.set_visible(true);
        }
    }

    fn poll(self: &Rc<Self>) {
        let cards: Vec<Rc<Card>> = self.cards.borrow().values().cloned().collect();

        for card in cards {
            // Skip hosts whose previous round is still running, so one slow
            // host does not hold back the whole round.
            if card.busy.get() {
                continue;
            }
            card.busy.set(true);

            // An expanded card asks for the detail probe, which already
            // contains everything the summary needs: one call, not two.
            let expanded = card.expanded.get();
            let command = if expanded {
                card.detail_command.clone()
            } else {
                card.command.clone()
            };

            glib::spawn_future_local(async move {
                let result = run(&command, &card.environment).await;
                card.busy.set(false);

                let output = match result {
                    Ok(output) => output,
                    Err(message) => return card.fail(&message),
                };

                if expanded {
                    match metrics::parse_detail(&output) {
                        Some(detail) => {
                            // Read before apply() overwrites it.
                            let previous_cpu = card.previous.get();
                            card.apply(detail.sample.clone());
                            card.apply_detail(&detail, previous_cpu);
                        }
                        None => card.fail("unexpected response"),
                    }
                } else {
                    match metrics::parse(&output) {
                        Some(sample) => card.apply(sample),
                        None => card.fail("unexpected response"),
                    }
                }
            });
        }
    }
}

impl Card {
    /// Rebuilds the detail panels from a fresh reading. Panels are recreated
    /// rather than patched in place: at one round every two seconds the cost
    /// is invisible, and it keeps the layout code in a single place.
    /// `previous_cpu` must be the reading from before `apply` ran, or the
    /// delta collapses to zero and the breakdown comes out empty.
    fn apply_detail(
        self: &Rc<Self>,
        detail: &metrics::Detail,
        previous_cpu: Option<metrics::CpuTimes>,
    ) {
        let rates = self.rates(&detail.interfaces);
        self.update_network_summary(&rates);

        let body = &self.detail_body;
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        body.append(&detail_columns(detail, &rates, previous_cpu, self.stacked.get()));
    }

    /// Converts counters into per-second rates using the previous reading.
    fn rates(&self, interfaces: &[metrics::Interface]) -> Vec<(String, metrics::Rate)> {
        let now = Instant::now();
        let previous = self.traffic.replace(Some((interfaces.to_vec(), now)));

        let Some((before, at)) = previous else {
            return interfaces.iter().map(|i| (i.name.clone(), metrics::Rate::default())).collect();
        };

        let seconds = now.duration_since(at).as_secs_f64();
        interfaces
            .iter()
            .map(|current| {
                let rate = before
                    .iter()
                    .find(|old| old.name == current.name)
                    .map(|old| metrics::rate(old, current, seconds))
                    .unwrap_or_default();
                (current.name.clone(), rate)
            })
            .collect()
    }

    fn update_network_summary(&self, rates: &[(String, metrics::Rate)]) {
        let total = rates.iter().fold(metrics::Rate::default(), |mut acc, (_, rate)| {
            acc.rx += rate.rx;
            acc.tx += rate.tx;
            acc
        });

        self.network.set_text(&format!(
            "↓ {}\n↑ {}",
            metrics::human_rate(total.rx),
            metrics::human_rate(total.tx)
        ));
        self.network.set_visible(true);
    }

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
                Some(value) => format!("{value:.0}%"),
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
        self.network.set_visible(false);
        self.gauges.set_visible(false);
        self.error_text.set_text(&format!("[!] {reason}"));
        self.error.set_visible(true);
    }
}

async fn run(command: &[String], environment: &[(String, String)]) -> Result<String, String> {
    let args: Vec<&std::ffi::OsStr> =
        command.iter().map(|part| std::ffi::OsStr::new(part)).collect();

    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    );

    // With no channel open yet, the keyring may answer - but only for this
    // host, which is what the environment tells askpass.
    for (key, value) in environment {
        launcher.setenv(std::ffi::OsStr::new(key), std::ffi::OsStr::new(value), true);
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

    // ssh's own message is what helps the user; the first line is enough.
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

fn build_card(host: &Host, stacked: Rc<Cell<bool>>) -> Card {
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

    // Live network summary beside the rings, as a rate rather than a total.
    let network = gtk::Label::new(None);
    network.set_xalign(0.0);
    network.set_valign(gtk::Align::Center);
    network.set_visible(false);
    network.add_css_class("row-subtitle");
    network.add_css_class("mono");

    let gauges = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    gauges.set_valign(gtk::Align::Center);
    gauges.set_halign(gtk::Align::End);

    let rings = [Ring::new(), Ring::new(), Ring::new()];
    let readings = [gtk::Label::new(None), gtk::Label::new(None), gtk::Label::new(None)];
    for ((ring, reading), label) in rings.iter().zip(readings.iter()).zip(["CPU", "MEM", "DISK"]) {
        gauges.append(&gauge(ring, reading, label));
    }

    // The error sits inside the card, next to the action that fixes it.
    let error = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    error.set_valign(gtk::Align::Center);
    error.set_halign(gtk::Align::End);
    error.set_visible(false);

    // Fixed width so Retry does not shift as the message length changes and
    // cards stay aligned down the column.
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
    header.append(&network);
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
        traffic: RefCell::new(None),
        network: network.clone(),
        stacked,
        environment: crate::data::askpass_environment(host),
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
    let environment = card.environment.clone();
    let error_box = card.error.clone();
    let error_label = card.error_text.clone();
    retry.connect_clicked(move |button| {
        button.set_sensitive(false);
        error_label.set_text("[!] connecting...");

        let command = command.clone();
        let environment = environment.clone();
        let button = button.clone();
        let error_box = error_box.clone();
        glib::spawn_future_local(async move {
            let result = run(&command, &environment).await;
            button.set_sensitive(true);
            if result.is_ok() {
                error_box.set_visible(false);
            }
        });
    });

    card
}

/// Toggles the detail when a card is clicked. Detail is only probed while
/// expanded, so collapsed cards cost nothing extra.
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
            refresh_detail(&card);
        } else {
            card.widget.remove_css_class("expanded");
        }
    });
}

/// Fetches the detail straight away, without waiting for the next round.
/// Used when a card is expanded and when pinning changes the layout.
fn refresh_detail(card: &Rc<Card>) {
    if card.busy.get() {
        return;
    }
    card.busy.set(true);

    let card = Rc::clone(card);
    let command = card.detail_command.clone();
    glib::spawn_future_local(async move {
        let result = run(&command, &card.environment).await;
        card.busy.set(false);

        match result.ok().and_then(|output| metrics::parse_detail(&output)) {
            Some(detail) => {
                let previous_cpu = card.previous.get();
                card.apply(detail.sample.clone());
                card.apply_detail(&detail, previous_cpu);
            }
            None => {
                let body = &card.detail_body;
                while let Some(child) = body.first_child() {
                    body.remove(&child);
                }
                let label = gtk::Label::new(Some("Could not fetch detail."));
                label.set_xalign(0.0);
                label.add_css_class("row-meta");
                body.append(&label);
            }
        }
    });
}

/// Panels are spread across two columns:
///   CPU     | MEMORY
///   DISK    | TOP PROCESSES
///   NETWORK |
/// The left column holds short fixed rows, the right one the growing tables.
/// In a narrow pane (pinned Monitor) the columns stack instead.
/// Panels are spread across two columns:
///   CPU     | MEMORY
///   DISK    | NETWORK INTERFACES
///            TOP PROCESSES
/// A pinned pane is narrow, so the columns stack there.
fn detail_columns(
    detail: &metrics::Detail,
    rates: &[(String, metrics::Rate)],
    previous_cpu: Option<metrics::CpuTimes>,
    stacked: bool,
) -> gtk::Widget {
    let sample = &detail.sample;

    let left = gtk::Box::new(gtk::Orientation::Vertical, 14);
    left.set_hexpand(true);
    left.append(&cpu_panel(detail, previous_cpu));
    left.append(&disk_panel(detail));

    let right = gtk::Box::new(gtk::Orientation::Vertical, 14);
    right.set_hexpand(true);
    right.append(&memory_panel(detail));
    right.append(&network_panel(detail, rates));

    let columns = gtk::Box::new(
        if stacked { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal },
        if stacked { 14 } else { 28 },
    );
    columns.set_homogeneous(!stacked);
    columns.append(&left);
    columns.append(&right);

    // Process rows are long; they get the full width underneath.
    let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    page.append(&columns);
    page.append(&process_panel(detail));

    let _ = sample;
    page.upcast()
}

/// A panel header with the headline figure on the right and a meter below.
fn panel_head(title: &str, value: &str, fraction: f64) -> gtk::Box {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let name = gtk::Label::new(Some(title));
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.add_css_class("section-header");

    let figure = gtk::Label::new(Some(value));
    figure.set_xalign(1.0);
    figure.add_css_class("panel-figure");

    head.append(&name);
    head.append(&figure);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.append(&head);
    column.append(&meter(fraction));
    column
}

/// A thin bar; the threshold class is set here, the colour in style.css.
fn meter(fraction: f64) -> gtk::LevelBar {
    let bar = gtk::LevelBar::new();
    bar.set_min_value(0.0);
    bar.set_max_value(100.0);
    bar.set_value(fraction.clamp(0.0, 100.0));
    bar.add_css_class("meter");

    if fraction >= 85.0 {
        bar.add_css_class("meter-alert");
    } else if fraction >= 60.0 {
        bar.add_css_class("meter-warn");
    }

    bar
}

fn cpu_panel(detail: &metrics::Detail, previous: Option<metrics::CpuTimes>) -> gtk::Widget {
    let sample = &detail.sample;
    let breakdown = metrics::cpu_breakdown(previous, sample.cpu);

    let busy = breakdown.map(|b| b.busy());
    let column = panel_head(
        "CPU",
        &busy.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "–".into()),
        busy.unwrap_or(0.0),
    );

    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(breakdown) = breakdown {
        rows.push(("User", format!("{:.1}%", breakdown.user)));
        rows.push(("System", format!("{:.1}%", breakdown.system)));
        rows.push(("IO Wait", format!("{:.1}%", breakdown.iowait)));
        rows.push(("Steal", format!("{:.1}%", breakdown.steal)));
        rows.push(("Idle", format!("{:.1}%", breakdown.idle)));
    }

    rows.push(("Cores", sample.cores.to_string()));
    rows.push((
        "Load 1 / 5 / 15",
        format!("{:.2} / {:.2} / {:.2}", detail.load[0], detail.load[1], detail.load[2]),
    ));
    rows.push(("Uptime", sample.uptime_text()));

    column.append(&rows_box(&rows));
    column.upcast()
}

fn memory_panel(detail: &metrics::Detail) -> gtk::Widget {
    let sample = &detail.sample;
    let used = sample.mem_total_kb.saturating_sub(sample.mem_available_kb);
    let percent = sample.memory_percent();

    let column = panel_head("MEMORY", &format!("{percent:.0}%"), percent);

    let swap = if detail.swap_total_kb == 0 {
        "0 B".to_string()
    } else {
        format!(
            "{} / {} ({:.0}%)",
            metrics::human_kb(detail.swap_total_kb - detail.swap_free_kb),
            metrics::human_kb(detail.swap_total_kb),
            detail.swap_percent()
        )
    };

    column.append(&rows_box(&[
        ("Total", metrics::human_kb(sample.mem_total_kb)),
        ("Used", metrics::human_kb(used)),
        ("Free", metrics::human_kb(sample.mem_free_kb)),
        ("Page Cache", metrics::human_kb(sample.mem_cached_kb)),
        ("Available", metrics::human_kb(sample.mem_available_kb)),
        ("Swap", swap),
    ]));
    column.upcast()
}

fn disk_panel(detail: &metrics::Detail) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.append(&super::rows::section_header("DISK"));

    if detail.disks.is_empty() {
        column.append(&no_data());
        return column.upcast();
    }

    for disk in &detail.disks {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let mount = gtk::Label::new(Some(&disk.mount));
        mount.set_xalign(0.0);
        mount.set_hexpand(true);
        mount.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        mount.add_css_class("row-subtitle");
        mount.add_css_class("mono");

        let size = gtk::Label::new(Some(&format!(
            "{} / {}",
            metrics::human_kb(disk.used_kb),
            metrics::human_kb(disk.total_kb)
        )));
        size.set_xalign(1.0);
        size.add_css_class("row-subtitle");
        size.add_css_class("mono");

        line.append(&mount);
        line.append(&size);

        let entry = gtk::Box::new(gtk::Orientation::Vertical, 2);
        entry.append(&line);
        entry.append(&meter(disk.used_percent));
        column.append(&entry);
    }

    column.upcast()
}

fn network_panel(
    detail: &metrics::Detail,
    rates: &[(String, metrics::Rate)],
) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.append(&super::rows::section_header("NETWORK INTERFACES"));

    if detail.interfaces.is_empty() {
        column.append(&no_data());
        return column.upcast();
    }

    let grid = gtk::Grid::new();
    grid.set_column_spacing(14);
    grid.set_row_spacing(2);
    grid.set_hexpand(true);

    for (index, header) in ["INTERFACE", "RX", "TX", "RX TOTAL", "TX TOTAL"].iter().enumerate() {
        let label = gtk::Label::new(Some(header));
        label.set_xalign(if index == 0 { 0.0 } else { 1.0 });
        label.add_css_class("row-meta");
        if index == 0 {
            label.set_hexpand(true);
        }
        grid.attach(&label, index as i32, 0, 1, 1);
    }

    for (row, interface) in detail.interfaces.iter().enumerate() {
        let rate = rates
            .iter()
            .find(|(name, _)| name == &interface.name)
            .map(|(_, rate)| *rate)
            .unwrap_or_default();

        let cells = [
            interface.name.clone(),
            metrics::human_rate(rate.rx),
            metrics::human_rate(rate.tx),
            metrics::human_kb(interface.rx_bytes / 1024),
            metrics::human_kb(interface.tx_bytes / 1024),
        ];

        for (index, value) in cells.iter().enumerate() {
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

fn process_panel(detail: &metrics::Detail) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.append(&super::rows::section_header("TOP PROCESSES"));

    if detail.processes.is_empty() {
        column.append(&no_data());
        return column.upcast();
    }

    let grid = gtk::Grid::new();
    grid.set_column_spacing(14);
    grid.set_row_spacing(2);
    grid.set_hexpand(true);

    for (index, header) in ["PID", "COMMAND", "CPU", "MEM"].iter().enumerate() {
        let label = gtk::Label::new(Some(header));
        label.set_xalign(if index <= 1 { 0.0 } else { 1.0 });
        label.add_css_class("row-meta");
        if index == 1 {
            label.set_hexpand(true);
        }
        grid.attach(&label, index as i32, 0, 1, 1);
    }

    for (row, process) in detail.processes.iter().enumerate() {
        let cells = [
            process.pid.to_string(),
            process.name.clone(),
            format!("{:.1}%", process.cpu),
            format!("{:.1}%", process.mem),
        ];

        for (index, value) in cells.iter().enumerate() {
            let label = gtk::Label::new(Some(value));
            label.set_xalign(if index <= 1 { 0.0 } else { 1.0 });
            label.add_css_class("row-subtitle");
            label.add_css_class("mono");
            if index == 1 {
                label.set_hexpand(true);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_tooltip_text(Some(value));
            }
            grid.attach(&label, index as i32, row as i32 + 1, 1, 1);
        }
    }

    column.append(&grid);
    column.upcast()
}

fn rows_box(rows: &[(&str, String)]) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);

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

    column
}

fn no_data() -> gtk::Label {
    let label = gtk::Label::new(Some("no data"));
    label.set_xalign(0.0);
    label.add_css_class("row-meta");
    label
}



fn gauge(ring: &Ring, reading: &gtk::Label, label: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_halign(gtk::Align::Center);

    // Percentage inside the ring: the colour threshold reads from afar, the
    // number up close.
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
        "No hosts are being watched. Right-click a host in the list and choose\n\
         \"Add to Monitor\".",
    ));
    empty.add_css_class("row-meta");
    empty.set_margin_top(24);
    list.append(&empty);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_child(Some(&list));

    // Header row: the pin button lives here so it is in the same place
    // whether Monitor is a tab or a pinned pane.
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("monitor-bar");

    let title = gtk::Label::new(Some("Monitor"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("section-header");

    let pin = gtk::Button::from_icon_name("pin-right-symbolic");
    pin.add_css_class("flat");
    pin.set_tooltip_text(Some("Pin to the right"));
    pin.set_valign(gtk::Align::Center);

    bar.append(&title);
    bar.append(&pin);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("content-bg");
    page.append(&bar);
    page.append(&scroller);

    let monitor = Rc::new(Monitor {
        widget: page.upcast(),
        pinned: Rc::new(Cell::new(false)),
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
