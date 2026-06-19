//! Settings window: account/token, meeting task ids + rounding, dynamic
//! meetings, recurring entries, days off. gtk4-rs with a Notebook (tabs).

use crate::models::{DynamicMeeting, RecurringEntry};
use crate::store::Store;
use gtk::prelude::*;
use gtk::{glib, Align, Box as GtkBox, Button, DropDown, Entry, Label, Notebook, Orientation, PasswordEntry, Window};
use std::sync::Arc;

pub fn open(store: Arc<Store>) -> Window {
    let window = Window::builder().title("TimeAgent Settings").default_width(500).default_height(470).hide_on_close(true).build();
    let nb = Notebook::new();
    nb.append_page(&account_tab(&store, &window), Some(&Label::new(Some("Account"))));
    nb.append_page(&meetings_tab(&store), Some(&Label::new(Some("Meetings"))));
    nb.append_page(&recurring_tab(&store), Some(&Label::new(Some("Recurring"))));
    nb.append_page(&daysoff_tab(&store), Some(&Label::new(Some("Days off"))));
    window.set_child(Some(&nb));
    window
}

fn field(label: &str, entry: &impl IsA<gtk::Widget>) -> GtkBox {
    let b = GtkBox::new(Orientation::Horizontal, 8);
    b.set_margin_top(4);
    let l = Label::new(Some(label));
    l.set_width_chars(16);
    l.set_halign(Align::Start);
    b.append(&l);
    b.append(entry);
    b
}

fn account_tab(store: &Arc<Store>, window: &Window) -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 6);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b.set_margin_start(12);
    b.set_margin_end(12);

    let (url, token, name, id) = {
        let s = store.settings.lock().unwrap();
        (s.tp_url.clone(), s.token.clone(), s.my_user_name.clone(), s.my_user_id)
    };

    let url_e = Entry::builder().text(&url).hexpand(true).placeholder_text("https://company.tpondemand.com").build();
    let tok_e = PasswordEntry::builder().hexpand(true).show_peek_icon(true).build();
    tok_e.set_text(&token);

    let who = Label::new(None);
    who.set_halign(Align::Start);
    if id != 0 {
        who.set_text(&format!("Signed in: {} (id {})", name, id));
    }

    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let store = store.clone();
        let (url_e2, tok_e2, who2) = (url_e.clone(), tok_e.clone(), who.clone());
        save.connect_clicked(move |_| {
            let new_url = url_e2.text().to_string();
            let new_tok = tok_e2.text().to_string();
            {
                let mut s = store.settings.lock().unwrap();
                let changed = new_tok != s.token || new_url != s.tp_url;
                s.tp_url = new_url;
                s.token = new_tok;
                if changed {
                    s.my_user_id = 0;
                    s.my_user_name.clear();
                    s.my_user_email.clear();
                }
                s.save();
            }
            store.rebuild_client();
            store.ensure_user();
            store.refresh();
            who2.set_text("Saved — detecting user…");

            // Updates are drained by the main loop, so poll the store here to
            // reflect detection success/failure rather than hang on the label.
            let store_poll = store.clone();
            let who_poll = who2.clone();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                let (id, name) = store_poll.user_identity();
                if id != 0 {
                    who_poll.set_text(&format!("Signed in: {} (id {})", name, id));
                    return glib::ControlFlow::Break;
                }
                if std::time::Instant::now() >= deadline {
                    let st = store_poll.last_status();
                    let msg = if st.is_empty() {
                        "Could not detect user — check the URL and token.".to_string()
                    } else {
                        format!("Could not detect user — {}", st)
                    };
                    who_poll.set_text(&msg);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        });
    }

    b.append(&field("Instance URL", &url_e));
    b.append(&field("API token", &tok_e));
    b.append(&who);
    b.append(&save);
    let hint = Label::new(Some("Token: TargetProcess → My Profile → Access Tokens. Stored in libsecret."));
    hint.add_css_class("dim-label");
    hint.set_halign(Align::Start);
    b.append(&hint);
    let _ = window;
    b
}

