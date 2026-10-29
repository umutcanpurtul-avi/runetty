//! The `SSH_ASKPASS` side of runetty.
//!
//! ssh runs this when it needs a secret and reads the answer from stdout.
//! Because we set `SSH_ASKPASS_REQUIRE=force`, ssh routes *every* prompt here,
//! not just host passwords: key passphrases come through as well. Answering a
//! passphrase prompt with a host password fails the connection, so the prompt
//! text decides what is being asked.
//!
//! Anything not found in the keyring is asked for in a small window, which is
//! what a conventional ssh-askpass does.

use adw::prelude::*;
use gtk::glib;

use crate::data::secrets;

/// Keyring entries for passphrases are keyed by the key file, so one key
/// shared by many hosts is only ever stored once.
fn passphrase_alias(path: &str) -> String {
    format!("key:{path}")
}

/// `Enter passphrase for key '/home/u/.ssh/id_ed25519':` -> the path.
fn passphrase_target(prompt: &str) -> Option<String> {
    if !prompt.to_lowercase().contains("passphrase") {
        return None;
    }

    let start = prompt.find('\'')?;
    let rest = &prompt[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

pub fn run() -> glib::ExitCode {
    let prompt: String = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with("--"))
        .unwrap_or_default();

    let alias = match passphrase_target(&prompt) {
        Some(path) => passphrase_alias(&path),
        None => match std::env::var("RUNETTY_ASKPASS_ALIAS") {
            Ok(alias) => alias,
            // No idea what is being asked and nothing to look up: let ssh fall
            // back to its own prompt rather than answering wrongly.
            Err(_) => return glib::ExitCode::FAILURE,
        },
    };

    if let Some(secret) = secrets::lookup(&alias) {
        println!("{secret}");
        return glib::ExitCode::SUCCESS;
    }

    ask(&prompt, &alias)
}

/// A minimal password window. Returning failure makes ssh give up, which is
/// the right outcome when the user cancels.
fn ask(prompt: &str, alias: &str) -> glib::ExitCode {
    if gtk::init().is_err() {
        return glib::ExitCode::FAILURE;
    }
    adw::init().ok();

    crate::load_stylesheet();

    let answer: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    let window = adw::ApplicationWindow::builder()
        .default_width(420)
        .title("runetty")
        .modal(true)
        .resizable(false)
        .build();
    window.add_css_class("runetty");

    let entry = adw::PasswordEntryRow::new();
    entry.set_title("Password");

    let remember = adw::SwitchRow::new();
    remember.set_title("Remember in the keyring");
    remember.set_active(true);

    let group = adw::PreferencesGroup::new();
    group.set_description(Some(if prompt.is_empty() {
        "ssh is asking for a secret."
    } else {
        prompt
    }));
    group.add(&entry);
    group.add(&remember);

    let cancel = gtk::Button::with_label("Cancel");
    let unlock = gtk::Button::with_label("Unlock");
    unlock.add_css_class("suggested-action");

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&unlock);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.append(&group);
    body.append(&buttons);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    window.set_content(Some(&view));

    let accept = {
        let answer = std::rc::Rc::clone(&answer);
        let entry = entry.clone();
        let remember = remember.clone();
        let window = window.clone();
        let alias = alias.to_string();

        move || {
            let secret = entry.text().to_string();
            if secret.is_empty() {
                return;
            }

            if remember.is_active() {
                if let Err(error) = secrets::store(&alias, &secret) {
                    eprintln!("runetty: could not store secret: {error}");
                }
            }

            *answer.borrow_mut() = Some(secret);
            window.close();
        }
    };

    {
        let accept = accept.clone();
        unlock.connect_clicked(move |_| accept());
    }
    {
        let accept = accept.clone();
        entry.connect_entry_activated(move |_| accept());
    }
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }

    let loop_ = glib::MainLoop::new(None, false);
    {
        let loop_ = loop_.clone();
        window.connect_close_request(move |_| {
            loop_.quit();
            glib::Propagation::Proceed
        });
    }

    window.present();
    loop_.run();

    let secret = answer.borrow_mut().take();
    match secret {
        Some(secret) => {
            println!("{secret}");
            glib::ExitCode::SUCCESS
        }
        None => glib::ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A passphrase prompt names the key file; a password prompt does not.
    #[test]
    fn passphrase_prompts_are_recognised() {
        let prompt = "Enter passphrase for key '/home/u/.ssh/id_ed25519': ";
        assert_eq!(passphrase_target(prompt).as_deref(), Some("/home/u/.ssh/id_ed25519"));

        assert_eq!(passphrase_target("root@10.0.0.1's password: "), None);
        assert_eq!(passphrase_target(""), None);
    }

    /// Passphrases are keyed by file, so one key serves every host using it.
    #[test]
    fn passphrase_alias_is_per_key_file() {
        assert_eq!(passphrase_alias("/home/u/.ssh/id_ed25519"), "key:/home/u/.ssh/id_ed25519");
    }
}
