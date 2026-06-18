//! End-of-meeting prompt and the task picker, as GTK dialogs. Shown on the main
//! thread when the watcher reports a meeting ended.

use crate::store::Store;
use gtk::prelude::*;
use gtk::{glib, Align, Box as GtkBox, Button, DropDown, Entry, Label, Orientation, StringList, Window};
use std::sync::Arc;

/// Present the end-of-meeting flow. `start_ms`/`end_ms` are unix-ms wall times.
pub fn present(store: Arc<Store>, start_ms: i64, end_ms: i64) {
    let raw_h = (end_ms - start_ms) as f64 / 3_600_000.0;
    let hours = (store.billable_hours(raw_h) * 100.0).round() / 100.0;
    let start_dt = chrono::DateTime::from_timestamp_millis(start_ms).unwrap_or_else(chrono::Utc::now);
    let win_label = {
        let f = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default();
        format!("{}-{}", f(start_ms), f(end_ms))
    };
    let has_dynamic = !store.settings.lock().unwrap().dynamic_meetings.is_empty();

    let window = Window::builder().title("Meeting ended").modal(true).default_width(360).build();
    let b = GtkBox::new(Orientation::Vertical, 10);
    b.set_margin_top(14); b.set_margin_bottom(14); b.set_margin_start(14); b.set_margin_end(14);
    b.append(&Label::new(Some(&format!("Meeting ended ({}, {:.2}h)", win_label, hours))));
    b.append(&Label::new(Some("How should this be logged?")));

    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let daily = Button::with_label("Daily");
    let defined = Button::with_label("Defined list");
    let choose = Button::with_label("Choose task");
    let cancel = Button::with_label("Cancel");
    btns.append(&daily);
    if has_dynamic { btns.append(&defined); }
    btns.append(&choose);
    btns.append(&cancel);
    b.append(&btns);
    window.set_child(Some(&b));

    let date = start_dt.with_timezone(&chrono::Local).date_naive();

    {
        let store = store.clone();
        let win = window.clone();
        daily.connect_clicked(move |_| {
            let tid = store.settings.lock().unwrap().daily_task_id;
            store.log_time(tid, hours, String::new(), date);
            win.close();
        });
    }
    {
        let store = store.clone();
        let win = window.clone();
        choose.connect_clicked(move |_| {
            win.close();
            pick_task(store.clone(), hours, date);
        });
    }
    if has_dynamic {
        let store = store.clone();
        let win = window.clone();
        defined.connect_clicked(move |_| {
            win.close();
            pick_defined(store.clone(), hours, date);
        });
    }
    {
        let store = store.clone();
        let win = window.clone();
        cancel.connect_clicked(move |_| {
            store.refresh();
            win.close();
        });
    }

    window.present();
}

/// Searchable task picker + description, then log.
fn pick_task(store: Arc<Store>, hours: f64, date: chrono::NaiveDate) {
    let items: Vec<_> = store.items.lock().unwrap().iter().filter(|i| !i.is_final).cloned().collect();
    let window = Window::builder().title("Choose task").modal(true).default_width(420).build();
    let b = GtkBox::new(Orientation::Vertical, 10);
    b.set_margin_top(14); b.set_margin_bottom(14); b.set_margin_start(14); b.set_margin_end(14);

    let labels: Vec<String> = items.iter().map(|i| format!("#{} — {}", i.id, i.name)).collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let model = StringList::new(&label_refs);
    let dd = DropDown::new(Some(model), gtk::Expression::NONE);
    if let Some(idx) = items.iter().position(|i| i.id == store.settings.lock().unwrap().meetings_task_id) {
        dd.set_selected(idx as u32);
    }
    let note = Entry::builder().placeholder_text("Description (optional)").build();
    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let ok = Button::with_label("Log");
    ok.add_css_class("suggested-action");
    let cancel = Button::with_label("Cancel");
    btns.append(&cancel);
    btns.append(&ok);

    b.append(&Label::new(Some("Select the task to log this time to")));
    b.append(&dd);
    b.append(&note);
    b.append(&btns);
    window.set_child(Some(&b));

    {
        let store = store.clone();
        let win = window.clone();
        let items = items.clone();
        let (dd2, note2) = (dd.clone(), note.clone());
        ok.connect_clicked(move |_| {
            let i = dd2.selected() as usize;
            if let Some(it) = items.get(i) {
                store.log_time(it.id, hours, note2.text().to_string(), date);
            }
            win.close();
        });
    }
    {
        let win = window.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    window.present();
}

fn pick_defined(store: Arc<Store>, hours: f64, date: chrono::NaiveDate) {
    let meetings = store.settings.lock().unwrap().dynamic_meetings.clone();
    let window = Window::builder().title("Select meeting").modal(true).default_width(380).build();
    let b = GtkBox::new(Orientation::Vertical, 10);
    b.set_margin_top(14); b.set_margin_bottom(14); b.set_margin_start(14); b.set_margin_end(14);
    let labels: Vec<String> = meetings.iter().map(|m| format!("{} (#{})", m.name, m.task_id)).collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let model = StringList::new(&label_refs);
    let dd = DropDown::new(Some(model), gtk::Expression::NONE);
    let note = Entry::builder().placeholder_text("Description (editable)").build();
    if let Some(first) = meetings.first() { note.set_text(&first.description); }
    {
        let note2 = note.clone();
        let meetings2 = meetings.clone();
        dd.connect_selected_notify(move |dd| {
            if let Some(m) = meetings2.get(dd.selected() as usize) { note2.set_text(&m.description); }
        });
    }
    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let ok = Button::with_label("Log");
    ok.add_css_class("suggested-action");
    let cancel = Button::with_label("Cancel");
    btns.append(&cancel);
    btns.append(&ok);
    b.append(&dd);
    b.append(&note);
    b.append(&btns);
    window.set_child(Some(&b));
    {
        let store = store.clone();
        let win = window.clone();
        let (dd2, note2) = (dd.clone(), note.clone());
        ok.connect_clicked(move |_| {
            if let Some(m) = meetings.get(dd2.selected() as usize) {
                store.log_time(m.task_id, hours, note2.text().to_string(), date);
            }
            win.close();
        });
    }
    {
        let win = window.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    window.present();
}
