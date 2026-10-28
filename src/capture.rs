//! `--screenshot <file>`: render the window to a PNG and exit.
//! Phase one is judged by putting two screenshots side by side, so this
//! is the app's own development tool for that comparison.

use std::time::Duration;

use adw::prelude::*;
use gtk::{glib, gdk};

pub fn schedule(window: &adw::ApplicationWindow, path: String) {
    let window = window.clone();

    // A snapshot before the first frame is blank, so wait a frame clock tick.
    // The delay can be stretched to capture screens still waiting on the network.
    let delay = std::env::var("RUNETTY_SCREENSHOT_DELAY_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);

    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
        match capture(&window) {
            Some(texture) => match texture.save_to_png(&path) {
                Ok(()) => println!("runetty: {path}"),
                Err(error) => eprintln!("runetty: could not write PNG: {error}"),
            },
            None => eprintln!("runetty: window has not been drawn yet"),
        }
        // close() can be swallowed while a dialog is up; quit the app outright.
        match window.application() {
            Some(app) => app.quit(),
            None => window.close(),
        }
    });
}

fn capture(window: &adw::ApplicationWindow) -> Option<gdk::Texture> {
    // The window itself: dialogs and toasts live in a layer above the
    // content tree, not inside it.
    let widget = window.clone().upcast::<gtk::Widget>();
    let (width, height) = (widget.width() as f64, widget.height() as f64);
    if width < 1.0 || height < 1.0 {
        return None;
    }

    let paintable = gtk::WidgetPaintable::new(Some(&widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width, height);

    let node = snapshot.to_node()?;
    let renderer = window.native()?.renderer()?;
    Some(renderer.render_texture(&node, None))
}
