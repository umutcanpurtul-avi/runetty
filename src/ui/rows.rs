//! The two-layer list row and its helpers.
//! The plan's "custom widget" item: no custom drawing, just gtk::Box composition.

use gtk::prelude::*;

use super::model::Agent;
use crate::data::sessions::State;
use crate::data::{self, Host};

pub fn section_header(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class("section-header");
    label
}

pub fn status_dot(state_css: &str) -> gtk::Box {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    dot.add_css_class(state_css);
    dot.set_valign(gtk::Align::Center);
    dot
}

pub fn badge(text: &str, state_css: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("badge");
    label.add_css_class(state_css);
    label.set_valign(gtk::Align::Center);
    label
}

/// Name (Inter) on top, `user@host` (monospace) below, meta on the right.
fn two_layer(title: &str, subtitle: &str, lead: Option<gtk::Widget>, trail: gtk::Widget) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("session-row");

    if let Some(lead) = lead {
        row.append(&lead);
    }

    let stack = gtk::Box::new(gtk::Orientation::Vertical, 1);
    stack.set_hexpand(true);
    stack.set_valign(gtk::Align::Center);

    let title_label = gtk::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title_label.add_css_class("row-title");

    let subtitle_label = gtk::Label::new(Some(subtitle));
    subtitle_label.set_xalign(0.0);
    subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle_label.add_css_class("row-subtitle");
    subtitle_label.add_css_class("mono");

    stack.append(&title_label);
    stack.append(&subtitle_label);

    trail.set_valign(gtk::Align::Center);

    row.append(&stack);
    row.append(&trail);
    row
}

pub fn host_row(host: &Host, state: State) -> gtk::Box {
    let when = gtk::Label::new(Some(&data::relative(host.last_used)));
    when.add_css_class("row-meta");

    let dot = status_dot(state.css());
    dot.set_tooltip_text(Some(state.tooltip()));

    two_layer(&host.alias, &host.user_host(), Some(dot.upcast()), when.upcast())
}

pub fn agent_row(agent: &Agent) -> gtk::Box {
    let dot = status_dot(agent.state.css());
    let badge = badge(agent.state.label(), agent.state.css());
    two_layer(agent.name, agent.host, Some(dot.upcast()), badge.upcast())
}
