//! Tasks & Bugs window: search, active-only filter, sprint/all scope, per-item
//! status change, parent-US link, hours total, direct logging, and edit/delete
//! of individual time entries. Built with gtk4-rs.

use crate::models::WorkItem;
use crate::store::Store;
use gtk::prelude::*;
use gtk::{glib, Align, Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation,
          PolicyType, ScrolledWindow, SearchEntry, StringList, Window};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub struct TasksWindow {
    pub window: Window,
    store: Arc<Store>,
    list_box: GtkBox,
    search: SearchEntry,
    active_only: CheckButton,
    scope: DropDown,
    status: Label,
}

impl TasksWindow {
    pub fn new(store: Arc<Store>) -> Rc<RefCell<TasksWindow>> {
        let window = Window::builder().title("TimeAgent — Tasks").default_width(760).default_height(560).build();

        let root = GtkBox::new(Orientation::Vertical, 0);

        // toolbar
        let toolbar = GtkBox::new(Orientation::Horizontal, 8);
        toolbar.set_margin_top(8);
        toolbar.set_margin_bottom(8);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let search = SearchEntry::new();
        search.set_placeholder_text(Some("Search tasks & bugs (name or #id)…"));
        search.set_hexpand(true);
        let active_only = CheckButton::with_label("Active only");
        active_only.set_active(true);
        let scope = DropDown::from_strings(&["Current sprint", "All"]);
        let refresh = Button::from_icon_name("view-refresh-symbolic");
        toolbar.append(&search);
        toolbar.append(&active_only);
        toolbar.append(&scope);
        toolbar.append(&refresh);
        root.append(&toolbar);

        // list
        let list_box = GtkBox::new(Orientation::Vertical, 6);
        list_box.set_margin_start(8);
        list_box.set_margin_end(8);
        let scroller = ScrolledWindow::builder().vexpand(true).hscrollbar_policy(PolicyType::Never).child(&list_box).build();
        root.append(&scroller);

        // footer
        let status = Label::new(None);
        status.set_halign(Align::Start);
        status.set_margin_start(8);
        status.set_margin_top(4);
        status.set_margin_bottom(6);
        root.append(&status);

        window.set_child(Some(&root));

        let me = Rc::new(RefCell::new(TasksWindow { window, store: store.clone(), list_box, search, active_only, scope, status }));

        // wiring
        {
            let me2 = me.clone();
            me.borrow().search.connect_search_changed(move |_| me2.borrow().render());
        }
        {
            let me2 = me.clone();
            me.borrow().active_only.connect_toggled(move |_| me2.borrow().render());
        }
        {
            let me2 = me.clone();
            let store = store.clone();
            me.borrow().scope.connect_selected_notify(move |dd| {
                *store.scope_all.lock().unwrap() = dd.selected() == 1;
                store.refresh();
                me2.borrow().render();
            });
        }
        {
            let store = store.clone();
            refresh.connect_clicked(move |_| store.refresh());
        }

        me.borrow().render();
        me
    }

    pub fn render(&self) {
        // clear
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        let q = self.search.text().to_lowercase();
        let active_only = self.active_only.is_active();
        let items = self.store.items.lock().unwrap().clone();
        let mut shown = 0;
        for item in items.iter() {
            if active_only && item.is_final {
                continue;
            }
            if !q.is_empty() && !(item.name.to_lowercase().contains(&q) || item.id.to_string().contains(&q)) {
                continue;
            }
            self.list_box.append(&self.row(item));
            shown += 1;
        }
        self.status.set_text(&format!("{} shown", shown));
    }

