//! `--screenshot <dosya>`: pencereyi PNG'ye basip cikar.
//! Faz 1'in gecme olcutu "iki ekran goruntusu yan yana ayirt edilemiyor"
//! oldugu icin bu, uygulamanin kendi gelistirme araci.

use std::time::Duration;

use adw::prelude::*;
use gtk::{glib, gdk};

pub fn schedule(window: &adw::ApplicationWindow, path: String) {
    let window = window.clone();

    // Ilk kare cizilmeden anlik goruntu bos cikar; bir frame clock turu bekle.
    // Ag bekleyen ekranlari yakalamak icin gecikme uzatilabiliyor.
    let delay = std::env::var("RUNETTY_SCREENSHOT_DELAY_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);

    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
        match capture(&window) {
            Some(texture) => match texture.save_to_png(&path) {
                Ok(()) => println!("runetty: {path}"),
                Err(error) => eprintln!("runetty: PNG yazilamadi: {error}"),
            },
            None => eprintln!("runetty: pencere henuz cizilmemis"),
        }
        // Diyalog acikken close() yutulabiliyor; uygulamayi dogrudan bitir.
        match window.application() {
            Some(app) => app.quit(),
            None => window.close(),
        }
    });
}

fn capture(window: &adw::ApplicationWindow) -> Option<gdk::Texture> {
    // Pencerenin kendisi: diyaloglar ve toast'lar icerik agacinin disinda,
    // ustune bindirilmis bir katmanda duruyor.
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
