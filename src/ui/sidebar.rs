//! Sidebar: a list that follows the active mode, with a fixed tool bar below.
//! Built from `crate::data::store::Store` and refreshed on every change.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use super::icon_rail::Mode;
use super::model::AGENTS;
use super::rows;
use crate::data::sessions::Sessions;
use crate::data::store::Store;
use crate::data::Host;

pub struct Sidebar {
    pub widget: adw::ToolbarView,
    /// The sidebar's top bar. Window actions live here: it uses the empty
    /// space in the header and leaves the search field alone in the centre.
    pub header: adw::HeaderBar,
    stack: gtk::Stack,
    sessions: gtk::Box,
    /// One context menu shared by all rows. Attaching a popover per row
    /// leaked orphaned widgets on every refresh.
    menu: gtk::PopoverMenu,
    store: Rc<RefCell<Store>>,
    live: Rc<Sessions>,
    /// The + button on a folder row opens the new-record dialog from here.
    new_in_folder: RefCell<Option<Rc<dyn Fn(&str)>>>,
    connect: Rc<dyn Fn(&str)>,
    connect_new: Rc<dyn Fn(&str)>,
    filter: RefCell<String>,
}

impl Sidebar {
    /// Wired after the window exists: the dialog needs a parent window.
    pub fn set_new_in_folder(&self, callback: Rc<dyn Fn(&str)>) {
        *self.new_in_folder.borrow_mut() = Some(callback);
    }

    pub fn set_mode(&self, mode: Mode) {
        // Monitor's cards live in the content area; the sidebar shows the same
        // host list, because the mode buttons sit there too.
        let page = match mode {
            Mode::Monitor => Mode::Sessions.id(),
            other => other.id(),
        };
        self.stack.set_visible_child_name(page);
    }

