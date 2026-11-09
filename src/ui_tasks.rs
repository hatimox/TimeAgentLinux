//! Tasks & Bugs window: search, active-only filter, sprint/all scope, per-item
//! status change, parent-US link, hours total, direct logging, and edit/delete
//! of individual time entries. Built with gtk4-rs.

use crate::command::TrayCmd;
use crate::models::WorkItem;
use crate::store::Store;
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
.ta-badge-story { background: #8a4fd2; }
.ta-summary { background: alpha(@theme_fg_color, 0.05); padding: 6px 12px; }
.ta-chip { background: alpha(@theme_fg_color, 0.08); border-radius: 10px; padding: 1px 8px; font-size: 12px; }
.ta-err { color: #d23b3b; font-size: 12px; }
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
    type_filter: DropDown,
    sort: DropDown,
    summary: GtkBox,
    status: Label,
    day_lbl: Label,
    week_lbl: Label,
    month_lbl: Label,
    day_next: Button,
    week_next: Button,
    month_next: Button,
    day_off: Rc<Cell<i32>>,
    week_off: Rc<Cell<i32>>,
    month_off: Rc<Cell<i32>>,
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
        search.set_placeholder_text(Some("Search stories, tasks & bugs (name or #id)…"));
        search.set_hexpand(true);
        let active_only = CheckButton::with_label("Active only");
        active_only.set_active(true);
        let scope = DropDown::from_strings(&["Current sprint", "All"]);
        scope.set_tooltip_text(Some("Sprint scope"));
        let type_filter = DropDown::from_strings(&["All types", "User Story", "Task", "Bug"]);
        type_filter.set_tooltip_text(Some("Filter by type"));
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
        toolbar.append(&type_filter);
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

        // sprint summary strip (rebuilt on every render)
        let summary = GtkBox::new(Orientation::Horizontal, 8);
        summary.add_css_class("ta-summary");
        root.append(&summary);

        // list
        let list_box = GtkBox::new(Orientation::Vertical, 6);
        list_box.set_margin_start(8);
        list_box.set_margin_end(8);
        let scroller = ScrolledWindow::builder().vexpand(true).hscrollbar_policy(PolicyType::Never).child(&list_box).build();
        root.append(&scroller);

        // footer: status + Day / Week / Month totals, each with its own ‹ ›
        let footer = GtkBox::new(Orientation::Horizontal, 12);
        footer.set_margin_start(8);
        footer.set_margin_end(8);
        footer.set_margin_top(4);
        footer.set_margin_bottom(6);
        let status = Label::new(None);
        status.set_halign(Align::Start);
        status.set_hexpand(true);
        footer.append(&status);
        let day_off = Rc::new(Cell::new(0));
        let week_off = Rc::new(Cell::new(0));
        let month_off = Rc::new(Cell::new(0));
        let (day_box, day_lbl, day_prev, day_next) = stepper(&day_off);
        let (week_box, week_lbl, week_prev, week_next) = stepper(&week_off);
        let (month_box, month_lbl, month_prev, month_next) = stepper(&month_off);
        footer.append(&day_box);
        footer.append(&week_box);
        footer.append(&month_box);
        root.append(&footer);

        // Clicking the day total opens that day's entries.
        {
            let (c, off) = (cmd.clone(), day_off.clone());
            let click = gtk::GestureClick::new();
            click.connect_released(move |_, _, _, _| {
                let _ = c.send(TrayCmd::OpenDay { offset: off.get() });
            });
            day_lbl.add_controller(click);
            day_lbl.set_tooltip_text(Some("Show this day's entries"));
            day_lbl.set_cursor_from_name(Some("pointer"));
        }

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
            type_filter,
            sort,
            summary,
            status,
            day_lbl,
            week_lbl,
            month_lbl,
            day_next,
            week_next,
            month_next,
            day_off: day_off.clone(),
            week_off: week_off.clone(),
            month_off: month_off.clone(),
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
            me.borrow().type_filter.connect_selected_notify(move |_| me2.borrow().render());
        }
        // footer steppers only refresh the totals (not the whole list)
        let (day_next_b, week_next_b, month_next_b) = {
            let w = me.borrow();
            (w.day_next.clone(), w.week_next.clone(), w.month_next.clone())
        };
        for (btn, off, delta) in [
            (day_prev, day_off.clone(), -1),
            (week_prev, week_off.clone(), -1),
            (month_prev, month_off.clone(), -1),
            (day_next_b, day_off.clone(), 1),
            (week_next_b, week_off.clone(), 1),
            (month_next_b, month_off.clone(), 1),
        ] {
            let me2 = me.clone();
            btn.connect_clicked(move |_| {
                let n = off.get() + delta;
                if n <= 0 {
                    off.set(n);
                }
                me2.borrow().update_footer();
            });
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
        let type_sel = match self.type_filter.selected() {
            1 => Some("User Story"),
            2 => Some("Task"),
            3 => Some("Bug"),
            _ => None,
        };
        self.render_summary(&items_all);

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
                if let Some(t) = type_sel {
                    if item.display_type != t {
                        return false;
                    }
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

        self.status.set_text(&format!("{} shown", shown));
        self.update_footer();

        self.rendering.set(false);
    }

    /// Day / Week / Month totals for the footer's current offsets.
    fn update_footer(&self) {
        let (d, w, m) = (self.day_off.get(), self.week_off.get(), self.month_off.get());
        let t = self.store.totals(d, w, m);
        use crate::target_value::apply;
        apply(&self.day_lbl, &format!("{} ", t.day_label), t.day, t.day_target, "off");
        apply(&self.week_lbl, &format!("{} ", t.week_label), t.week, t.week_target, "off");
        apply(&self.month_lbl, &format!("{} ", t.month_label), t.month, t.month_target, "off");
        self.day_next.set_sensitive(d < 0);
        self.week_next.set_sensitive(w < 0);
        self.month_next.set_sensitive(m < 0);
    }

    /// Open/done counts per type plus my all-time hours on those items, for the
    /// selected sprint scope (ignores search, type, status and Active filters).
    fn render_summary(&self, items: &[WorkItem]) {
        while let Some(c) = self.summary.first_child() {
            self.summary.remove(&c);
        }
        let scope = if self.scope.selected() == 1 { "All sprints" } else { "Current sprint" };
        let head = Label::new(Some(scope));
        head.add_css_class("ta-title");
        self.summary.append(&head);
        self.summary.append(&summary_chip(items, "User Stories", |i| i.is_story()));
        self.summary.append(&summary_chip(items, "Tasks", |i| !i.is_story() && !i.is_bug()));
        self.summary.append(&summary_chip(items, "Bugs", |i| i.is_bug()));
        let hours: f64 = items.iter().map(|i| self.store.hours_for(i.id)).sum();
        let h = Label::new(Some(&format!("⏱ {:.2}h logged", hours)));
        h.set_hexpand(true);
        h.set_halign(Align::End);
        h.set_tooltip_text(Some("All time you logged on these items"));
        self.summary.append(&h);
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
        let badge = Label::new(Some(&item.display_type.to_uppercase()));
        badge.add_css_class("ta-badge");
        badge.add_css_class(if item.is_story() { "ta-badge-story" } else if item.is_bug() { "ta-badge-bug" } else { "ta-badge-task" });
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
        if item.us_id != 0 && item.us_id != item.id {
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
        let required = self.store.settings.lock().unwrap().time_description_required;
        let note = Entry::builder().placeholder_text(note_placeholder(required)).hexpand(true).build();
        let log = Button::with_label("Log");
        log.add_css_class("suggested-action");
        let err = Label::new(None);
        err.add_css_class("ta-err");
        err.set_halign(Align::Start);
        err.set_xalign(0.0);
        err.set_wrap(true);
        err.set_selectable(true);
        err.set_visible(false);
        let busy = Rc::new(Cell::new(false));
        sync_log_btn(&store, &note, &log, &busy);
        {
            let (store, note2, log2, busy2) = (store.clone(), note.clone(), log.clone(), busy.clone());
            note.connect_changed(move |_| sync_log_btn(&store, &note2, &log2, &busy2));
        }
        {
            let store = store.clone();
            let id = item.id;
            let (hrs2, date2, note2, err2, busy2) = (hrs.clone(), date.clone(), note.clone(), err.clone(), busy.clone());
            log.connect_clicked(move |btn| {
                let h: f64 = hrs2.text().replace(',', ".").parse().unwrap_or(0.0);
                if h <= 0.0 {
                    err2.set_text("Enter hours greater than 0.");
                    err2.set_visible(true);
                    return;
                }
                let d = chrono::NaiveDate::parse_from_str(date2.text().as_str(), "%Y-%m-%d")
                    .unwrap_or_else(|_| chrono::Local::now().date_naive());
                busy2.set(true);
                btn.set_sensitive(false);
                err2.set_visible(false);
                let rx = store.log_time(id, h, note2.text().to_string(), d);
                let (store, btn, hrs, note, err, busy) =
                    (store.clone(), btn.clone(), hrs2.clone(), note2.clone(), err2.clone(), busy2.clone());
                // Clear the fields only once TP accepts the entry; on failure
                // they stay filled and the error shows under the row.
                glib::spawn_future_local(async move {
                    match rx.recv().await {
                        Ok(Ok(())) => {
                            hrs.set_text("");
                            note.set_text("");
                        }
                        Ok(Err(e)) => {
                            err.set_text(&e);
                            err.set_visible(true);
                            let req = store.settings.lock().unwrap().time_description_required;
                            note.set_placeholder_text(Some(note_placeholder(req)));
                        }
                        Err(_) => {}
                    }
                    busy.set(false);
                    sync_log_btn(&store, &note, &btn, &busy);
                });
            });
        }
        l3.append(&hrs);
        l3.append(&date);
        l3.append(&note);
        l3.append(&log);
        row.append(&l3);
        row.append(&err);

        row
    }
}

/// "Name: N open · M done" chip for the items matching `pred`.
fn summary_chip(items: &[WorkItem], name: &str, pred: fn(&WorkItem) -> bool) -> Label {
    let of: Vec<&WorkItem> = items.iter().filter(|i| pred(*i)).collect();
    let done = of.iter().filter(|i| i.is_final).count();
    let l = Label::new(Some(&format!("{}: {} open · {} done", name, of.len() - done, done)));
    l.add_css_class("ta-chip");
    l.set_tooltip_text(Some(&format!("{} {} assigned to you", of.len(), name.to_lowercase())));
    l
}

fn note_placeholder(required: bool) -> &'static str {
    if required { "Description (required)" } else { "optional note" }
}

/// Log is disabled while saving, or when a description is required but empty.
fn sync_log_btn(store: &Arc<Store>, note: &Entry, btn: &Button, busy: &Rc<Cell<bool>>) {
    let required = store.settings.lock().unwrap().time_description_required;
    btn.set_sensitive(!busy.get() && !(required && note.text().trim().is_empty()));
}

/// ‹ label › stepper for a footer total: (box, label, prev, next).
fn stepper(_off: &Rc<Cell<i32>>) -> (GtkBox, Label, Button, Button) {
    let b = GtkBox::new(Orientation::Horizontal, 2);
    let prev = Button::from_icon_name("go-previous-symbolic");
    prev.add_css_class("flat");
    let next = Button::from_icon_name("go-next-symbolic");
    next.add_css_class("flat");
    next.set_sensitive(false);
    let l = Label::new(None);
    l.add_css_class("ta-foot-fig");
    b.append(&prev);
    b.append(&l);
    b.append(&next);
    (b, l, prev, next)
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
