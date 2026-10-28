//! Window skeleton: icon rail, OverlaySplitView and tabbed content.
//! Data changes reach both the sidebar and the empty state through a single
//! `refresh`; we do not keep two separate copies of the state.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::data::sessions::{self, Sessions};
use crate::data::settings::{Settings, Theme};
use crate::data::store::Store;
use crate::ui::{
    agents, empty_state, host_dialog, icon_rail, monitor, preferences, sidebar,
    trust_dialog,
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

    // Keep the palette in step with the system theme and with the choice.
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

    // A raw command, typed in the empty state's field and tied to no record.
    let run: Rc<dyn Fn(&str)> = {
        let tabs = Rc::clone(&tabs);
        Rc::new(move |command: &str| tabs.open_or_focus(command, None))
    };

    // Builds the command for a stored host and updates its timestamp.
    let command_for: Rc<dyn Fn(&str) -> Option<String>> = {
        let store = Rc::clone(&store);
        Rc::new(move |alias: &str| {
            let mut store = store.borrow_mut();
            let host = store.hosts().into_iter().find(|host| host.alias == alias)?;
            store.touch(alias);
            Some(host.command())
        })
    };

    // Verify the host key before connecting. ssh's terminal prompt never
    // reaches the user because of askpass, so we ask instead.
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

    // Always a new tab: Ctrl-click, middle-click and "New Session" in the menu.
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

    // Stored host: the timestamp is updated, then the command runs.
    let connect: Rc<dyn Fn(&str)> = {
        let command_for = Rc::clone(&command_for);
        let tabs = Rc::clone(&tabs);
        let guarded = Rc::clone(&guarded);
        let refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let refresh_slot = Rc::clone(&refresh);
        REFRESH.with(|cell| *cell.borrow_mut() = Some(refresh_slot));

        Rc::new(move |alias: &str| {
            // An open session means the key is already verified; just focus it.
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

    // Attaching to a tmux session is just another command in a tab.
    let agents = agents::build(Rc::clone(&store), Rc::clone(&run));

    let sidebar = sidebar::build(
        Rc::clone(&store),
        Rc::clone(&live),
        &agents.widget,
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

    // A pinned Monitor sits in its own right-hand column; tabs stay on the
    // left. Paned leaves the width to the user: rather than freezing one
    // "not too wide, not too narrow" number, it can be dragged.
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

    // The mode switcher lives in the sidebar's top bar, directly above the
    // list, so which list you are looking at is clear without a separate rail.
    // The mode drives both the sidebar and the content area: Monitor opens its
    // own page, the others return to sessions.
    let current_mode = Rc::new(Cell::new(mode));

    let rail = icon_rail::build(mode, {
        let sidebar = Rc::clone(&sidebar);
        let monitor = Rc::clone(&monitor);
        let agents = Rc::clone(&agents);
        let stack = content_stack.clone();
        let tabs = Rc::clone(&tabs);
        let current_mode = Rc::clone(&current_mode);
        let panes_for_rail = panes.clone();

        move |mode| {
            sidebar.set_mode(mode);
            current_mode.set(mode);
            agents.set_active(mode == icon_rail::Mode::Agents);

            if mode == icon_rail::Mode::Monitor {
                // If pinned it belongs in the pane; put it back if it fell out.
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
        Some("Restore hidden ssh config records"),
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

    // One layer for toasts; all feedback passes through here.
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));
    window.set_content(Some(&toasts));

    // The + on a folder row opens the record dialog inside that folder.
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

    // Return to the empty state when the last tab closes.
    {
        let stack = content_stack.clone();
        tabs.view.connect_n_pages_notify(move |view| {
            stack.set_visible_child_name(if view.n_pages() == 0 { "empty" } else { "tabs" });
        });
    }

    // Monitor only runs while its own tab is selected: a background tab has no
    // reason to burn network and CPU.
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

    // The pin preference is kept across sessions.
    if mode == icon_rail::Mode::Agents {
        agents.set_active(true);
    }

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

    // Comma-separated list; a `+` prefix opens an extra session. Lets focus
    // and multi-session behaviour be verified in one command.
    if let Some(aliases) = autoconnect.map(str::to_string) {
        // After the window is presented: dialogs need a parent window or
        // they silently fail to appear.
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

    // Triggers a parameterless window action, for verification.
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

// `connect` and `refresh` depend on each other and one is built first.
// A single late-binding slot unties the knot.
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

/// Tells libadwaita the choice; the palette change arrives via the `dark` signal.
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

/// Tab ledger. It knows which host is open in which tab, so clicking a host
/// again focuses the open tab instead of opening another.
struct Tabs {
    view: adw::TabView,
    stack: gtk::Stack,
    /// Alias to its open tabs. A host can have several sessions; focusing
    /// goes to the most recently opened one.
    open: RefCell<HashMap<String, Vec<adw::TabPage>>>,
    /// Monitor is a tab too: it joins the terminal sessions rather than
    /// replacing them.
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

    /// Refreshes the terminal font of open tabs to match the new setting.
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

    /// Closes the selected tab and reconnects to the same host.
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

    /// Closes the tab when pinning, freeing the widget so it can be added to
    /// the pane.
    fn close_monitor(&self) {
        let page = self.monitor_page.borrow_mut().take();
        if let Some(page) = page {
            self.view.close_page(&page);
        }
    }

    /// Opens the Monitor tab, or switches to it if already open.
    fn open_monitor(self: &Rc<Self>, monitor: &Rc<monitor::Monitor>) {
        if let Some(page) = self.monitor_page.borrow().clone() {
            self.view.set_selected_page(&page);
            self.stack.set_visible_child_name("tabs");
            return;
        }

        let page = self.view.append(&monitor.widget);
        page.set_title("Monitor");

        // The ledger must be written before selection; moreover TabView selects
        // the first page on its own, so relying on the selection signal to start
        // rounds is not enough - we start them explicitly here.
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
            // If it is leaving the tab to be pinned, rounds should continue.
            if !monitor.is_pinned() {
                monitor.set_active(false);
            }
            *this.monitor_page.borrow_mut() = None;
            if let Some(id) = slot.borrow_mut().take() {
                view.disconnect(id);
            }
        }));
    }

    /// Focuses an open session if there is one and returns `true`.
    fn focus(&self, alias: &str) -> bool {
        let existing = self.open.borrow().get(alias).and_then(|p| p.last().cloned());
        let Some(page) = existing else { return false };

        self.view.set_selected_page(&page);
        self.stack.set_visible_child_name("tabs");
        true
    }

    /// Focuses an open session, or opens a new one.
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

        // Closing the tab drops the record from the ledger and turns the dot
        // red. The signal is specific to this page, so it disconnects itself
        // the first time it fires.
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

            // With another tab still open the host counts as connected.
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
                // The tab is still open but the command ended: connection dropped.
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
            "Terminal support is disabled in this build.\n{command}"
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

/// The pieces action setup needs. Passing them as separate parameters made
/// the signature unreadable.
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

    // Context menu: the alias arrives as the action's target.
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
                    format!("\"{alias}\" added to Monitor")
                } else {
                    format!("\"{alias}\" removed from Monitor")
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
                toast(&window, "No session to reconnect");
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
                toast(&window, "No hidden records");
                return;
            }

            store.borrow_mut().unhide_all();
            refresh();
            toast(&window, &format!("{count} records restored"));
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

            // Order matters: mark it pinned first, then close the tab, so the
            // page-detached handler does not stop a pinned Monitor.
            monitor.set_pinned(true);
            tabs.close_monitor();

            // close_page does not free the widget in the same frame; adding it
            // to the pane right away hits "already has a parent" and Monitor
            // vanished from the screen entirely.
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

/// Places a pinned Monitor in the right-hand column. Idempotent: it leaves
/// things alone if already there, and detaches any other parent first.
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

    // Still attached, so we cannot add it this frame; retry next tick.
    if monitor.widget.parent().is_some() {
        let panes = panes.clone();
        let monitor = Rc::clone(monitor);
        glib::idle_add_local_once(move || attach_pinned(&panes, &monitor));
        return;
    }

    panes.set_end_child(Some(&monitor.widget));

    // Default width: wide enough for the cards, narrow enough not to crowd the
    // content. The user can drag the handle to change it.
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
        "One window for SSH sessions, agents and host monitoring.\n\n\
         Contact: purtulumut97@gmail.com",
    );

    about.present(Some(window));
}

fn toast(window: &adw::ApplicationWindow, text: &str) {
    match window.content().and_downcast::<adw::ToastOverlay>() {
        Some(overlay) => overlay.add_toast(adw::Toast::new(text)),
        None => eprintln!("runetty: {text}"),
    }
}

/// Creates an empty folder; hosts move in by choosing it in the record dialog.
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
            dialog.set_body("The folder name cannot be empty.");
            return;
        }

        store.borrow_mut().add_folder(&name);
        refresh();
    });

    dialog.present(Some(window));
}

/// Deletion cannot be undone, so it needs confirmation; for an ssh config
/// record we also say plainly what happens - we do not touch the user's file.
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
            "\"{alias}\" is defined in ~/.ssh/config. runetty never writes to that file; \
             the record is only hidden and can be restored from the + menu."
        ),
        _ => format!("Delete the record \"{alias}\"? This cannot be undone."),
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

        // A hidden record can be restored; the user should not have to hunt
        // for that in a menu.
        if deleted {
            toast(&parent, &format!("\"{alias}\" deleted"));
            return;
        }

        let toast = adw::Toast::new(&format!("\"{alias}\" hidden from the list"));
        toast.set_button_label(Some("Undo"));
        toast.set_action_name(Some("win.restore-hidden"));
        toast.set_timeout(6);

        if let Some(overlay) = parent.content().and_downcast::<adw::ToastOverlay>() {
            overlay.add_toast(toast);
        }
    });

    dialog.present(Some(window));
}
