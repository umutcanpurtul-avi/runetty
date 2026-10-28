//! Pencere iskeleti: ikon rayi + OverlaySplitView + sekmeli icerik.
//! Veri degisiklikleri tek bir `refresh` uzerinden kenar cubugu ve bos duruma
//! birlikte yansir; iki yerde ayri durum tutmuyoruz.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::data::sessions::{self, Sessions};
use crate::data::store::Store;
use crate::ui::{empty_state, host_dialog, icon_rail, sidebar};

pub fn build(
    app: &adw::Application,
    mode: icon_rail::Mode,
    exec: Option<&str>,
    autoconnect: Option<&str>,
) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1180)
        .default_height(760)
        .title("runetty")
        .build();
    window.add_css_class("runetty");

    let store = Rc::new(RefCell::new(Store::load()));
    let live = Sessions::new();
    let tabs = adw::TabView::new();
    let content_stack = gtk::Stack::new();

    // Ham komut: bos durumun komut alanindan gelir.
    // Durum noktasi: sekme acilinca yesil, surec olunce sari, sekme kapaninca
    // kayit listeden dusup kirmiziya doner. Gec baglama yine REFRESH uzerinden.
    let mark: Rc<dyn Fn(&str, sessions::State)> = {
        let live = Rc::clone(&live);
        Rc::new(move |alias: &str, state| {
            live.set(alias, state);
            with_refresh(|refresh| refresh());
        })
    };

    let run: Rc<dyn Fn(&str)> = {
        let tabs = tabs.clone();
        let stack = content_stack.clone();
        let mark = Rc::clone(&mark);
        Rc::new(move |command: &str| open_tab(&tabs, &stack, command, None, &mark))
    };

    // Kayitli sunucu: zaman damgasi guncellenir, sonra komut calisir.
    let connect: Rc<dyn Fn(&str)> = {
        let store = Rc::clone(&store);
        let tabs = tabs.clone();
        let stack = content_stack.clone();
        let mark = Rc::clone(&mark);
        let refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let refresh_slot = Rc::clone(&refresh);
        REFRESH.with(|cell| *cell.borrow_mut() = Some(refresh_slot));

        Rc::new(move |alias: &str| {
            let command = {
                let mut store = store.borrow_mut();
                let host = store.hosts().into_iter().find(|host| host.alias == alias);
                match host {
                    Some(host) => {
                        store.touch(alias);
                        host.command()
                    }
                    None => return,
                }
            };

            open_tab(&tabs, &stack, &command, Some(alias), &mark);
            if let Some(refresh) = refresh.borrow().as_ref() {
                refresh();
            }
        })
    };

    let sidebar = sidebar::build(Rc::clone(&store), Rc::clone(&live), Rc::clone(&connect));
    sidebar.set_mode(mode);

    let empty = empty_state::build(Rc::clone(&store), Rc::clone(&connect), Rc::clone(&run));

    let refresh: Rc<dyn Fn()> = {
        let sidebar = Rc::clone(&sidebar);
        let empty = Rc::clone(&empty);
        Rc::new(move || {
            sidebar.refresh();
            empty.refresh();
        })
    };
    REFRESH.with(|cell| {
        if let Some(slot) = cell.borrow().as_ref() {
            *slot.borrow_mut() = Some(Rc::clone(&refresh));
        }
    });

    let split = adw::OverlaySplitView::new();
    split.set_hexpand(true);
    split.set_sidebar_width_fraction(0.22);
    split.set_max_sidebar_width(300.0);
    split.set_min_sidebar_width(220.0);
    split.set_sidebar(Some(&sidebar.widget));

    // Mod secici kenar cubugunun ust barinda: listenin tam ustunde,
    // hangi listeye baktigin soldaki ayri bir raya bakmadan belli oluyor.
    let rail = icon_rail::build(mode, {
        let sidebar = Rc::clone(&sidebar);
        move |mode| sidebar.set_mode(mode)
    });
    sidebar.header.pack_start(&rail);

    let search = gtk::SearchEntry::new();
    search.add_css_class("global-search");
    search.set_placeholder_text(Some("Search servers, sessions, agents"));

    let header = adw::HeaderBar::new();
    header.add_css_class("runetty");
    header.set_title_widget(Some(&search));

    header.pack_end(&icon_button("open-menu-symbolic", "Menu"));
    header.pack_end(&icon_button("preferences-system-symbolic", "Preferences"));
    header.pack_end(&icon_button("view-refresh-symbolic", "Reconnect"));

    content_stack.set_vexpand(true);
    content_stack.add_named(&empty_page(&empty.widget), Some("empty"));
    content_stack.add_named(&tabs_page(&tabs), Some("tabs"));
    content_stack.set_visible_child_name("empty");

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&header);
    content_view.set_content(Some(&content_stack));
    split.set_content(Some(&content_view));

    window.set_content(Some(&split));

    install_actions(&window, &split, &search, &store, &connect, &refresh);

    // Son sekme kapaninca bos duruma geri don.
    {
        let stack = content_stack.clone();
        tabs.connect_n_pages_notify(move |view| {
            stack.set_visible_child_name(if view.n_pages() == 0 { "empty" } else { "tabs" });
        });
    }

    if let Some(command) = exec {
        open_tab(&tabs, &content_stack, command, None, &mark);
    }

    if let Some(alias) = autoconnect {
        connect(alias);
    }

    window
}