    /// Opens the context menu programmatically, for `--context-menu <alias>`.
    pub fn popup_menu_for(&self, alias: &str) {
        let monitored = self.store.borrow().is_monitored(alias);
        self.menu.set_menu_model(Some(&context_menu(alias, monitored)));
        self.menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(40, 70, 1, 1)));
        self.menu.popup();
    }

    /// The list is rebuilt wholesale. While record counts stay in the hundreds
    /// this is simpler than incremental updates and quite enough.
    pub fn refresh(self: &Rc<Self>) {
        while let Some(child) = self.sessions.first_child() {
            self.sessions.remove(&child);
        }

        let store = self.store.borrow();
        let needle = self.filter.borrow().to_lowercase();
        let matches = |host: &Host| {
            needle.is_empty()
                || host.alias.to_lowercase().contains(&needle)
                || host.user_host().to_lowercase().contains(&needle)
        };

        let recents: Vec<Host> =
            store.recents(3).into_iter().filter(|host| matches(host)).collect();
        if !recents.is_empty() {
            self.sessions.append(&rows::section_header("Recents"));
            self.sessions.append(&self.host_list(&recents));
        }

        let all: Vec<Host> = store.hosts().into_iter().filter(|host| matches(host)).collect();
        let any_folder = !store.folders().is_empty();
        drop(store);

        let servers = rows::section_header("Servers");
        // Dropping on the header ungroups the host.
        attach_drop_target(&servers, None, self);
        self.sessions.append(&servers);

        if all.is_empty() && !any_folder {
            self.sessions.append(&empty_hint(&needle));
            return;
        }

        // Empty folders stay visible so a folder can be created first and
        // filled afterwards.
        let folders = if needle.is_empty() {
            self.store.borrow().folders()
        } else {
            let mut used: Vec<String> =
                all.iter().filter_map(|host| host.folder.clone()).collect();
            used.sort();
            used.dedup();
            used
        };

        for folder in folders {
            let inside: Vec<Host> = all
                .iter()
                .filter(|host| host.folder.as_deref() == Some(folder.as_str()))
                .cloned()
                .collect();

            let expander = gtk::Expander::new(None);
            let header = folder_header(&folder, self);
            expander.set_label_widget(Some(&header));
            attach_drop_target(&expander, Some(&folder), self);
            expander.set_expanded(true);
            expander.set_margin_start(6);
            expander.set_child(Some(&if inside.is_empty() {
                empty_folder_hint().upcast::<gtk::Widget>()
            } else {
                self.host_list(&inside).upcast::<gtk::Widget>()
            }));
            self.sessions.append(&expander);
        }

        let loose: Vec<Host> =
            all.iter().filter(|host| host.folder.is_none()).cloned().collect();
        if !loose.is_empty() {
            self.sessions.append(&self.host_list(&loose));
        }
    }

    fn host_list(self: &Rc<Self>, hosts: &[Host]) -> gtk::ListBox {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);

        for host in hosts {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&rows::host_row(host, self.live.state(&host.alias))));
            attach_drag_source(&row, &host.alias);
            list.append(&row);
        }

        let hosts = hosts.to_vec();

        {
            let hosts = hosts.clone();
            let this = Rc::clone(self);
            list.connect_row_activated(move |_, row| {
                if let Some(host) = hosts.get(row.index() as usize) {
                    (this.connect)(&host.alias);
                }
            });
        }

        // Ctrl-click and middle-click open an extra session to the same host.
        // A plain click focuses the existing tab, so a second session needs
        // its own route.
        {
            let hosts = hosts.clone();
            let this = Rc::clone(self);
            let extra = gtk::GestureClick::new();
            extra.set_button(0);
            extra.connect_pressed(move |gesture, _, _, y| {
                let control = gesture
                    .current_event_state()
                    .contains(gtk::gdk::ModifierType::CONTROL_MASK);
                let middle = gesture.current_button() == gtk::gdk::BUTTON_MIDDLE;
                if !control && !middle {
                    return;
                }

                let Some(list) = gesture.widget().and_downcast::<gtk::ListBox>() else {
                    return;
                };
                let Some(row) = list.row_at_y(y as i32) else { return };
                let Some(host) = hosts.get(row.index() as usize) else { return };

                gesture.set_state(gtk::EventSequenceState::Claimed);
                (this.connect_new)(&host.alias);
            });
            list.add_controller(extra);
        }

        let this = Rc::clone(self);
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        gesture.connect_pressed(move |gesture, _, x, y| {
            let Some(list) = gesture.widget().and_downcast::<gtk::ListBox>() else {
                return;
            };
            let Some(row) = list.row_at_y(y as i32) else { return };
            let Some(host) = hosts.get(row.index() as usize) else { return };

            let monitored = this.store.borrow().is_monitored(&host.alias);
            this.menu.set_menu_model(Some(&context_menu(&host.alias, monitored)));

            // Translate the click point into the space the menu is anchored in.
            let point = list
                .compute_point(&this.stack, &gtk::graphene::Point::new(x as f32, y as f32))
                .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));

            this.menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                point.x() as i32,
                point.y() as i32,
                1,
                1,
            )));
            this.menu.popup();
        });
        list.add_controller(gesture);

        list
    }
}

/// Actions carry the alias as their target, so one window-level action
/// serves every row.
fn context_menu(alias: &str, monitored: bool) -> gio::Menu {
    let monitor_label = if monitored {
        "Remove from Monitor"
    } else {
        "Add to Monitor"
    };

    let menu = gio::Menu::new();
    for (label, action) in [
        ("Connect", "win.host-connect"),
        ("New Session", "win.host-connect-new"),
        (monitor_label, "win.toggle-monitor"),
        ("Edit", "win.host-edit"),
        ("Delete", "win.host-delete"),
    ] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some(action), Some(&alias.to_variant()));
        menu.append_item(&item);
    }
    menu
}