    fn row(&self, item: &WorkItem) -> GtkBox {
        let store = self.store.clone();
        let row = GtkBox::new(Orientation::Vertical, 4);
        row.add_css_class("card");
        row.set_margin_top(2);

        // line 1: badge + name + hours
        let l1 = GtkBox::new(Orientation::Horizontal, 8);
        let badge = Label::new(Some(&item.display_type));
        badge.add_css_class(if item.entity_type == "Bugs" { "bug-badge" } else { "task-badge" });
        let name = Label::new(Some(&item.name));
        name.set_hexpand(true);
        name.set_halign(Align::Start);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l1.append(&badge);
        l1.append(&name);
        let total = self.store.hours_for(item.id);
        if total > 0.0 {
            let h = Button::with_label(&format!("{:.2}h", total));
            // toggle the slots panel
            let slots_holder = GtkBox::new(Orientation::Vertical, 4);
            let sh = slots_holder.clone();
            let store2 = store.clone();
            let item_id = item.id;
            let shown = Rc::new(RefCell::new(false));
            h.connect_clicked(move |_| {
                let mut vis = shown.borrow_mut();
                *vis = !*vis;
                while let Some(c) = sh.first_child() { sh.remove(&c); }
                if *vis {
                    build_slots(&sh, &store2, item_id);
                }
            });
            l1.append(&h);
            row.append(&l1);
            row.append(&slots_holder);
        } else {
            row.append(&l1);
        }

        // line 2: links + state + project + sprint
        let l2 = GtkBox::new(Orientation::Horizontal, 8);
        let id_link = Button::with_label(&format!("#{} ↗", item.id));
        id_link.add_css_class("link");
        {
            let store = store.clone();
            let id = item.id;
            id_link.connect_clicked(move |_| store.open_in_tp(id));
        }
        l2.append(&id_link);
        if item.us_id != 0 {
            let us = Button::with_label(&format!("US #{} ↗", item.us_id));
            us.add_css_class("link");
            us.set_tooltip_text(Some(&item.us_name));
            let store = store.clone();
            let usid = item.us_id;
            us.connect_clicked(move |_| store.open_in_tp(usid));
            l2.append(&us);
        }
        // state dropdown
        let states = self.store.states_for(item);
        if !states.is_empty() {
            let names: Vec<&str> = states.iter().map(|s| s.name.as_str()).collect();
            let model = StringList::new(&names);
            let dd = DropDown::new(Some(model), gtk::Expression::NONE);
            if let Some(idx) = states.iter().position(|s| s.id == item.state_id) {
                dd.set_selected(idx as u32);
            }
            let store = store.clone();
            let item2 = item.clone();
            let states2 = states.clone();
            dd.connect_selected_notify(move |dd| {
                let i = dd.selected() as usize;
                if let Some(s) = states2.get(i) {
                    if s.id != item2.state_id {
                        store.change_state(item2.clone(), s.clone());
                    }
                }
            });
            l2.append(&dd);
        } else {
            l2.append(&Label::new(Some(&format!("{} ▾", item.state_name))));
        }
        l2.append(&Label::new(Some(&item.project_name)));
        if !item.sprint.is_empty() {
            let sp = Label::new(Some(&item.sprint));
            sp.add_css_class("pill");
            l2.append(&sp);
        }
        row.append(&l2);

        // line 3: direct log row
        let l3 = GtkBox::new(Orientation::Horizontal, 6);
        let hrs = Entry::builder().placeholder_text("hrs").max_width_chars(5).build();
        let date = Entry::builder().placeholder_text("YYYY-MM-DD").build();
        date.set_text(&chrono::Local::now().format("%Y-%m-%d").to_string());
        let note = Entry::builder().placeholder_text("optional note").hexpand(true).build();
        let log = Button::with_label("Log");
        log.add_css_class("suggested-action");
        {
            let store = store.clone();
            let id = item.id;
            let (hrs2, date2, note2) = (hrs.clone(), date.clone(), note.clone());
            log.connect_clicked(move |_| {
                let h: f64 = hrs2.text().replace(',', ".").parse().unwrap_or(0.0);
                if h <= 0.0 {
                    return;
                }
                let d = chrono::NaiveDate::parse_from_str(date2.text().as_str(), "%Y-%m-%d")
                    .unwrap_or_else(|_| chrono::Local::now().date_naive());
                store.log_time(id, h, note2.text().to_string(), d);
                hrs2.set_text("");
                note2.set_text("");
            });
        }
        l3.append(&hrs);
        l3.append(&date);
        l3.append(&note);
        l3.append(&log);
        row.append(&l3);

        row
    }
}

/// Build the per-task time-entry breakdown (edit/delete) into `holder`.
fn build_slots(holder: &GtkBox, store: &Arc<Store>, item_id: i64) {
    let times: Vec<_> = store.times.lock().unwrap().iter().filter(|t| t.item_id == item_id).cloned().collect();
    let total: f64 = times.iter().map(|t| t.hours).sum();
    let head = GtkBox::new(Orientation::Horizontal, 8);
    head.append(&Label::new(Some("Time entries")));
    let t = Label::new(Some(&format!("Total {:.2}h", total)));
    t.set_hexpand(true);
    t.set_halign(Align::End);
    head.append(&t);
    holder.append(&head);

    let mut slots = times;
    slots.sort_by(|a, b| b.day.cmp(&a.day));
    for entry in slots {
        let r = GtkBox::new(Orientation::Horizontal, 6);
        r.append(&Label::new(Some(&entry.day)));
        let hrs = Entry::builder().text(&format!("{}", entry.hours)).max_width_chars(5).build();
        let note = Entry::builder().text(&entry.description).hexpand(true).build();
        let save = Button::with_label("Save");
        save.add_css_class("suggested-action");
        {
            let store = store.clone();
            let (hrs2, note2) = (hrs.clone(), note.clone());
            let (tid, day) = (entry.id, entry.day.clone());
            save.connect_clicked(move |_| {
                let h: f64 = hrs2.text().replace(',', ".").parse().unwrap_or(0.0);
                if h > 0.0 {
                    store.update_time(tid, h, note2.text().to_string(), day.clone());
                }
            });
        }
        let del = Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("destructive-action");
        {
            let store = store.clone();
            let tid = entry.id;
            let win = holder.root().and_downcast::<gtk::Window>();
            del.connect_clicked(move |_| {
                // simple confirm dialog
                let dialog = gtk::AlertDialog::builder()
                    .message("Delete this time entry?")
                    .buttons(["Cancel", "Delete"])
                    .cancel_button(0)
                    .default_button(0)
                    .build();
                let store2 = store.clone();
                dialog.choose(win.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
                    if res == Ok(1) {
                        store2.delete_time(tid);
                    }
                });
            });
        }
        r.append(&hrs);
        r.append(&note);
        r.append(&save);
        r.append(&del);
        holder.append(&r);
    }
}