// `connect` ile `refresh` birbirine bagimli; biri digerinden once kuruluyor.
// Dugumu tek bir gec-baglama noktasiyla cozuyoruz.
thread_local! {
    static REFRESH: RefCell<Option<Rc<RefCell<Option<Rc<dyn Fn()>>>>>> =
        const { RefCell::new(None) };
}

fn with_refresh<F: FnOnce(&Rc<dyn Fn()>)>(action: F) {
    let refresh = REFRESH.with(|cell| {
        cell.borrow().as_ref().and_then(|slot| slot.borrow().clone())
    });
    if let Some(refresh) = refresh {
        action(&refresh);
    }
}

fn empty_page(child: &gtk::Box) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("content-bg");
    page.append(child);
    page.upcast()
}

fn tabs_page(tabs: &adw::TabView) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("content-bg");

    let bar = adw::TabBar::new();
    bar.add_css_class("runetty");
    bar.set_view(Some(tabs));
    bar.set_autohide(false);

    page.append(&bar);
    page.append(tabs);
    page.upcast()
}

fn open_tab(
    tabs: &adw::TabView,
    stack: &gtk::Stack,
    command: &str,
    alias: Option<&str>,
    mark: &Rc<dyn Fn(&str, sessions::State)>,
) {
    let child = terminal_widget(command, alias, mark);
    let page = tabs.append(&child);
    page.set_title(&tab_title(command));
    tabs.set_selected_page(&page);
    stack.set_visible_child_name("tabs");

    if let Some(alias) = alias {
        mark(alias, sessions::State::Open);

        // Sekme kapaninca kayit tekrar "acik oturum yok" durumuna doner.
        // Sinyal bu sayfaya ozel, bu yuzden ilk tetiklenmede kendini soker.
        let mark = Rc::clone(mark);
        let alias = alias.to_string();
        let closed = page.clone();
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> =
            Rc::new(RefCell::new(None));
        let slot = Rc::clone(&handler);

        *handler.borrow_mut() = Some(tabs.connect_page_detached(move |view, page, _| {
            if page != &closed {
                return;
            }
            mark(&alias, sessions::State::Closed);
            if let Some(id) = slot.borrow_mut().take() {
                view.disconnect(id);
            }
        }));
    }
}

#[cfg(feature = "terminal")]
fn terminal_widget(
    command: &str,
    alias: Option<&str>,
    mark: &Rc<dyn Fn(&str, sessions::State)>,
) -> gtk::Widget {
    let owned = alias.map(str::to_string);
    let mark = Rc::clone(mark);

    crate::ui::terminal::page(Some(command), alias, move |status| {
        if let Some(alias) = &owned {
            // Sekme hala acik ama surec bitti: baglanti dustu.
            mark(alias, if status == 0 {
                sessions::State::Closed
            } else {
                sessions::State::Trouble
            });
        }
    })
}

#[cfg(not(feature = "terminal"))]
fn terminal_widget(
    command: &str,
    _alias: Option<&str>,
    _mark: &Rc<dyn Fn(&str, sessions::State)>,
) -> gtk::Widget {
    let label = gtk::Label::new(Some(&format!(
        "Terminal destegi bu derlemede kapali.\n{command}"
    )));
    label.add_css_class("dim");
    label.set_vexpand(true);
    label.upcast()
}

/// "ssh deploy@web-a.prod" -> "web-a.prod"
fn tab_title(command: &str) -> String {
    command
        .strip_prefix("ssh ")
        .map(|rest| rest.split_whitespace().last().unwrap_or(rest))
        .and_then(|target| target.split('@').next_back())
        .map(str::to_string)
        .unwrap_or_else(|| {
            command.split_whitespace().next().unwrap_or(command).to_string()
        })
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button
}

