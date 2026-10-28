//! Agents panel: long-running tmux sessions across the watched hosts.
//!
//! One `tmux list-sessions` per host, over the same shared SSH channel the
//! terminal and Monitor use. Nothing is installed on the far side, and hosts
//! without tmux simply contribute nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use super::rows;
use crate::data::agents::{self, Agent};
use crate::data::store::Store;

/// Sessions change far more slowly than metrics, so a slower round is plenty.
const INTERVAL: Duration = Duration::from_secs(5);

pub struct Agents {
    pub widget: gtk::Widget,
    list: gtk::Box,
    store: Rc<RefCell<Store>>,
    attach: Rc<dyn Fn(&str)>,
    tick: RefCell<Option<glib::SourceId>>,
    busy: Cell<bool>,
}

impl Agents {
    /// Rounds run only while the panel is on screen.
    pub fn set_active(self: &Rc<Self>, active: bool) {
        if !active {
            if let Some(id) = self.tick.borrow_mut().take() {
                id.remove();
            }
            return;
        }

        self.poll();

        if self.tick.borrow().is_some() {
            return;
        }

        let this = Rc::clone(self);
        let id = glib::timeout_add_local(INTERVAL, move || {
            this.poll();
            glib::ControlFlow::Continue
        });
        *self.tick.borrow_mut() = Some(id);
    }

    fn poll(self: &Rc<Self>) {
        // One round at a time: a slow host must not pile up requests.
        if self.busy.get() {
            return;
        }

        let hosts = self.store.borrow().monitored();
        if hosts.is_empty() {
            self.show(&[], 0);
            return;
        }

        self.busy.set(true);
        let this = Rc::clone(self);

        glib::spawn_future_local(async move {
            let mut found: Vec<Agent> = Vec::new();
            let mut unreachable = 0;

            for host in hosts {
                match run(&agents::probe_args(&host)).await {
                    Ok(output) => found.extend(agents::parse(&output, &host.alias)),
                    Err(_) => unreachable += 1,
                }
            }

            found.sort_by(|a, b| b.activity.cmp(&a.activity));
            this.busy.set(false);
            this.show(&found, unreachable);
        });
    }

    fn show(self: &Rc<Self>, found: &[Agent], unreachable: usize) {
        let list = &self.list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        list.append(&rows::section_header("Agents"));

        if found.is_empty() {
            list.append(&empty_hint(&self.store.borrow(), unreachable));
            return;
        }

        let now = crate::data::now();
        let rows_box = gtk::ListBox::new();
        rows_box.set_selection_mode(gtk::SelectionMode::Single);

        for agent in found {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&agent_row(agent, now)));
            rows_box.append(&row);
        }

        let agents: Vec<Agent> = found.to_vec();
        let this = Rc::clone(self);
        rows_box.connect_row_activated(move |_, row| {
            let Some(agent) = agents.get(row.index() as usize) else { return };
            let host = this
                .store
                .borrow()
                .hosts()
                .into_iter()
                .find(|host| host.alias == agent.host);

            if let Some(host) = host {
                (this.attach)(&agent.attach_command(&host));
            }
        });

        list.append(&rows_box);

        if unreachable > 0 {
            list.append(&note(&format!("{unreachable} host(s) unreachable")));
        }
    }
}

/// Name on top, `host · N windows` below, state dot and badge around them.
fn agent_row(agent: &Agent, now: u64) -> gtk::Box {
    let state = agent.state(now);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("session-row");

    let dot = rows::status_dot(state.css());
    dot.set_tooltip_text(Some(state.label()));
    row.append(&dot);

    let stack = gtk::Box::new(gtk::Orientation::Vertical, 1);
    stack.set_hexpand(true);
    stack.set_valign(gtk::Align::Center);

    let name = gtk::Label::new(Some(&agent.name));
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.add_css_class("row-title");

    let windows = if agent.windows == 1 { "window" } else { "windows" };
    let detail = gtk::Label::new(Some(&format!(
        "{} · {} {windows}",
        agent.host, agent.windows
    )));
    detail.set_xalign(0.0);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    detail.add_css_class("row-subtitle");
    detail.add_css_class("mono");

    stack.append(&name);
    stack.append(&detail);
    row.append(&stack);

    let badge = rows::badge(state.label(), state.css());
    row.append(&badge);
    row
}

fn empty_hint(store: &Store, unreachable: usize) -> gtk::Label {
    let text = if store.monitored().is_empty() {
        "No watched hosts. Right-click a host and choose \"Add to Monitor\";\n\
         its tmux sessions appear here."
    } else if unreachable > 0 {
        "No sessions found. The watched hosts could not be reached."
    } else {
        "No tmux sessions on the watched hosts."
    };

    note(text)
}

fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_margin_start(12);
    label.set_margin_end(12);
    label.set_margin_top(4);
    label.add_css_class("row-meta");
    label
}

async fn run(command: &[String]) -> Result<String, String> {
    let args: Vec<&std::ffi::OsStr> =
        command.iter().map(|part| std::ffi::OsStr::new(part)).collect();

    let process = gio::Subprocess::newv(
        &args,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .map_err(|error| error.message().to_string())?;

    let (stdout, _stderr) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|error| error.message().to_string())?;

    if !process.is_successful() {
        return Err("unreachable".into());
    }

    Ok(stdout.map(|text| text.to_string()).unwrap_or_default())
}

pub fn build(store: Rc<RefCell<Store>>, attach: Rc<dyn Fn(&str)>) -> Rc<Agents> {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_child(Some(&list));

    let agents = Rc::new(Agents {
        widget: scroller.upcast(),
        list,
        store,
        attach,
        tick: RefCell::new(None),
        busy: Cell::new(false),
    });

    agents.show(&[], 0);
    agents
}