/// Row drag source: what moves is the alias, the target folder decides where.
fn attach_drag_source(row: &gtk::ListBoxRow, alias: &str) {
    let source = gtk::DragSource::new();
    source.set_actions(gtk::gdk::DragAction::MOVE);

    let alias = alias.to_string();
    source.connect_prepare(move |_, _, _| {
        Some(gtk::gdk::ContentProvider::for_value(&alias.to_value()))
    });

    // The drag image is the row itself. We do not hold the row in the closure:
    // the row owns the controller, and the reverse would make a reference cycle.
    source.connect_drag_begin(|source, _| {
        if let Some(widget) = source.widget() {
            let paintable = gtk::WidgetPaintable::new(Some(&widget));
            source.set_icon(Some(&paintable), 0, 0);
        }
    });

    row.add_controller(source);
}

/// Drop target for a folder header or the "Servers" header.
/// A `folder` of None ungroups the host.
fn attach_drop_target(
    widget: &impl IsA<gtk::Widget>,
    folder: Option<&str>,
    sidebar: &Rc<Sidebar>,
) {
    let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);

    let folder = folder.map(str::to_string);
    let sidebar = Rc::clone(sidebar);
    target.connect_drop(move |target, value, _, _| {
        let Ok(alias) = value.get::<String>() else {
            return false;
        };

        if let Some(widget) = target.widget() {
            widget.remove_css_class("drop-target");
        }

        sidebar.store.borrow_mut().set_folder(&alias, folder.as_deref());
        sidebar.refresh();
        true
    });

    target.connect_enter(|target, _, _| {
        if let Some(widget) = target.widget() {
            widget.add_css_class("drop-target");
        }
        gtk::gdk::DragAction::MOVE
    });

    target.connect_leave(|target| {
        if let Some(widget) = target.widget() {
            widget.remove_css_class("drop-target");
        }
    });

    widget.as_ref().add_controller(target);
}

/// Folder row: the name, plus rename and "new host in this folder".
fn folder_header(folder: &str, sidebar: &Rc<Sidebar>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.add_css_class("folder-header");

    let label = gtk::Label::new(Some(folder));
    label.add_css_class("folder-label");
    label.set_hexpand(true);
    label.set_xalign(0.0);
    row.append(&label);

    let rename = folder_button("document-edit-symbolic", "Rename folder");
    {
        let sidebar = Rc::clone(sidebar);
        let folder = folder.to_string();
        rename.connect_clicked(move |_| rename_folder(&sidebar, &folder));
    }
    row.append(&rename);

    let add = folder_button("list-add-symbolic", "New host in this folder");
    {
        let sidebar = Rc::clone(sidebar);
        let folder = folder.to_string();
        add.connect_clicked(move |_| {
            let callback = sidebar.new_in_folder.borrow().clone();
            if let Some(callback) = callback {
                callback(&folder);
            }
        });
    }
    row.append(&add);

    attach_folder_menu(&label, folder, sidebar);
    row
}

fn folder_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("folder-action");
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button
}

fn rename_folder(sidebar: &Rc<Sidebar>, folder: &str) {
    let entry = adw::EntryRow::new();
    entry.set_title("Folder name");
    entry.set_text(folder);

    let group = adw::PreferencesGroup::new();
    group.add(&entry);

    let dialog = adw::AlertDialog::new(Some("Rename folder"), None);
    dialog.set_extra_child(Some(&group));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Save");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let owner = Rc::clone(sidebar);
    let old = folder.to_string();
    dialog.connect_response(None, move |dialog, response| {
        if response != "rename" {
            return;
        }

        let new = entry.text().trim().to_string();
        if new.is_empty() {
            dialog.set_body("The folder name cannot be empty.");
            return;
        }

        owner.store.borrow_mut().rename_folder(&old, &new);
        owner.refresh();
    });

    dialog.present(Some(&sidebar.widget));
}

