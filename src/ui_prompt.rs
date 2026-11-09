//! End-of-meeting prompt and the task picker, as GTK dialogs. Shown on the main
//! thread when the watcher reports a meeting ended.

use crate::store::Store;
use gtk::prelude::*;
use gtk::{glib, Align, Box as GtkBox, Button, DropDown, Entry, Label, Orientation, StringList, Window};
use std::cell::Cell;
use std::rc::Rc;
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
        let label = win_label.clone();
        daily.connect_clicked(move |_| {
            win.close();
            log_daily(store.clone(), format!("Daily · meeting {}", label), hours, date);
        });
    }
    {
        let store = store.clone();
        let win = window.clone();
        let label = win_label.clone();
        choose.connect_clicked(move |_| {
            win.close();
            pick_task(store.clone(), hours, date, format!("Meeting {}", label));
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
fn pick_task(store: Arc<Store>, hours: f64, date: chrono::NaiveDate, title: String) {
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
    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let ok = Button::with_label("Next…");
    ok.add_css_class("suggested-action");
    let cancel = Button::with_label("Cancel");
    btns.append(&cancel);
    btns.append(&ok);

    b.append(&Label::new(Some("Select the task to log this time to")));
    b.append(&dd);
    b.append(&btns);
    window.set_child(Some(&b));

    // Hours + description go through the shared dialog, which stays open until
    // the entry saves or is explicitly discarded.
    {
        let store = store.clone();
        let win = window.clone();
        let items = items.clone();
        let dd2 = dd.clone();
        let title = title.clone();
        ok.connect_clicked(move |_| {
            let i = dd2.selected() as usize;
            if let Some(it) = items.get(i) {
                log_dialog(store.clone(), title.clone(), it.id, hours, date, String::new(), None);
            }
            win.close();
        });
    }
    {
        let win = window.clone();
        let store = store.clone();
        cancel.connect_clicked(move |_| {
            store.set_status(format!("{} ignored", title));
            win.close();
        });
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
    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let ok = Button::with_label("Next…");
    ok.add_css_class("suggested-action");
    let cancel = Button::with_label("Cancel");
    btns.append(&cancel);
    btns.append(&ok);
    b.append(&dd);
    b.append(&btns);
    window.set_child(Some(&b));
    {
        let store = store.clone();
        let win = window.clone();
        let dd2 = dd.clone();
        ok.connect_clicked(move |_| {
            if let Some(m) = meetings.get(dd2.selected() as usize) {
                // Prefill: the meeting's description, else its name.
                let desc = if m.description.trim().is_empty() { m.name.clone() } else { m.description.clone() };
                log_dialog(store.clone(), m.name.clone(), m.task_id, hours, date, desc, None);
            }
            win.close();
        });
    }
    {
        let win = window.clone();
        cancel.connect_clicked(move |_| {
            store.set_status("Meeting ignored".into());
            win.close();
        });
    }
    window.present();
}

/// Daily: if a description isn't required, try "" first and only show the
/// dialog (prefilled "Daily") on failure; if required, show it straight away.
fn log_daily(store: Arc<Store>, title: String, hours: f64, date: chrono::NaiveDate) {
    let (tid, required) = {
        let s = store.settings.lock().unwrap();
        (s.daily_task_id, s.time_description_required)
    };
    if required {
        log_dialog(store, title, tid, hours, date, "Daily".into(), None);
        return;
    }
    let rx = store.log_time(tid, hours, String::new(), date);
    glib::spawn_future_local(async move {
        if let Ok(Err(e)) = rx.recv().await {
            log_dialog(store, title, tid, hours, date, "Daily".into(), Some(e));
        }
    });
}

fn hint_text(required: bool) -> &'static str {
    if required { "Hours and description (required)." } else { "Hours and description (optional)." }
}

fn show_error(l: &Label, m: &str) {
    l.set_text(m);
    l.set_visible(true);
}

/// Hours + description form for one time entry. Stays open, showing TP's error
/// and keeping what was typed, until the entry saves or the user confirms
/// discarding it.
pub fn log_dialog(
    store: Arc<Store>,
    title: String,
    entity_id: i64,
    hours: f64,
    date: chrono::NaiveDate,
    description: String,
    error: Option<String>,
) {
    let required = store.settings.lock().unwrap().time_description_required;
    let window = Window::builder().title(title.as_str()).modal(true).default_width(380).build();
    let b = GtkBox::new(Orientation::Vertical, 8);
    b.set_margin_top(14); b.set_margin_bottom(14); b.set_margin_start(14); b.set_margin_end(14);

    let heading = Label::new(Some(&title));
    heading.add_css_class("heading");
    heading.set_halign(Align::Start);
    heading.set_wrap(true);
    let hint = Label::new(Some(hint_text(required)));
    hint.add_css_class("dim-label");
    hint.set_halign(Align::Start);
    let err_lbl = Label::new(None);
    err_lbl.add_css_class("error");
    err_lbl.set_halign(Align::Start);
    err_lbl.set_xalign(0.0);
    err_lbl.set_wrap(true);
    err_lbl.set_selectable(true);
    err_lbl.set_visible(false);
    if let Some(e) = &error { show_error(&err_lbl, e); }

    let hrs_e = Entry::builder().text(format!("{:.2}", hours).as_str()).max_width_chars(7).build();
    let hrs_row = GtkBox::new(Orientation::Horizontal, 8);
    let hl = Label::new(Some("Hours"));
    hl.set_width_chars(8);
    hl.set_halign(Align::Start);
    hrs_row.append(&hl);
    hrs_row.append(&hrs_e);
    let desc_e = Entry::builder()
        .text(description.as_str())
        .hexpand(true)
        .placeholder_text(if required { "Description (required)" } else { "Description" })
        .build();

    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    btns.append(&cancel);
    btns.append(&save);

    b.append(&heading);
    b.append(&hint);
    b.append(&err_lbl);
    b.append(&hrs_row);
    b.append(&desc_e);
    b.append(&btns);
    window.set_child(Some(&b));
    window.set_default_widget(Some(&save));

    // Set once the entry saved or the user confirmed discarding it.
    let done = Rc::new(Cell::new(false));

    {
        let (store, win, done) = (store.clone(), window.clone(), done.clone());
        let (hrs_e, desc_e, err_lbl, hint) = (hrs_e.clone(), desc_e.clone(), err_lbl.clone(), hint.clone());
        save.connect_clicked(move |btn| {
            let required = store.settings.lock().unwrap().time_description_required;
            let h: f64 = hrs_e.text().trim().replace(',', ".").parse().unwrap_or(0.0);
            if h <= 0.0 {
                show_error(&err_lbl, "Enter hours greater than 0.");
                return;
            }
            let d = desc_e.text().to_string();
            if required && d.trim().is_empty() {
                show_error(&err_lbl, "A description is required.");
                return;
            }
            btn.set_sensitive(false);
            err_lbl.set_visible(false);
            let rx = store.log_time(entity_id, h, d, date);
            let (store, win, done, btn) = (store.clone(), win.clone(), done.clone(), btn.clone());
            let (desc_e, err_lbl, hint) = (desc_e.clone(), err_lbl.clone(), hint.clone());
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(())) => {
                        done.set(true);
                        win.close();
                    }
                    Ok(Err(e)) => {
                        show_error(&err_lbl, &e);
                        // The failure may have just taught us a description is required.
                        let req = store.settings.lock().unwrap().time_description_required;
                        hint.set_text(hint_text(req));
                        if req { desc_e.set_placeholder_text(Some("Description (required)")); }
                        btn.set_sensitive(true);
                    }
                    Err(_) => btn.set_sensitive(true),
                }
            });
        });
    }
    {
        let win = window.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    // Every close path (Cancel, window X) asks before discarding.
    {
        let (store, done, hrs_e, title) = (store.clone(), done.clone(), hrs_e.clone(), title.clone());
        window.connect_close_request(move |w| {
            if done.get() {
                return glib::Propagation::Proceed;
            }
            let dialog = gtk::AlertDialog::builder()
                .message("Discard this time entry?")
                .detail(format!("{} h will not be logged to TargetProcess.", hrs_e.text()).as_str())
                .buttons(["Keep editing", "Discard"])
                .cancel_button(0)
                .default_button(0)
                .build();
            let (w2, done2, store2, title2) = (w.clone(), done.clone(), store.clone(), title.clone());
            dialog.choose(Some(w), gtk::gio::Cancellable::NONE, move |res| {
                if matches!(res, Ok(1)) {
                    done2.set(true);
                    store2.set_status(format!("Not logged: {}", title2));
                    w2.close();
                }
            });
            glib::Propagation::Stop
        });
    }
    window.present();
}