fn install_actions(
    window: &adw::ApplicationWindow,
    split: &adw::OverlaySplitView,
    search: &gtk::SearchEntry,
    store: &Rc<RefCell<Store>>,
    connect: &Rc<dyn Fn(&str)>,
    refresh: &Rc<dyn Fn()>,
) {
    let focus_search = gio::SimpleAction::new("global-search", None);
    {
        let search = search.clone();
        focus_search.connect_activate(move |_, _| {
            search.grab_focus();
        });
    }
    window.add_action(&focus_search);

    let toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
    {
        let split = split.clone();
        toggle_sidebar.connect_activate(move |_, _| {
            split.set_show_sidebar(!split.shows_sidebar());
        });
    }
    window.add_action(&toggle_sidebar);

    let string = Some(glib::VariantTy::STRING);

    // Sag tik menusu: alias eylemin hedefi olarak geliyor.
    let host_connect = gio::SimpleAction::new("host-connect", string);
    {
        let connect = Rc::clone(connect);
        host_connect.connect_activate(move |_, target| {
            if let Some(alias) = target.and_then(glib::Variant::str) {
                connect(alias);
            }
        });
    }
    window.add_action(&host_connect);

    let host_edit = gio::SimpleAction::new("host-edit", string);
    {
        let window = window.clone();
        let store = Rc::clone(store);
        let refresh = Rc::clone(refresh);
        host_edit.connect_activate(move |_, target| {
            let alias = target.and_then(glib::Variant::str).map(str::to_string);
            let refresh = Rc::clone(&refresh);
            host_dialog::present(&window, Rc::clone(&store), alias, move || refresh());
        });
    }
    window.add_action(&host_edit);

    let host_new = gio::SimpleAction::new("host-new", None);
    {
        let window = window.clone();
        let store = Rc::clone(store);
        let refresh = Rc::clone(refresh);
        host_new.connect_activate(move |_, _| {
            let refresh = Rc::clone(&refresh);
            host_dialog::present(&window, Rc::clone(&store), None, move || refresh());
        });
    }
    window.add_action(&host_new);

    let host_delete = gio::SimpleAction::new("host-delete", string);
    {
        let window = window.clone();
        let store = Rc::clone(store);
        let refresh = Rc::clone(refresh);
        host_delete.connect_activate(move |_, target| {
            let Some(alias) = target.and_then(glib::Variant::str).map(str::to_string) else {
                return;
            };
            confirm_delete(&window, Rc::clone(&store), alias, Rc::clone(&refresh));
        });
    }
    window.add_action(&host_delete);

    let reload = gio::SimpleAction::new("reload-ssh-config", None);
    {
        let store = Rc::clone(store);
        let refresh = Rc::clone(refresh);
        reload.connect_activate(move |_, _| {
            store.borrow_mut().reload_ssh_config();
            refresh();
        });
    }
    window.add_action(&reload);

    let restore = gio::SimpleAction::new("restore-hidden", None);
    {
        let store = Rc::clone(store);
        let refresh = Rc::clone(refresh);
        restore.connect_activate(move |_, _| {
            store.borrow_mut().unhide_all();
            refresh();
        });
    }
    window.add_action(&restore);

    // Faz 2'de doldurulacak.
    let palette = gio::SimpleAction::new("command-palette", None);
    palette.connect_activate(|_, _| glib::g_debug!("runetty", "TODO: command palette"));
    window.add_action(&palette);
}

/// Silme geri alinamadigi icin onay sart; ssh config kaydinda ne oldugunu da
/// acikca soyluyoruz — kullanicinin dosyasina dokunmuyoruz.
fn confirm_delete(
    window: &adw::ApplicationWindow,
    store: Rc<RefCell<Store>>,
    alias: String,
    refresh: Rc<dyn Fn()>,
) {
    use crate::data::Source;

    let source = store
        .borrow()
        .hosts()
        .into_iter()
        .find(|host| host.alias == alias)
        .map(|host| host.source);

    let body = match source {
        Some(Source::SshConfig) => format!(
            "\"{alias}\" ~/.ssh/config icinde tanimli. runetty o dosyaya yazmaz; \
             kayit yalnizca listeden gizlenir ve + menusunden geri getirilebilir."
        ),
        _ => format!("\"{alias}\" kaydi silinsin mi? Bu islem geri alinamaz."),
    };

    let dialog = adw::AlertDialog::new(Some("Delete"), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response(
        "delete",
        if matches!(source, Some(Source::SshConfig)) { "Hide" } else { "Delete" },
    );
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            store.borrow_mut().remove(&alias);
            refresh();
        }
    });

    dialog.present(Some(window));
}