/// Right-click a folder name to delete it. A folder is only a label, so
/// deleting it removes no hosts; it just ungroups them.
fn attach_folder_menu(label: &gtk::Label, folder: &str, sidebar: &Rc<Sidebar>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gtk::gdk::BUTTON_SECONDARY);

    let folder = folder.to_string();
    let sidebar = Rc::clone(sidebar);
    gesture.connect_pressed(move |_, _, _, _| {
        let dialog = adw::AlertDialog::new(
            Some("Delete folder"),
            Some(&format!(
                "Remove the folder \"{folder}\"? The hosts inside are not deleted, \
                 they are only ungrouped."
            )),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");

        let parent = sidebar.widget.clone();
        let owner = Rc::clone(&sidebar);
        let name = folder.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                owner.store.borrow_mut().remove_folder(&name);
                owner.refresh();
            }
        });

        dialog.present(Some(&parent));
    });

    label.add_controller(gesture);
}

fn empty_folder_hint() -> gtk::Label {
    let label = gtk::Label::new(Some("Empty - move a host here with Edit"));
    label.set_xalign(0.0);
    label.set_margin_start(18);
    label.set_margin_bottom(4);
    label.add_css_class("row-meta");
    label
}

fn empty_hint(needle: &str) -> gtk::Label {
    let text = if needle.is_empty() {
        "No records. Add one with + below, or create ~/.ssh/config."
    } else {
        "No matching records."
    };

    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_margin_start(12);
    label.set_margin_end(12);
    label.set_margin_top(4);
    label.add_css_class("row-meta");
    label
}

pub fn build(
    store: Rc<RefCell<Store>>,
    live: Rc<Sessions>,
    connect: Rc<dyn Fn(&str)>,
    connect_new: Rc<dyn Fn(&str)>,
) -> Rc<Sidebar> {
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    stack.set_vexpand(true);

    let sessions = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stack.add_named(&scrolled(&sessions), Some(Mode::Sessions.id()));
    stack.add_named(&agents_page(), Some(Mode::Agents.id()));

    let view = adw::ToolbarView::new();
    view.add_css_class("sidebar-pane");
    view.set_top_bar_style(adw::ToolbarStyle::Flat);
    view.set_bottom_bar_style(adw::ToolbarStyle::Flat);

    // No title above the sidebar; an empty bar so the window can be dragged.
    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    header.set_show_end_title_buttons(false);
    header.add_css_class("flat");
    view.add_top_bar(&header);
    view.set_content(Some(&stack));

    // The menu is parented to `stack`, not the box holding the list: refresh()
    // removes every child of that box, and the popover counted as one, so it
    // was unparented and the next right-click popped up with no parent - a crash.
    let menu = gtk::PopoverMenu::from_model(None::<&gio::Menu>);
    menu.set_parent(&stack);
    menu.set_has_arrow(false);
    menu.set_halign(gtk::Align::Start);

    let sidebar = Rc::new(Sidebar {
        widget: view,
        header,
        stack,
        menu,
        sessions,
        store,
        live,
        new_in_folder: RefCell::new(None),
        connect,
        connect_new,
        filter: RefCell::new(String::new()),
    });

    sidebar.widget.add_bottom_bar(&footer(&sidebar));
    sidebar.refresh();
    sidebar
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    sw.set_vexpand(true);
    sw.set_child(Some(child));
    sw
}

fn agents_page() -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&rows::section_header("Agents"));

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    for agent in AGENTS {
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&rows::agent_row(agent)));
        list.append(&row);
    }
    page.append(&list);

    scrolled(&page).upcast()
}


fn footer(sidebar: &Rc<Sidebar>) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.add_css_class("sidebar-footer");

    let menu = gio::Menu::new();
    menu.append(Some("New Session"), Some("win.host-new"));
    menu.append(Some("Reload ~/.ssh/config"), Some("win.reload-ssh-config"));
    menu.append(Some("Restore hidden records"), Some("win.restore-hidden"));

    let add = gtk::MenuButton::new();
    add.set_icon_name("list-add-symbolic");
    add.set_menu_model(Some(&menu));
    add.set_tooltip_text(Some("New"));

    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some("Filter"));
    filter.set_hexpand(true);

    let this = Rc::clone(sidebar);
    filter.connect_search_changed(move |entry| {
        *this.filter.borrow_mut() = entry.text().to_string();
        this.refresh();
    });

    bar.append(&add);
    bar.append(&filter);
    bar
}
