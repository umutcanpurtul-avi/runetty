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

/// Which connection this askpass was started for, as told by runetty through
/// the environment. Prompts are judged against it rather than on their
/// wording alone.
#[derive(Debug, Default)]
struct Context {
    alias: Option<String>,
    /// Names the target may appear under in a prompt: alias and HostName.
    hosts: Vec<String>,
    user: Option<String>,
    /// Key files that may legitimately ask for a passphrase.
    keys: Vec<String>,
}

impl Context {
    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        Self {
            alias: var("RUNETTY_ASKPASS_ALIAS"),
            hosts: var("RUNETTY_ASKPASS_HOSTS")
                .map(|value| value.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            user: var("RUNETTY_ASKPASS_USER"),
            keys: var("RUNETTY_ASKPASS_KEYS")
                .map(|value| value.lines().map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }

    /// Whether `user@host` names the machine this password was saved for.
    /// A jump host, or anything else ssh happens to be talking to, does not.
    fn is_target(&self, who: &str) -> bool {
        let Some((user, host)) = who.rsplit_once('@') else {
            return false;
        };

        let host_matches = self.hosts.iter().any(|name| name.eq_ignore_ascii_case(host));
        let user_matches = self.user.as_deref().is_none_or(|expected| expected == user);

        host_matches && user_matches
    }
}

/// What a prompt is, and therefore what may answer it.
#[derive(Debug, PartialEq)]
enum Ask {
    /// A passphrase for one of the keys this connection uses.
    Passphrase(String),
    /// The password of the host this connection is for.
    Password,
    /// Something the user may answer, but the keyring may not: a server's own
    /// keyboard-interactive text, a jump host's password, a key we did not
    /// expect. Shown with where it came from, and never stored.
    Untrusted(String),
    /// ssh's own "do you trust this host key?" question. runetty checks keys
    /// before connecting, but not for hosts behind a jump host or proxy: only
    /// ssh knows which machine it reached there, so the question is passed on.
    HostKey,
    /// Not a secret at all.
    Refuse,
}

/// OpenSSH prefixes keyboard-interactive prompts with `(user@host) `. What
/// follows is written by the server, so its wording proves nothing.
fn server_prompt(prompt: &str) -> Option<(&str, &str)> {
    let rest = prompt.trim_start().strip_prefix('(')?;
    let (who, text) = rest.split_once(") ")?;
    who.contains('@').then_some((who, text))
}

/// `user@host's password: ` - written by ssh itself, naming the host asking.
fn password_prompt(prompt: &str) -> Option<&str> {
    let who = prompt.trim().strip_suffix("'s password:")?;
    who.contains('@').then_some(who)
}

fn classify(prompt: &str, context: &Context) -> Ask {
    let lower = prompt.to_lowercase();

    // Server-written text first: a server can phrase its prompt to look like
    // anything, including a local passphrase request naming one of our key
    // files or ssh's own host key question. Only the plain password of the
    // right host is answered for it.
    if let Some((who, text)) = server_prompt(prompt) {
        let text = text.to_lowercase();
        if context.is_target(who) && text.contains("password") && !text.contains("passphrase") {
            return Ask::Password;
        }
        return Ask::Untrusted(format!("{who} asks"));
    }

    // A yes/no is never a secret. The one ssh asks about a new host key is
    // put to the user; any other is refused.
    if lower.contains("yes/no") || lower.contains("fingerprint") {
        return if lower.starts_with("the authenticity of host") && lower.contains("(yes/no") {
            Ask::HostKey
        } else {
            Ask::Refuse
        };
    }

    if lower.contains("passphrase") {
        return match quoted_path(prompt) {
            Some(path) if context.keys.iter().any(|key| key == &path) => Ask::Passphrase(path),
            _ => Ask::Untrusted("ssh asks".into()),
        };
    }

    if let Some(who) = password_prompt(prompt) {
        // With ProxyJump the same askpass answers the jump host too, and its
        // prompt also says "password". Only the target gets the target's.
        return if context.is_target(who) {
            Ask::Password
        } else {
            Ask::Untrusted(format!("{who} asks"))
        };
    }

    Ask::Refuse
}

/// `Enter passphrase for key '/home/u/.ssh/id_ed25519':` -> the path.
fn quoted_path(prompt: &str) -> Option<String> {
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

    let context = Context::from_env();

    let alias = match classify(&prompt, &context) {
        Ask::Passphrase(path) => passphrase_alias(&path),
        Ask::Password => match &context.alias {
            Some(alias) => alias.clone(),
            None => return glib::ExitCode::FAILURE,
        },
        // Asked of the user, never looked up and never remembered.
        Ask::Untrusted(source) => return ask(&prompt, None, Some(&source)),
        Ask::HostKey => return confirm_host_key(&prompt),
        Ask::Refuse => return glib::ExitCode::FAILURE,
    };

    if let Some(secret) = secrets::lookup(&alias) {
        println!("{secret}");
        return glib::ExitCode::SUCCESS;
    }

    ask(&prompt, Some(&alias), None)
}

/// A label that shows its text literally - no markup, no mnemonics.
fn plain_label(text: Option<&str>) -> gtk::Label {
    let label = gtk::Label::new(text);
    label.set_use_markup(false);
    label.set_use_underline(false);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label
}

/// The prompt as it is safe to show: control and bidi-override characters
/// dropped (a server could send escape sequences or reorder what the user
/// reads) and the length capped.
fn displayable(prompt: &str) -> Option<String> {
    const LIMIT: usize = 400;

    let text: String = prompt
        .trim()
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect();
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(LIMIT) {
        Some((cut, _)) => format!("{}\u{2026}", &text[..cut]),
        None => text,
    })
}

/// Puts ssh's host key question to the user. "yes" makes ssh record the key
/// itself, honouring HashKnownHosts and the rest of its configuration; a
/// cancel makes it give up.
fn confirm_host_key(prompt: &str) -> glib::ExitCode {
    if gtk::init().is_err() {
        return glib::ExitCode::FAILURE;
    }
    adw::init().ok();

    crate::load_stylesheet();

    let trusted = std::rc::Rc::new(std::cell::Cell::new(false));

    let dialog = adw::AlertDialog::new(Some("Unknown host"), None);
    // The question ends in ssh's "(yes/no/[fingerprint])?", which the buttons
    // answer; showing it would invite typing.
    let question = prompt.rsplit_once("Are you sure").map_or(prompt, |(text, _)| text);
    let message = plain_label(displayable(question).as_deref());
    message.add_css_class("mono");
    dialog.set_extra_child(Some(&message));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("trust", "Trust and connect");
    dialog.set_response_appearance("trust", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let loop_ = glib::MainLoop::new(None, false);
    {
        let trusted = std::rc::Rc::clone(&trusted);
        let loop_ = loop_.clone();
        dialog.connect_response(None, move |_, response| {
            trusted.set(response == "trust");
            loop_.quit();
        });
    }

    let window = adw::ApplicationWindow::builder()
        .default_width(480)
        .title("runetty")
        .build();
    window.add_css_class("runetty");
    window.present();
    dialog.present(Some(&window));
    loop_.run();

    if trusted.get() {
        println!("yes");
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    }
}

/// A minimal password window. Returning failure makes ssh give up, which is
/// the right outcome when the user cancels.
///
/// `store_as` is the keyring entry an answer may be saved under; `None` for
/// anything untrusted, where there is no option to save at all. `source`
/// names who is asking when the text did not come from ssh itself - the user
/// should know a server wrote it before typing a secret into it.
fn ask(prompt: &str, store_as: Option<&str>, source: Option<&str>) -> glib::ExitCode {
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

    // Plain labels, never markup: the prompt can be text a server wrote, and
    // AdwPreferencesGroup renders its title and description as Pango markup.
    let heading = plain_label(source.map(|source| format!("{source}:")).as_deref());
    heading.add_css_class("heading");

    let message = plain_label(Some(&match source {
        // Text from the far side is shown as a quotation from someone else,
        // not as runetty speaking.
        Some(_) => format!(
            "\u{201c}{}\u{201d}\n\nThis text comes from the other end of the \
             connection, not from runetty. Only answer if you expected it.",
            displayable(prompt).unwrap_or_else(|| "(no text)".into())
        ),
        None => displayable(prompt).unwrap_or_else(|| "ssh is asking for a secret.".into()),
    }));
    message.add_css_class("dim-label");

    let group = adw::PreferencesGroup::new();
    group.add(&entry);
    if store_as.is_some() {
        group.add(&remember);
    }

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
    if source.is_some() {
        body.append(&heading);
    }
    body.append(&message);
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
        let store_as = store_as.map(str::to_string);

        move || {
            let secret = entry.text().to_string();
            if secret.is_empty() {
                return;
            }

            if let Some(alias) = store_as.as_deref().filter(|_| remember.is_active()) {
                if let Err(error) = secrets::store(alias, &secret) {
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

    /// The connection runetty would describe for root@10.0.0.1, saved as "edge".
    fn edge() -> Context {
        Context {
            alias: Some("edge".into()),
            hosts: vec!["edge".into(), "10.0.0.1".into()],
            user: Some("root".into()),
            keys: vec!["/home/u/.ssh/id_ed25519".into()],
        }
    }

    // --- the legitimate paths still work ---

    #[test]
    fn the_targets_password_is_answered() {
        assert_eq!(classify("root@10.0.0.1's password: ", &edge()), Ask::Password);
        assert_eq!(classify("root@edge's password: ", &edge()), Ask::Password);
    }

    /// PAM-based hosts ask through keyboard-interactive; the target's own
    /// password prompt there must keep working.
    #[test]
    fn the_targets_keyboard_interactive_password_is_answered() {
        assert_eq!(classify("(root@10.0.0.1) Password: ", &edge()), Ask::Password);
    }

    #[test]
    fn an_expected_keys_passphrase_is_answered() {
        let prompt = "Enter passphrase for key '/home/u/.ssh/id_ed25519': ";
        assert_eq!(
            classify(prompt, &edge()),
            Ask::Passphrase("/home/u/.ssh/id_ed25519".into())
        );
    }

    // --- finding 1: a server must not obtain a stored passphrase ---

    /// The reported attack: a server sends text worded exactly like ssh's own
    /// passphrase prompt, naming a real key file, through keyboard-interactive.
    #[test]
    fn a_server_cannot_ask_for_a_passphrase() {
        let attack = "(root@10.0.0.1) Enter passphrase for key '/home/u/.ssh/id_ed25519':";
        assert_eq!(classify(attack, &edge()), Ask::Untrusted("root@10.0.0.1 asks".into()));
    }

    /// Even the target's own server only gets its password through this path,
    /// and only when the text actually asks for one.
    #[test]
    fn server_text_that_is_not_a_password_is_untrusted() {
        for text in ["(root@10.0.0.1) Verification code: ", "(root@10.0.0.1) OTP: "] {
            assert!(matches!(classify(text, &edge()), Ask::Untrusted(_)), "{text}");
        }
    }

    /// A different machine using keyboard-interactive gets nothing.
    #[test]
    fn another_servers_password_prompt_is_untrusted() {
        assert!(matches!(
            classify("(root@evil.example) Password: ", &edge()),
            Ask::Untrusted(_)
        ));
    }

    // --- finding 2: a jump host must not receive the target's password ---

    /// With ProxyJump the same askpass answers the bastion as well.
    #[test]
    fn a_jump_host_does_not_get_the_targets_password() {
        assert_eq!(
            classify("admin@bastion's password: ", &edge()),
            Ask::Untrusted("admin@bastion asks".into())
        );
    }

    #[test]
    fn the_right_host_with_the_wrong_user_is_not_the_target() {
        assert!(matches!(
            classify("deploy@10.0.0.1's password: ", &edge()),
            Ask::Untrusted(_)
        ));
    }

    /// With nothing in the environment nothing is the target.
    #[test]
    fn an_empty_context_trusts_nothing() {
        let empty = Context::default();
        assert!(matches!(classify("root@10.0.0.1's password: ", &empty), Ask::Untrusted(_)));
    }

    // --- everything else ---

    #[test]
    fn an_unexpected_keys_passphrase_is_untrusted() {
        let prompt = "Enter passphrase for key '/tmp/other_key': ";
        assert!(matches!(classify(prompt, &edge()), Ask::Untrusted(_)));
    }

    #[test]
    fn the_host_key_question_goes_to_the_user() {
        let prompt = "The authenticity of host '10.0.0.1' can't be established.\n\
                      ED25519 key fingerprint is SHA256:abc.\n\
                      Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert_eq!(classify(prompt, &edge()), Ask::HostKey);
    }

    /// A server cannot pose as ssh asking about a key, and other yes/no
    /// questions are not put to the user at all.
    #[test]
    fn only_ssh_itself_asks_about_host_keys() {
        let posing = "(admin@web) The authenticity of host 'x' can't be established.\n\
                      Are you sure you want to continue connecting (yes/no)? ";
        assert_eq!(classify(posing, &edge()), Ask::Untrusted("admin@web asks".into()));
        assert_eq!(classify("Allow this (yes/no)? ", &edge()), Ask::Refuse);
    }

    #[test]
    fn anything_unrecognised_is_refused() {
        assert_eq!(classify("", &edge()), Ask::Refuse);
        assert_eq!(classify("Something else entirely", &edge()), Ask::Refuse);
    }

    /// Passphrases are keyed by file, so one key serves every host using it.
    #[test]
    fn passphrase_alias_is_per_key_file() {
        assert_eq!(passphrase_alias("/home/u/.ssh/id_ed25519"), "key:/home/u/.ssh/id_ed25519");
    }

    #[test]
    fn server_text_is_shown_without_its_tricks() {
        assert_eq!(displayable("  \x1b[31mred\u{202e}txt.exe  "), Some("[31mredtxt.exe".into()));
        assert_eq!(displayable("\x07\x08"), None);
        assert_eq!(displayable(&"a".repeat(1000)).map(|t| t.chars().count()), Some(401));
        // Markup is not interpreted by the label; it only has to survive intact.
        assert_eq!(displayable("<b>Password</b>"), Some("<b>Password</b>".into()));
    }
}