fn int_entry(value: i64) -> Entry {
    Entry::builder().text(&value.to_string()).max_width_chars(8).build()
}

fn meetings_tab(store: &Arc<Store>) -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 6);
    b.set_margin_top(12); b.set_margin_start(12); b.set_margin_end(12);

    let (daily, meetings, min_m, step_m, dyn_list) = {
        let s = store.settings.lock().unwrap();
        (s.daily_task_id, s.meetings_task_id, s.meeting_min_minutes, s.meeting_step_minutes, s.dynamic_meetings.clone())
    };
    let daily_e = int_entry(daily);
    let meet_e = int_entry(meetings);
    let min_e = int_entry(min_m);
    let step_e = int_entry(step_m);
    b.append(&field("Daily task id", &daily_e));
    b.append(&field("Meetings task id", &meet_e));
    b.append(&field("Min minutes", &min_e));
    b.append(&field("Step minutes", &step_e));

    b.append(&Label::new(Some("Dynamic meeting shortcuts")));
    let dyn_box = GtkBox::new(Orientation::Vertical, 4);
    let rows: std::rc::Rc<std::cell::RefCell<Vec<(Entry, Entry, DynamicMeeting)>>> = Default::default();
    let render_dyn = {
        let dyn_box = dyn_box.clone();
        let rows = rows.clone();
        move |meetings: &[DynamicMeeting]| {
            while let Some(c) = dyn_box.first_child() { dyn_box.remove(&c); }
            rows.borrow_mut().clear();
            for m in meetings {
                let r = GtkBox::new(Orientation::Horizontal, 6);
                let name = Entry::builder().text(&m.name).hexpand(true).build();
                let tid = Entry::builder().text(&m.task_id.to_string()).max_width_chars(8).build();
                r.append(&name);
                r.append(&tid);
                dyn_box.append(&r);
                rows.borrow_mut().push((name, tid, m.clone()));
            }
        }
    };
    render_dyn(&dyn_list);
    b.append(&dyn_box);
    let add = Button::with_label("+ Add meeting");
    {
        let store = store.clone();
        let render_dyn = render_dyn.clone();
        add.connect_clicked(move |_| {
            let mut s = store.settings.lock().unwrap();
            s.dynamic_meetings.push(DynamicMeeting { id: glib::uuid_string_random().to_string(), name: "New meeting".into(), task_id: 0, description: String::new() });
            render_dyn(&s.dynamic_meetings);
        });
    }
    b.append(&add);

    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let store = store.clone();
        let rows = rows.clone();
        save.connect_clicked(move |_| {
            let mut s = store.settings.lock().unwrap();
            s.daily_task_id = daily_e.text().parse().unwrap_or(0);
            s.meetings_task_id = meet_e.text().parse().unwrap_or(0);
            s.meeting_min_minutes = min_e.text().parse().unwrap_or(30);
            s.meeting_step_minutes = step_e.text().parse().unwrap_or(15);
            s.dynamic_meetings = rows.borrow().iter().filter_map(|(n, t, m)| {
                let name = n.text().to_string();
                let tid: i64 = t.text().parse().unwrap_or(0);
                if name.trim().is_empty() && tid == 0 { None }
                else { Some(DynamicMeeting { id: m.id.clone(), name, task_id: tid, description: m.description.clone() }) }
            }).collect();
            s.save();
        });
    }
    b.append(&save);
    b
}

