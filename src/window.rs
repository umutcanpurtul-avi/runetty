//! Pencere iskeleti: ikon rayi + OverlaySplitView + sekmeli icerik.
//! Veri degisiklikleri tek bir `refresh` uzerinden kenar cubugu ve bos duruma
//! birlikte yansir; iki yerde ayri durum tutmuyoruz.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::data::sessions::{self, Sessions};
use crate::data::settings::{Settings, Theme};
use crate::data::store::Store;
use crate::ui::{
    empty_state, host_dialog, icon_rail, monitor, preferences, sidebar, trust_dialog,
};

pub fn build(
    app: &adw::Application,
    mode: icon_rail::Mode,
    exec: Option<&str>,
    autoconnect: Option<&str>,
    context_menu: Option<&str>,
    edit: Option<&str>,
    action: Option<&str>,
    demo_metrics: bool,
    expand: Option<&str>,
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
    let content_stack = gtk::Stack::new();
    let settings = Rc::new(RefCell::new(Settings::load()));
    crate::apply_user_css(&settings.borrow().appearance.css());
    apply_theme(settings.borrow().appearance.theme);

    // Sistem temasi degisince ya da secim degisince palet takip etsin.
    {
        let manager = adw::StyleManager::default();
        crate::apply_palette(manager.is_dark());
        manager.connect_dark_notify(|manager| crate::apply_palette(manager.is_dark()));
    }

    let tabs = Tabs::new(
        adw::TabView::new(),
        content_stack.clone(),
        Rc::clone(&live),
        Rc::clone(&settings),
    );

    // Ham komut: bos durumun komut alanindan gelir, bir kayda bagli degil.
    let run: Rc<dyn Fn(&str)> = {
        let tabs = Rc::clone(&tabs);
        Rc::new(move |command: &str| tabs.open_or_focus(command, None))
    };

    // Kayitli sunucu icin komutu uretip zaman damgasini gunceller.
    let command_for: Rc<dyn Fn(&str) -> Option<String>> = {
        let store = Rc::clone(&store);
        Rc::new(move |alias: &str| {
            let mut store = store.borrow_mut();
            let host = store.hosts().into_iter().find(|host| host.alias == alias)?;
            store.touch(alias);
            Some(host.command())
        })
    };

    // Baglanmadan once host anahtarini dogrula. ssh'in terminaldeki sorusu
    // askpass yuzunden kullaniciya ulasmiyor; soruyu biz soruyoruz.
    let guarded: Rc<dyn Fn(&str, Rc<dyn Fn()>)> = {
        let store = Rc::clone(&store);
        let window = window.clone();
        Rc::new(move |alias: &str, proceed: Rc<dyn Fn()>| {
            let host = store.borrow().hosts().into_iter().find(|host| host.alias == alias);
            let Some(host) = host else { return };

            trust_dialog::ensure_trusted(
                &window,
                &host.hostname,
                host.port.unwrap_or(22),
                move || proceed(),
            );
        })
    };

    // Her zaman yeni sekme: Ctrl+tik, orta tik ve menudeki "New Session".
    let connect_new: Rc<dyn Fn(&str)> = {
        let command_for = Rc::clone(&command_for);
        let tabs = Rc::clone(&tabs);
        let guarded = Rc::clone(&guarded);
        Rc::new(move |alias: &str| {
            let command_for = Rc::clone(&command_for);
            let tabs = Rc::clone(&tabs);
            let alias = alias.to_string();

            guarded(
                &alias.clone(),
                Rc::new(move || {
                    if let Some(command) = command_for(&alias) {
                        tabs.spawn(&command, Some(&alias));
                        with_refresh(|refresh| refresh());
                    }
                }),
            );
        })
    };

    // Kayitli sunucu: zaman damgasi guncellenir, sonra komut calisir.
    let connect: Rc<dyn Fn(&str)> = {
        let command_for = Rc::clone(&command_for);
        let tabs = Rc::clone(&tabs);
        let guarded = Rc::clone(&guarded);
        let refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let refresh_slot = Rc::clone(&refresh);
        REFRESH.with(|cell| *cell.borrow_mut() = Some(refresh_slot));

        Rc::new(move |alias: &str| {
            // Acik oturum varsa anahtar zaten dogrulanmis; dogrudan odaklan.
            if tabs.focus(alias) {
                return;
            }

            let command_for = Rc::clone(&command_for);
            let tabs = Rc::clone(&tabs);
            let refresh = Rc::clone(&refresh);
            let alias = alias.to_string();

            guarded(
                &alias.clone(),
                Rc::new(move || {
                    let Some(command) = command_for(&alias) else { return };

                    tabs.open_or_focus(&command, Some(&alias));
                    if let Some(refresh) = refresh.borrow().as_ref() {
                        refresh();
                    }
                }),
            );
        })
    };

    let sidebar = sidebar::build(
        Rc::clone(&store),
        Rc::clone(&live),
        Rc::clone(&connect),
        Rc::clone(&connect_new),
    );
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

    let monitor = monitor::build(Rc::clone(&store));

    // Sabitlenmis Monitor sagda kendi sutununda durur; sekmeler solda kalir.
    // Paned, genisligi kullaniciya birakiyor: "ne cok buyuk ne cok kucuk"
    // karari bir kez verilip dondurulmak yerine surukleyerek ayarlanabiliyor.
    let panes = gtk::Paned::new(gtk::Orientation::Horizontal);
    panes.set_start_child(Some(&content_stack));
    panes.set_resize_start_child(true);
    panes.set_shrink_start_child(false);
    panes.set_shrink_end_child(false);
    panes.set_wide_handle(true);

    let split = adw::OverlaySplitView::new();
    split.set_hexpand(true);
    split.set_sidebar_width_fraction(0.22);
    split.set_max_sidebar_width(300.0);
    split.set_min_sidebar_width(220.0);
    split.set_sidebar(Some(&sidebar.widget));

    // Mod secici kenar cubugunun ust barinda: listenin tam ustunde,
    // hangi listeye baktigin soldaki ayri bir raya bakmadan belli oluyor.
    // Mod hem kenar cubugunu hem icerik alanini degistiriyor: Monitor kendi
    // sayfasini aciyor, digerleri oturumlara geri donuyor.
    let current_mode = Rc::new(Cell::new(mode));

    let rail = icon_rail::build(mode, {
        let sidebar = Rc::clone(&sidebar);
        let monitor = Rc::clone(&monitor);
        let stack = content_stack.clone();
        let tabs = Rc::clone(&tabs);
        let current_mode = Rc::clone(&current_mode);
        let panes_for_rail = panes.clone();

        move |mode| {
            sidebar.set_mode(mode);
            current_mode.set(mode);

            if mode == icon_rail::Mode::Monitor {
                // Sabitlenmisse panelde olmali; bir sekilde dusmusse geri koy.
                if monitor.is_pinned() {
                    attach_pinned(&panes_for_rail, &monitor);
                    return;
                }
                tabs.open_monitor(&monitor);
                return;
            }

            stack.set_visible_child_name(if tabs.view.n_pages() == 0 {
                "empty"
            } else {
                "tabs"
            });
        }
    });
    sidebar.header.pack_start(&rail);

    let search = gtk::SearchEntry::new();
    search.add_css_class("global-search");
    search.set_placeholder_text(Some("Search servers, sessions, agents"));

    let header = adw::HeaderBar::new();
    header.add_css_class("runetty");
    header.set_title_widget(Some(&search));

    let app_menu = gio::Menu::new();
    app_menu.append(Some("Preferences"), Some("win.preferences"));
    app_menu.append(Some("New Session"), Some("win.host-new"));
    app_menu.append(Some("New Folder"), Some("win.folder-new"));
    app_menu.append(Some("Reload ~/.ssh/config"), Some("win.reload-ssh-config"));
    app_menu.append(
        Some("Gizlenen ssh config kayitlarini geri getir"),
        Some("win.restore-hidden"),
    );
    app_menu.append(Some("About runetty"), Some("win.about"));
    app_menu.append(Some("Quit"), Some("app.quit"));

    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_tooltip_text(Some("Menu"));
    menu_button.add_css_class("flat");
    menu_button.set_menu_model(Some(&app_menu));

    header.pack_end(&menu_button);
    header.pack_end(&action_button(
        "preferences-system-symbolic",
        "Preferences",
        "win.preferences",
    ));
    header.pack_end(&action_button(
        "view-refresh-symbolic",
        "Reconnect",
        "win.reconnect",
    ));

    content_stack.set_vexpand(true);
    content_stack.add_named(&empty_page(&empty.widget), Some("empty"));
    content_stack.add_named(&tabs_page(&tabs.view), Some("tabs"));
    content_stack.set_visible_child_name("empty");

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&header);
    content_view.set_content(Some(&panes));
    split.set_content(Some(&content_view));

    // Toast'lar icin tek bir katman; geri bildirim buradan geciyor.
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));
    window.set_content(Some(&toasts));

    // Klasor satirindaki + : kayit diyalogu o klasorde acilsin.
    {
        let window = window.clone();
        let store = Rc::clone(&store);
        let refresh = Rc::clone(&refresh);
        sidebar.set_new_in_folder(Rc::new(move |folder: &str| {
            let refresh = Rc::clone(&refresh);
            host_dialog::present_in_folder(
                &window,
                Rc::clone(&store),
                None,
                Some(folder.to_string()),
                move || refresh(),
            );
        }));
    }

    install_actions(Wiring {
        window: &window,
        split: &split,
        search: &search,
        store: &store,
        settings: &settings,
        monitor: &monitor,
        panes: &panes,
        tabs: &tabs,
        command_for: &command_for,
        connect: &connect,
        connect_new: &connect_new,
        refresh: &refresh,
    });

    // Son sekme kapaninca bos duruma geri don.
    {
        let stack = content_stack.clone();
        tabs.view.connect_n_pages_notify(move |view| {
            stack.set_visible_child_name(if view.n_pages() == 0 { "empty" } else { "tabs" });
        });
    }

    // Monitor turu yalnizca kendi sekmesi secilikken calisir: arka plandaki
    // bir sekmenin ag ve CPU yakmasi icin sebep yok.
    {
        let monitor = Rc::clone(&monitor);
        let tabs_for_notify = Rc::clone(&tabs);
        tabs.view.connect_selected_page_notify(move |view| {
            let showing = view
                .selected_page()
                .zip(tabs_for_notify.monitor_page.borrow().clone())
                .is_some_and(|(selected, monitor_page)| selected == monitor_page);

            monitor.set_active(showing);
        });
    }

    // Sabitleme tercihi oturumlar arasinda korunuyor.
    if settings.borrow().layout.monitor_pinned {
        monitor.set_pinned(true);
        let monitor = Rc::clone(&monitor);
        let panes = panes.clone();
        glib::idle_add_local_once(move || attach_pinned(&panes, &monitor));
    } else if mode == icon_rail::Mode::Monitor {
        tabs.open_monitor(&monitor);
    }

    if mode == icon_rail::Mode::Monitor {
        if demo_metrics {
            monitor.demo();
        }

        if let Some(alias) = expand.map(str::to_string) {
            let monitor = Rc::clone(&monitor);
            glib::idle_add_local_once(move || monitor.expand(&alias));
        }
    }

    if let Some(command) = exec {
        tabs.open_or_focus(command, None);
    }

    // Virgulle ayrilmis liste; `+` oneki ek oturum acar. Odaklanma ve
    // coklu oturum davranisini tek komutla dogrulamak icin.
    if let Some(aliases) = autoconnect.map(str::to_string) {
        // Pencere sunulduktan sonra: diyaloglarin bir ust pencereye
        // ihtiyaci var, yoksa sessizce gorunmuyorlar.
        let connect = Rc::clone(&connect);
        let connect_new = Rc::clone(&connect_new);
        glib::idle_add_local_once(move || {
            for alias in aliases.split(',').map(str::trim).filter(|a| !a.is_empty()) {
                match alias.strip_prefix('+') {
                    Some(alias) => connect_new(alias),
                    None => connect(alias),
                }
            }
        });
    }

    // Parametresiz bir pencere eylemini tetikler; dogrulama icin.
    if let Some(name) = action.map(str::to_string) {
        let window = window.clone();
        glib::idle_add_local_once(move || {
            gtk::prelude::WidgetExt::activate_action(
                &window,
                &format!("win.{name}"),
                None,
            )
            .ok();
        });
    }

    if let Some(alias) = edit.map(str::to_string) {
        let window = window.clone();
        let store = Rc::clone(&store);
        let refresh = Rc::clone(&refresh);
        glib::idle_add_local_once(move || {
            let refresh = Rc::clone(&refresh);
            host_dialog::present(&window, store, Some(alias), move || refresh());
        });
    }

    if let Some(alias) = context_menu.map(str::to_string) {
        let sidebar = Rc::clone(&sidebar);
        glib::idle_add_local_once(move || sidebar.popup_menu_for(&alias));
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

/// Secimi libadwaita'ya bildirir; palet degisimi `dark` sinyaliyle geliyor.
pub fn apply_theme(theme: Theme) {
    let scheme = match theme {
        Theme::System => adw::ColorScheme::Default,
        Theme::Light => adw::ColorScheme::ForceLight,
        Theme::Dark => adw::ColorScheme::ForceDark,
    };

    adw::StyleManager::default().set_color_scheme(scheme);
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

/// Sekme defteri. Hangi sunucunun hangi sekmede acik oldugunu bilir; ayni
/// sunucuya tekrar tiklamak yeni sekme acmak yerine acik olana goturuyor.
struct Tabs {
    view: adw::TabView,
    stack: gtk::Stack,
    /// Alias -> acik sekmeleri. Ayni sunucuya birden fazla oturum
    /// acilabiliyor; odaklanma en son acilana gider.
    open: RefCell<HashMap<String, Vec<adw::TabPage>>>,
    /// Monitor da bir sekme; terminal oturumlarinin yanina geliyor,
    /// yerlerini almiyor.
    monitor_page: RefCell<Option<adw::TabPage>>,
    live: Rc<Sessions>,
    settings: Rc<RefCell<Settings>>,
}

impl Tabs {
    fn new(
        view: adw::TabView,
        stack: gtk::Stack,
        live: Rc<Sessions>,
        settings: Rc<RefCell<Settings>>,
    ) -> Rc<Self> {
        Rc::new(Self {
            view,
            stack,
            open: RefCell::new(HashMap::new()),
            monitor_page: RefCell::new(None),
            live,
            settings,
        })
    }

    /// Acik sekmelerin terminal fontunu yeni ayara gore tazeler.
    #[cfg(feature = "terminal")]
    fn restyle(&self) {
        let font = self.settings.borrow().appearance.terminal_pango();
        for index in 0..self.view.n_pages() {
            let page = self.view.nth_page(index);
            crate::ui::terminal::restyle(&page.child(), &font);
        }
    }

    #[cfg(not(feature = "terminal"))]
    fn restyle(&self) {}

    /// Secili sekmeyi kapatip ayni sunucuya yeniden baglanir.
    fn reconnect(self: &Rc<Self>, command_for: &Rc<dyn Fn(&str) -> Option<String>>) -> bool {
        let Some(page) = self.view.selected_page() else { return false };

        let alias = self
            .open
            .borrow()
            .iter()
            .find(|(_, pages)| pages.contains(&page))
            .map(|(alias, _)| alias.clone());

        let Some(alias) = alias else { return false };
        let Some(command) = command_for(&alias) else { return false };

        self.view.close_page(&page);
        self.spawn(&command, Some(&alias));
        true
    }

    fn mark(&self, alias: &str, state: sessions::State) {
        self.live.set(alias, state);
        with_refresh(|refresh| refresh());
    }

    /// Sabitlenirken sekmeyi kapatir; widget serbest kalsin ki panele
    /// yeniden eklenebilsin.
    fn close_monitor(&self) {
        let page = self.monitor_page.borrow_mut().take();
        if let Some(page) = page {
            self.view.close_page(&page);
        }
    }

    /// Monitor sekmesini acar ya da acik olana gecer.
    fn open_monitor(self: &Rc<Self>, monitor: &Rc<monitor::Monitor>) {
        if let Some(page) = self.monitor_page.borrow().clone() {
            self.view.set_selected_page(&page);
            self.stack.set_visible_child_name("tabs");
            return;
        }

        let page = self.view.append(&monitor.widget);
        page.set_title("Monitor");

        // Defter secimden once yazilmali; ustelik ilk sayfa eklenirken
        // TabView onu kendiliginden seciyor, yani secim sinyaline guvenip
        // turu baslatmak yetmiyor — burada acikca baslatiyoruz.
        *self.monitor_page.borrow_mut() = Some(page.clone());

        self.view.set_selected_page(&page);
        self.stack.set_visible_child_name("tabs");
        monitor.set_active(true);

        let this = Rc::clone(self);
        let monitor = Rc::clone(monitor);
        let closed = page;
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> =
            Rc::new(RefCell::new(None));
        let slot = Rc::clone(&handler);

        *handler.borrow_mut() = Some(self.view.connect_page_detached(move |view, page, _| {
            if page != &closed {
                return;
            }
            // Sabitlenerek sekmeden cikariliyorsa tur devam etmeli.
            if !monitor.is_pinned() {
                monitor.set_active(false);
            }
            *this.monitor_page.borrow_mut() = None;
            if let Some(id) = slot.borrow_mut().take() {
                view.disconnect(id);
            }
        }));
    }

    /// Acik oturum varsa ona gecer ve `true` doner.
    fn focus(&self, alias: &str) -> bool {
        let existing = self.open.borrow().get(alias).and_then(|p| p.last().cloned());
        let Some(page) = existing else { return false };

        self.view.set_selected_page(&page);
        self.stack.set_visible_child_name("tabs");
        true
    }

    /// Acik oturum varsa ona gecer; yoksa yenisini acar.
    fn open_or_focus(self: &Rc<Self>, command: &str, alias: Option<&str>) {
        if let Some(alias) = alias {
            if self.focus(alias) {
                return;
            }
        }

        self.spawn(command, alias);
    }

    fn spawn(self: &Rc<Self>, command: &str, alias: Option<&str>) {
        let child = self.terminal(command, alias);
        let page = self.view.append(&child);
        page.set_title(&tab_title(command));
        self.view.set_selected_page(&page);
        self.stack.set_visible_child_name("tabs");

        let Some(alias) = alias else { return };

        self.open.borrow_mut().entry(alias.to_string()).or_default().push(page.clone());
        self.mark(alias, sessions::State::Open);

        // Sekme kapaninca kayit defterden dusuyor ve nokta kirmiziya doner.
        // Sinyal bu sayfaya ozel, bu yuzden ilk tetiklenmede kendini soker.
        let this = Rc::clone(self);
        let alias = alias.to_string();
        let closed = page;
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> =
            Rc::new(RefCell::new(None));
        let slot = Rc::clone(&handler);

        *handler.borrow_mut() = Some(self.view.connect_page_detached(move |view, page, _| {
            if page != &closed {
                return;
            }
            let empty = {
                let mut open = this.open.borrow_mut();
                let remaining = open.entry(alias.clone()).or_default();
                remaining.retain(|candidate| candidate != &closed);
                let empty = remaining.is_empty();
                if empty {
                    open.remove(&alias);
                }
                empty
            };

            // Baska sekmesi duruyorsa sunucu hala bagli sayilir.
            if empty {
                this.mark(&alias, sessions::State::Closed);
            }
            if let Some(id) = slot.borrow_mut().take() {
                view.disconnect(id);
            }
        }));
    }

    #[cfg(feature = "terminal")]
    fn terminal(self: &Rc<Self>, command: &str, alias: Option<&str>) -> gtk::Widget {
        let owned = alias.map(str::to_string);
        let this = Rc::clone(self);

        let font = self.settings.borrow().appearance.terminal_pango();

        crate::ui::terminal::page(Some(command), alias, &font, move |status| {
            if let Some(alias) = &owned {
                // Sekme hala acik ama komut bitti: baglanti dustu.
                this.mark(
                    alias,
                    if status == 0 {
                        sessions::State::Closed
                    } else {
                        sessions::State::Trouble
                    },
                );
            }
        })
    }

    #[cfg(not(feature = "terminal"))]
    fn terminal(self: &Rc<Self>, command: &str, _alias: Option<&str>) -> gtk::Widget {
        let label = gtk::Label::new(Some(&format!(
            "Terminal destegi bu derlemede kapali.\n{command}"
        )));
        label.add_css_class("dim");
        label.set_vexpand(true);
        label.upcast()
    }
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

fn action_button(icon: &str, tooltip: &str, action: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.set_action_name(Some(action));
    button
}

/// Eylem kurulumunun ihtiyaci olan parcalar. Ayri ayri parametre olarak
/// tasimak imzayi okunmaz hale getiriyordu.
struct Wiring<'a> {
    window: &'a adw::ApplicationWindow,
    split: &'a adw::OverlaySplitView,
    search: &'a gtk::SearchEntry,
    store: &'a Rc<RefCell<Store>>,
    settings: &'a Rc<RefCell<Settings>>,
    monitor: &'a Rc<monitor::Monitor>,
    panes: &'a gtk::Paned,
    tabs: &'a Rc<Tabs>,
    command_for: &'a Rc<dyn Fn(&str) -> Option<String>>,
    connect: &'a Rc<dyn Fn(&str)>,
    connect_new: &'a Rc<dyn Fn(&str)>,
    refresh: &'a Rc<dyn Fn()>,
}

fn install_actions(w: Wiring) {
    let window = w.window;

    let add = |action: &gio::SimpleAction| window.add_action(action);

    let focus_search = gio::SimpleAction::new("global-search", None);
    {
        let search = w.search.clone();
        focus_search.connect_activate(move |_, _| {
            search.grab_focus();
        });
    }
    add(&focus_search);

    let toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
    {
        let split = w.split.clone();
        toggle_sidebar.connect_activate(move |_, _| {
            split.set_show_sidebar(!split.shows_sidebar());
        });
    }
    add(&toggle_sidebar);

    let string = Some(glib::VariantTy::STRING);

    // Sag tik menusu: alias eylemin hedefi olarak geliyor.
    let host_connect = gio::SimpleAction::new("host-connect", string);
    {
        let connect = Rc::clone(w.connect);
        host_connect.connect_activate(move |_, target| {
            if let Some(alias) = target.and_then(glib::Variant::str) {
                connect(alias);
            }
        });
    }
    add(&host_connect);

    let host_connect_new = gio::SimpleAction::new("host-connect-new", string);
    {
        let connect_new = Rc::clone(w.connect_new);
        host_connect_new.connect_activate(move |_, target| {
            if let Some(alias) = target.and_then(glib::Variant::str) {
                connect_new(alias);
            }
        });
    }
    add(&host_connect_new);

    let toggle_monitor = gio::SimpleAction::new("toggle-monitor", string);
    {
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        let monitor = Rc::clone(w.monitor);
        let window = window.clone();
        toggle_monitor.connect_activate(move |_, target| {
            let Some(alias) = target.and_then(glib::Variant::str) else { return };

            let monitored = !store.borrow().is_monitored(alias);
            store.borrow_mut().set_monitored(alias, monitored);
            refresh();
            monitor.rebuild_now();

            toast(
                &window,
                &if monitored {
                    format!("\"{alias}\" Monitor'e eklendi")
                } else {
                    format!("\"{alias}\" Monitor'den cikarildi")
                },
            );
        });
    }
    add(&toggle_monitor);

    let host_edit = gio::SimpleAction::new("host-edit", string);
    {
        let window = window.clone();
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        host_edit.connect_activate(move |_, target| {
            let alias = target.and_then(glib::Variant::str).map(str::to_string);
            let refresh = Rc::clone(&refresh);
            host_dialog::present(&window, Rc::clone(&store), alias, move || refresh());
        });
    }
    add(&host_edit);

    let host_new = gio::SimpleAction::new("host-new", None);
    {
        let window = window.clone();
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        host_new.connect_activate(move |_, _| {
            let refresh = Rc::clone(&refresh);
            host_dialog::present(&window, Rc::clone(&store), None, move || refresh());
        });
    }
    add(&host_new);

    let folder_new = gio::SimpleAction::new("folder-new", None);
    {
        let window = window.clone();
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        folder_new.connect_activate(move |_, _| {
            new_folder(&window, Rc::clone(&store), Rc::clone(&refresh));
        });
    }
    add(&folder_new);

    let host_delete = gio::SimpleAction::new("host-delete", string);
    {
        let window = window.clone();
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        host_delete.connect_activate(move |_, target| {
            let Some(alias) = target.and_then(glib::Variant::str).map(str::to_string) else {
                return;
            };
            confirm_delete(&window, Rc::clone(&store), alias, Rc::clone(&refresh));
        });
    }
    add(&host_delete);

    let reconnect = gio::SimpleAction::new("reconnect", None);
    {
        let tabs = Rc::clone(w.tabs);
        let command_for = Rc::clone(w.command_for);
        let window = window.clone();
        reconnect.connect_activate(move |_, _| {
            if !tabs.reconnect(&command_for) {
                toast(&window, "Yeniden baglanacak bir oturum yok");
            }
        });
    }
    add(&reconnect);

    let prefs = gio::SimpleAction::new("preferences", None);
    {
        let window = window.clone();
        let settings = Rc::clone(w.settings);
        let tabs = Rc::clone(w.tabs);
        prefs.connect_activate(move |_, _| {
            let settings_for_apply = Rc::clone(&settings);
            let tabs = Rc::clone(&tabs);
            preferences::present(&window, Rc::clone(&settings), move || {
                crate::apply_user_css(&settings_for_apply.borrow().appearance.css());
                tabs.restyle();
            });
        });
    }
    add(&prefs);

    let reload = gio::SimpleAction::new("reload-ssh-config", None);
    {
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        reload.connect_activate(move |_, _| {
            store.borrow_mut().reload_ssh_config();
            refresh();
        });
    }
    add(&reload);

    let restore = gio::SimpleAction::new("restore-hidden", None);
    {
        let store = Rc::clone(w.store);
        let refresh = Rc::clone(w.refresh);
        let window = window.clone();
        restore.connect_activate(move |_, _| {
            let count = store.borrow().hidden_count();
            if count == 0 {
                toast(&window, "Gizlenmis kayit yok");
                return;
            }

            store.borrow_mut().unhide_all();
            refresh();
            toast(&window, &format!("{count} kayit geri getirildi"));
        });
    }
    add(&restore);

    let about = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        about.connect_activate(move |_, _| show_about(&window));
    }
    add(&about);

    let toggle_pin = gio::SimpleAction::new("toggle-monitor-pin", None);
    {
        let monitor = Rc::clone(w.monitor);
        let tabs = Rc::clone(w.tabs);
        let panes = w.panes.clone();
        let settings_for_pin = Rc::clone(w.settings);
        toggle_pin.connect_activate(move |_, _| {
            if monitor.is_pinned() {
                monitor.set_pinned(false);
                panes.set_end_child(gtk::Widget::NONE);
                tabs.open_monitor(&monitor);

                let mut settings = settings_for_pin.borrow_mut();
                settings.layout.monitor_pinned = false;
                settings.save();
                return;
            }

            {
                let mut settings = settings_for_pin.borrow_mut();
                settings.layout.monitor_pinned = true;
                settings.save();
            }

            // Sira onemli: once sabit isaretle, sonra sekmeyi kapat.
            // page-detached isleyicisi sabitlenmis Monitor'u durdurmasin.
            monitor.set_pinned(true);
            tabs.close_monitor();

            // close_page widget'i hemen serbest birakmiyor; ayni karede
            // panele eklemek "zaten bir ebeveyni var" hatasina dusuyor ve
            // Monitor ekrandan tamamen kayboluyordu.
            let monitor = Rc::clone(&monitor);
            let panes = panes.clone();
            glib::idle_add_local_once(move || {
                attach_pinned(&panes, &monitor);
            });
        });
    }
    add(&toggle_pin);

    let palette = gio::SimpleAction::new("command-palette", None);
    palette.connect_activate(|_, _| glib::g_debug!("runetty", "TODO: command palette"));
    add(&palette);
}

/// Sabitlenmis Monitor'u sag sutuna yerlestirir. Idempotent: zaten
/// oradaysa dokunmuyor, baska bir ebeveyni varsa once koparıyor.
fn attach_pinned(panes: &gtk::Paned, monitor: &Rc<monitor::Monitor>) {
    if panes.end_child().as_ref() == Some(&monitor.widget) {
        monitor.set_active(true);
        return;
    }

    if let Some(parent) = monitor.widget.parent() {
        if let Some(view) = parent.downcast_ref::<adw::TabView>() {
            if let Some(page) = view.page(&monitor.widget).into() {
                let page: adw::TabPage = page;
                view.close_page(&page);
            }
        }
    }

    // Hala bagliysa bu karede ekleyemeyiz; bir tur sonra tekrar dene.
    if monitor.widget.parent().is_some() {
        let panes = panes.clone();
        let monitor = Rc::clone(monitor);
        glib::idle_add_local_once(move || attach_pinned(&panes, &monitor));
        return;
    }

    panes.set_end_child(Some(&monitor.widget));

    // Varsayilan genislik: kartlar sigacak kadar genis, icerigi bogmayacak
    // kadar dar. Kullanici tutamaktan surukleyerek degistirebiliyor.
    let width = panes.width();
    if width > 0 {
        panes.set_position(if width > 900 { width - 420 } else { width / 2 });
    }

    monitor.set_active(true);
}

fn show_about(window: &adw::ApplicationWindow) {
    let about = adw::AboutDialog::new();
    about.set_application_name("runetty");
    about.set_application_icon(crate::config::APP_ID);
    about.set_version(crate::config::VERSION);
    about.set_developer_name("Umutcan Purtul");
    about.set_developers(&[
        "Umutcan Purtul <purtulumut97@gmail.com>",
        "Claude — Anthropic",
    ]);
    about.set_copyright("© 2026 Umutcan Purtul");
    about.set_license_type(gtk::License::MitX11);
    about.set_website("https://github.com/umutcanpurtul-avi/runetty");
    about.set_comments(
        "SSH oturumlari, agent'lar ve sunucu izleme icin tek pencere.\n\n\
         Iletisim: purtulumut97@gmail.com",
    );

    about.present(Some(window));
}

fn toast(window: &adw::ApplicationWindow, text: &str) {
    match window.content().and_downcast::<adw::ToastOverlay>() {
        Some(overlay) => overlay.add_toast(adw::Toast::new(text)),
        None => eprintln!("runetty: {text}"),
    }
}

/// Bos klasor olusturur; icine sunucu, kayit diyalogundan secilerek girer.
fn new_folder(
    window: &adw::ApplicationWindow,
    store: Rc<RefCell<Store>>,
    refresh: Rc<dyn Fn()>,
) {
    let entry = adw::EntryRow::new();
    entry.set_title("Folder name");

    let group = adw::PreferencesGroup::new();
    group.add(&entry);

    let dialog = adw::AlertDialog::new(Some("New Folder"), None);
    dialog.set_extra_child(Some(&group));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |dialog, response| {
        if response != "create" {
            return;
        }

        let name = entry.text().trim().to_string();
        if name.is_empty() {
            dialog.set_body("Klasor adi bos birakilamaz.");
            return;
        }

        store.borrow_mut().add_folder(&name);
        refresh();
    });

    dialog.present(Some(window));
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

    let parent = window.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }

        let deleted = store.borrow_mut().remove(&alias);
        refresh();

        // Gizlenen kayit geri alinabilir; kullanici bunu menuden aramasin.
        if deleted {
            toast(&parent, &format!("\"{alias}\" silindi"));
            return;
        }

        let toast = adw::Toast::new(&format!("\"{alias}\" listeden gizlendi"));
        toast.set_button_label(Some("Geri al"));
        toast.set_action_name(Some("win.restore-hidden"));
        toast.set_timeout(6);

        if let Some(overlay) = parent.content().and_downcast::<adw::ToastOverlay>() {
            overlay.add_toast(toast);
        }
    });

    dialog.present(Some(window));
}
