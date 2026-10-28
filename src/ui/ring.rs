//! CPU / MEM / DISK halkasi.
//!
//! Ozel cizim `snapshot` ile yapiliyor (cairo degil): on kart x uc halka,
//! iki saniyede bir yeniden ciziliyor. Renk widget'in CSS renginden geliyor,
//! yani esik kararini Rust veriyor ama paleti yine style.css tutuyor.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Ring {
        pub value: Cell<f64>,
        pub known: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Ring {
        const NAME: &'static str = "RunettyRing";
        type Type = super::Ring;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Ring {}

    impl WidgetImpl for Ring {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (44, 44, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let size = widget.width().min(widget.height()) as f32;
            if size <= 0.0 {
                return;
            }

            let thickness = (size * 0.11).max(3.0);
            let radius = (size - thickness) / 2.0;
            let center = gtk::graphene::Point::new(
                widget.width() as f32 / 2.0,
                widget.height() as f32 / 2.0,
            );

            let track = gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 0.08);
            let stroke = gsk::Stroke::new(thickness);
            stroke.set_line_cap(gsk::LineCap::Round);

            // Zemin halkasi her zaman cizilir: deger bilinmese de yer tutuyor.
            let builder = gsk::PathBuilder::new();
            builder.add_circle(&center, radius);
            snapshot.append_stroke(&builder.to_path(), &stroke, &track);

            if !self.known.get() {
                return;
            }

            let fraction = (self.value.get() / 100.0).clamp(0.0, 1.0) as f32;
            if fraction <= 0.0 {
                return;
            }

            // Tam dolu yay `arc_to` ile cizilemez; daireyi oldugu gibi kullan.
            let path = if fraction >= 0.999 {
                let builder = gsk::PathBuilder::new();
                builder.add_circle(&center, radius);
                builder.to_path()
            } else {
                arc(&center, radius, fraction)
            };

            snapshot.append_stroke(&path, &stroke, &widget.color());
        }
    }

    /// Saat yonunde, tepeden baslayan yay.
    fn arc(center: &gtk::graphene::Point, radius: f32, fraction: f32) -> gsk::Path {
        use std::f32::consts::PI;

        let start = -PI / 2.0;
        let sweep = fraction * 2.0 * PI;
        let steps = ((fraction * 64.0).ceil() as usize).max(2);

        let builder = gsk::PathBuilder::new();
        builder.move_to(
            center.x() + radius * start.cos(),
            center.y() + radius * start.sin(),
        );

        // Kisa dogru parcalari: 64 adimda halka gozle duz gorunmuyor ve
        // her turda yeniden kurmak conic yay matematiginden ucuz.
        for step in 1..=steps {
            let angle = start + sweep * (step as f32 / steps as f32);
            builder.line_to(
                center.x() + radius * angle.cos(),
                center.y() + radius * angle.sin(),
            );
        }

        builder.to_path()
    }
}

use gtk::gsk;

glib::wrapper! {
    pub struct Ring(ObjectSubclass<imp::Ring>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub fn new() -> Self {
        let ring: Self = glib::Object::new();
        ring.add_css_class("ring");
        ring
    }

    /// `None` = olcum yok. Sifir gostermek "her sey yolunda" demek olurdu.
    pub fn set_value(&self, value: Option<f64>) {
        let imp = self.imp();

        match value {
            Some(value) => {
                if imp.known.get() && (imp.value.get() - value).abs() < 0.5 {
                    return; // Degismediyse yeniden cizme.
                }
                imp.value.set(value);
                imp.known.set(true);
            }
            None => {
                if !imp.known.get() {
                    return;
                }
                imp.known.set(false);
            }
        }

        self.apply_threshold(value);
        self.queue_draw();
    }

    /// Esik karari burada, renk style.css'te.
    fn apply_threshold(&self, value: Option<f64>) {
        for class in ["ring-warn", "ring-alert"] {
            self.remove_css_class(class);
        }

        match value {
            Some(value) if value >= 85.0 => self.add_css_class("ring-alert"),
            Some(value) if value >= 60.0 => self.add_css_class("ring-warn"),
            _ => {}
        }
    }
}
