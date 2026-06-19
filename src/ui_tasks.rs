//! Tasks & Bugs window: search, active-only filter, sprint/all scope, per-item
//! status change, parent-US link, hours total, direct logging, and edit/delete
//! of individual time entries. Built with gtk4-rs.

use crate::command::TrayCmd;
use crate::models::WorkItem;
use crate::store::Store;
use chrono::Datelike;
use gtk::prelude::*;
use gtk::{glib, Align, Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation,
          PolicyType, ScrolledWindow, SearchEntry, StringList, Window};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const CSS: &str = "
.ta-card {
  background: alpha(@theme_fg_color, 0.035);
  border: 1px solid alpha(@theme_fg_color, 0.08);
  border-radius: 12px; padding: 12px;
}
.ta-badge { color: white; font-weight: bold; font-size: 10px; padding: 2px 8px; border-radius: 6px; }
.ta-badge-task { background: #2d7dd2; }
.ta-badge-bug { background: #d2452d; }
.ta-title { font-weight: bold; font-size: 14px; }
.ta-pill { background: alpha(#3d7de8, 0.18); border-radius: 8px; padding: 1px 8px; font-size: 11px; }
.ta-link { background: none; border: none; box-shadow: none; color: #3d7de8; padding: 0 2px; min-height: 0; }
.ta-link:hover { color: #2566c0; }
.ta-hours { background: alpha(@theme_fg_color, 0.08); border-radius: 8px; padding: 2px 10px; font-weight: bold; }
.ta-muted { color: alpha(@theme_fg_color, 0.55); font-size: 12px; }
.ta-foot-fig { font-weight: bold; }
";

fn css_once() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(CSS);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

pub struct TasksWindow {
    pub window: Window,
    store: Arc<Store>,
    list_box: GtkBox,
    search: SearchEntry,
    active_only: CheckButton,
    scope: DropDown,
    status_filter: DropDown,
    status_model: StringList,
    sort: DropDown,
    status: Label,
    /// The Stop button of the currently-tracked row, so a 1s tick can update
    /// its elapsed label without re-rendering the whole list (which would wipe
    /// the per-row note/hrs inputs).
    track_btn: RefCell<Option<Button>>,
    /// Guards against re-entrant render() from rebuilding the status dropdown.
    rendering: Cell<bool>,
}

impl TasksWindow {
    pub fn new(store: Arc<Store>, cmd: std::sync::mpsc::Sender<TrayCmd>) -> Rc<RefCell<TasksWindow>> {
        css_once();
        // hide_on_close: closing must not destroy the window — main.rs keeps an
        // Rc to it and re-present()s on the next open; presenting a destroyed
        // window leaves GTK in an inconsistent state (blank, frozen frame).
        let window = Window::builder()
            .title("TimeAgent — Tasks")
            .default_width(760)
            .default_height(560)
            .hide_on_close(true)
            .build();

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
        scope.set_tooltip_text(Some("Sprint scope"));
        let status_model = StringList::new(&["All statuses"]);
        let status_filter = DropDown::new(Some(status_model.clone()), gtk::Expression::NONE);
        status_filter.set_tooltip_text(Some("Filter by status"));
        let sort = DropDown::from_strings(&["Name A–Z", "Name Z–A", "Most hours", "Least hours"]);
        sort.set_tooltip_text(Some("Sort order"));
        let refresh = Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Reload tasks & times"));
        let settings_btn = Button::from_icon_name("preferences-system-symbolic");
        settings_btn.set_tooltip_text(Some("Settings"));
        toolbar.append(&search);
        toolbar.append(&active_only);
        toolbar.append(&scope);
        toolbar.append(&status_filter);
        toolbar.append(&sort);
        toolbar.append(&refresh);
        toolbar.append(&settings_btn);
        root.append(&toolbar);

        // Settings opens via the shared command pump (main.rs).
        {
            let c = cmd.clone();
            settings_btn.connect_clicked(move |_| { let _ = c.send(TrayCmd::OpenSettings); });
        }

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

        let me = Rc::new(RefCell::new(TasksWindow {
            window,
            store: store.clone(),
            list_box,
            search,
            active_only,
            scope,
            status_filter,
            status_model,
            sort,
            status,
            track_btn: RefCell::new(None),
            rendering: Cell::new(false),
        }));

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
            me.borrow().status_filter.connect_selected_notify(move |_| me2.borrow().render());
        }
        {
            let me2 = me.clone();
            me.borrow().sort.connect_selected_notify(move |_| me2.borrow().render());
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

        // Tick the active task's Stop button (elapsed) every second.
        {
            let me2 = me.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
                let w = me2.borrow();
                if let Some((_, d)) = w.store.tracking_task() {
                    if let Some(btn) = w.track_btn.borrow().as_ref() {
                        btn.set_label(&format!("■  {}", fmt_hms(d.as_secs())));
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        me.borrow().render();
        me
    }

    pub fn render(&self) {
        if self.rendering.get() {
            return; // re-entrant call (e.g. from rebuilding the status dropdown)
        }
        self.rendering.set(true);

        // clear
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        *self.track_btn.borrow_mut() = None; // re-captured by the active row below

        let items_all = self.store.items.lock().unwrap().clone();
        self.sync_status_options(&items_all);

        let q = self.search.text().to_lowercase();
        let active_only = self.active_only.is_active();
        let status_sel = self
            .status_model
            .string(self.status_filter.selected())
            .map(|g| g.to_string());
        let status_on = status_sel.as_deref().map(|s| s != "All statuses").unwrap_or(false);

        let mut items: Vec<WorkItem> = items_all
            .into_iter()
            .filter(|item| {
                if active_only && item.is_final {
                    return false;
                }
                if !q.is_empty()
                    && !(item.name.to_lowercase().contains(&q) || item.id.to_string().contains(&q))
                {
                    return false;
                }
                if status_on && Some(item.state_name.as_str()) != status_sel.as_deref() {
                    return false;
                }
                true
            })
            .collect();

        let eq = std::cmp::Ordering::Equal;
        match self.sort.selected() {
            1 => items.sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase())),
            2 => items.sort_by(|a, b| self.store.hours_for(b.id).partial_cmp(&self.store.hours_for(a.id)).unwrap_or(eq)),
            3 => items.sort_by(|a, b| self.store.hours_for(a.id).partial_cmp(&self.store.hours_for(b.id)).unwrap_or(eq)),
            _ => items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        }

        let mut shown = 0;
        for item in &items {
            self.list_box.append(&self.row(item));
            shown += 1;
        }

        // Tracked-hours summary (Today / this week / this month).
        let month = chrono::Local::now().date_naive().with_day(1).unwrap();
        let (today_h, week_h, month_h) = self.store.hours_summary(month);
        self.status.set_text(&format!(
            "{} shown      Today {:.2}h      Week {:.2}h      {} {:.2}h",
            shown,
            today_h,
            week_h,
            month.format("%B %Y"),
            month_h
        ));

        self.rendering.set(false);
    }

    /// Rebuild the status dropdown from the statuses present in `items`,
    /// preserving the current selection by name. No-op if unchanged.
    fn sync_status_options(&self, items: &[WorkItem]) {
        let mut statuses: Vec<String> = items
            .iter()
            .map(|i| i.state_name.clone())
            .filter(|s| !s.is_empty())
            .collect();
        statuses.sort();
        statuses.dedup();
        let want: Vec<String> = std::iter::once("All statuses".to_string())
            .chain(statuses)
            .collect();
        let cur: Vec<String> = (0..self.status_model.n_items())
            .filter_map(|i| self.status_model.string(i).map(|g| g.to_string()))
            .collect();
        if cur == want {
            return;
        }
        let prev = self
            .status_model
            .string(self.status_filter.selected())
            .map(|g| g.to_string());
        while self.status_model.n_items() > 0 {
            self.status_model.remove(0);
        }
        for s in &want {
            self.status_model.append(s);
        }
        let idx = prev
            .and_then(|p| want.iter().position(|w| *w == p))
            .unwrap_or(0) as u32;
        self.status_filter.set_selected(idx);
    }

    fn row(&self, item: &WorkItem) -> GtkBox {
        let store = self.store.clone();
        let row = GtkBox::new(Orientation::Vertical, 6);
        row.add_css_class("ta-card");
        row.set_margin_top(4);

        // line 1: badge + name + start/stop + hours
        let l1 = GtkBox::new(Orientation::Horizontal, 8);
        let badge = Label::new(Some(&item.display_type));
        badge.add_css_class("ta-badge");
        badge.add_css_class(if item.entity_type == "Bugs" { "ta-badge-bug" } else { "ta-badge-task" });
        badge.set_valign(Align::Center);
        let name = Label::new(Some(&item.name));
        name.add_css_class("ta-title");
        name.set_hexpand(true);
        name.set_halign(Align::Start);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l1.append(&badge);
        l1.append(&name);

        // start / stop stopwatch for this task
        let tracking = self.store.tracking_task();
        let is_active = tracking.map(|(id, _)| id) == Some(item.id);
        let label = if is_active {
            let secs = tracking.map(|(_, d)| d.as_secs()).unwrap_or(0);
            format!("■  {}", fmt_hms(secs))
        } else {
            "▶  Start".to_string()
        };
        let track = Button::with_label(&label);
        track.add_css_class(if is_active { "destructive-action" } else { "suggested-action" });
        track.set_valign(Align::Center);
        {
            let store = store.clone();
            let id = item.id;
            track.connect_clicked(move |_| {
                if is_active {
                    store.stop_task();
                } else {
                    store.start_task(id);
                }
            });
        }
        if is_active {
            *self.track_btn.borrow_mut() = Some(track.clone());
        }
        l1.append(&track);

        let total = self.store.hours_for(item.id);
        if total > 0.0 {
            let h = Button::with_label(&format!("⏱ {:.2}h", total));
            h.add_css_class("ta-hours");
            h.add_css_class("flat");
            h.set_valign(Align::Center);
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
        id_link.add_css_class("ta-link");
        id_link.add_css_class("flat");
        {
            let store = store.clone();
            let id = item.id;
            id_link.connect_clicked(move |_| store.open_in_tp(id));
        }
        l2.append(&id_link);
        if item.us_id != 0 {
            let us = Button::with_label(&format!("US #{} ↗", item.us_id));
            us.add_css_class("ta-link");
            us.add_css_class("flat");
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
        let proj = Label::new(Some(&item.project_name));
        proj.add_css_class("ta-muted");
        proj.set_hexpand(true);
        proj.set_halign(Align::End);
        proj.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l2.append(&proj);
        if !item.sprint.is_empty() {
            let sp = Label::new(Some(&item.sprint));
            sp.add_css_class("ta-pill");
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

fn fmt_hms(secs: u64) -> String {
    format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
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