fn recurring_tab(store: &Arc<Store>) -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 6);
    b.set_margin_top(12); b.set_margin_start(12); b.set_margin_end(12);
    let list = store.settings.lock().unwrap().recurring.clone();
    let box2 = GtkBox::new(Orientation::Vertical, 4);
    let rows: std::rc::Rc<std::cell::RefCell<Vec<(Entry, Entry, Entry, RecurringEntry)>>> = Default::default();
    let render = {
        let box2 = box2.clone();
        let rows = rows.clone();
        move |recs: &[RecurringEntry]| {
            while let Some(c) = box2.first_child() { box2.remove(&c); }
            rows.borrow_mut().clear();
            for r in recs {
                let row = GtkBox::new(Orientation::Horizontal, 6);
                let label = Entry::builder().text(&r.label).hexpand(true).build();
                let tid = Entry::builder().text(&r.task_id.to_string()).max_width_chars(8).build();
                let hrs = Entry::builder().text(&r.hours.to_string()).max_width_chars(5).build();
                row.append(&label); row.append(&tid); row.append(&hrs);
                box2.append(&row);
                rows.borrow_mut().push((label, tid, hrs, r.clone()));
            }
        }
    };
    render(&list);
    b.append(&box2);
    let add = Button::with_label("+ Add recurring");
    {
        let store = store.clone();
        let render = render.clone();
        add.connect_clicked(move |_| {
            let mut s = store.settings.lock().unwrap();
            s.recurring.push(RecurringEntry { id: glib::uuid_string_random().to_string(), label: "New".into(), task_id: 0, hours: 1.0 });
            render(&s.recurring);
        });
    }
    b.append(&add);
    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let store = store.clone();
        let rows = rows.clone();
        save.connect_clicked(move |_| {
            let mut s = store.settings.lock().unwrap();
            s.recurring = rows.borrow().iter().map(|(l, t, h, r)| RecurringEntry {
                id: r.id.clone(),
                label: l.text().to_string(),
                task_id: t.text().parse().unwrap_or(0),
                hours: h.text().replace(',', ".").parse().unwrap_or(1.0),
            }).collect();
            s.save();
        });
    }
    b.append(&save);
    let hint = Label::new(Some("Auto-logged once per working day on launch, skipping days off."));
    hint.add_css_class("dim-label");
    b.append(&hint);
    b
}

fn daysoff_tab(store: &Arc<Store>) -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 6);
    b.set_margin_top(12); b.set_margin_start(12); b.set_margin_end(12);
    let (region, weekly, days) = {
        let s = store.settings.lock().unwrap();
        (s.region.clone(), s.weekly_off.clone(), s.days_off.clone())
    };
    let region_dd = DropDown::from_strings(&["none", "morocco"]);
    region_dd.set_selected(if region == "morocco" { 1 } else { 0 });
    b.append(&field("Region", &region_dd));

    b.append(&Label::new(Some("Weekly off (0=Sun … 6=Sat)")));
    let wk_box = GtkBox::new(Orientation::Horizontal, 4);
    let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let mut checks = vec![];
    for (i, n) in names.iter().enumerate() {
        let c = gtk::CheckButton::with_label(n);
        c.set_active(weekly.contains(&(i as i64)));
        wk_box.append(&c);
        checks.push(c);
    }
    b.append(&wk_box);

    b.append(&Label::new(Some("Specific days off")));
    let days_box = GtkBox::new(Orientation::Vertical, 2);
    for d in &days { days_box.append(&Label::new(Some(d))); }
    b.append(&days_box);
    let add_box = GtkBox::new(Orientation::Horizontal, 6);
    let new_day = Entry::builder().placeholder_text("YYYY-MM-DD").build();
    let add = Button::with_label("Add");
    add_box.append(&new_day);
    add_box.append(&add);
    b.append(&add_box);
    {
        let store = store.clone();
        let (days_box2, new_day2) = (days_box.clone(), new_day.clone());
        add.connect_clicked(move |_| {
            let d = new_day2.text().to_string();
            if d.len() == 10 {
                store.settings.lock().unwrap().days_off.push(d.clone());
                days_box2.append(&Label::new(Some(&d)));
                new_day2.set_text("");
            }
        });
    }

    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let store = store.clone();
        save.connect_clicked(move |_| {
            let mut s = store.settings.lock().unwrap();
            s.region = if region_dd.selected() == 1 { "morocco".into() } else { "none".into() };
            s.weekly_off = checks.iter().enumerate().filter(|(_, c)| c.is_active()).map(|(i, _)| i as i64).collect();
            s.save();
        });
    }
    b.append(&save);
    b
}
