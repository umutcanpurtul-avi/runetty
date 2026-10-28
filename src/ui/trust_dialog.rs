//! Ilk baglantida "bu sunucuya guveniyor musun?" diyalogu.
//! ssh'in terminalde sordugu sorunun arayuz karsiligi.

use adw::prelude::*;

use crate::data::hostkey;

/// Anahtar bilinmiyorsa diyalogu gosterir ve kullanici guvenirse `proceed`
/// calisir. Bilinen ya da karar verilemeyen durumda dogrudan `proceed`.
pub fn ensure_trusted<F>(
    parent: &impl IsA<gtk::Widget>,
    host: &str,
    port: u16,
    proceed: F,
) where
    F: Fn() + 'static,
{
    if hostkey::status(host, port) != hostkey::Known::No {
        proceed();
        return;
    }

    let keys = match hostkey::scan(host, port) {
        Ok(keys) => keys,
        Err(reason) => {
            // Anahtar alinamadi: ssh'in kendi akisina birak, hata orada gorunur.
            eprintln!("runetty: {host} anahtari alinamadi: {reason}");
            proceed();
            return;
        }
    };

    let dialog = adw::AlertDialog::new(
        Some("Bilinmeyen sunucu"),
        Some(&format!(
            "{host} ile ilk kez baglaniyorsun. Parmak izini bekledigin \
             degerle karsilastir; dogruysa guvenebilirsin."
        )),
    );

    dialog.set_extra_child(Some(&fingerprint_list(&keys)));
    dialog.add_response("cancel", "Vazgec");
    dialog.add_response("trust", "Guven ve baglan");
    dialog.set_response_appearance("trust", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response != "trust" {
            return;
        }

        if let Err(error) = hostkey::trust(&keys) {
            eprintln!("runetty: anahtar kaydedilemedi: {error}");
            return;
        }

        proceed();
    });

    dialog.present(Some(parent));
}

fn fingerprint_list(keys: &[String]) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    group.set_title("Parmak izi");

    let fingerprints = hostkey::fingerprints(keys);
    if fingerprints.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("Parmak izi hesaplanamadi");
        row.set_subtitle("ssh-keygen bulunamadi");
        group.add(&row);
        return group.upcast();
    }

    for (kind, fingerprint) in fingerprints {
        let row = adw::ActionRow::new();
        row.set_title(&kind);
        row.set_subtitle(&fingerprint);
        row.set_subtitle_selectable(true);
        row.add_css_class("mono");
        group.add(&row);
    }

    group.upcast()
}
