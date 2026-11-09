//! Day entries window (the Linux counterpart of TodayView.swift): every time
//! entry logged on one day across all tasks, editable / deletable inline, with
//! ‹ › to browse days (never past today).

use crate::store::Store;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Entry, Label, Orientation, PolicyType, ScrolledWindow, Window};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

pub struct DayWindow {
    pub window: Window,
    store: Arc<Store>,
    /// 0 = today, -1 = yesterday, …
    offset: Rc<Cell<i32>>,
    title: Label,
    total: Label,
    next: Button,
    today_btn: Button,
    list: GtkBox,
}

impl DayWindow {
    pub fn new(store: Arc<Store>) -> Rc<RefCell<DayWindow>> {
        let window = Window::builder().title("TimeAgent — Day").default_width(620).default_height(440).hide_on_close(true).build();
        let root = GtkBox::new(Orientation::Vertical, 10);
        root.set_margin_top(12); root.set_margin_bottom(12); root.set_margin_start(12); root.set_margin_end(12);

        let head = GtkBox::new(Orientation::Horizontal, 6);
        let prev = Button::from_icon_name("go-previous-symbolic");
        prev.set_tooltip_text(Some("Previous day"));
        let next = Button::from_icon_name("go-next-symbolic");
        next.set_tooltip_text(Some("Next day"));
        let title = Label::new(Some("Today"));
        title.add_css_class("title-3");
        let today_btn = Button::with_label("Today");
        let total = Label::new(Some("0.00h"));
        total.add_css_class("title-3");
        total.set_hexpand(true);
        total.set_halign(Align::End);
        head.append(&prev);
        head.append(&next);
        head.append(&title);
        head.append(&today_btn);
        head.append(&total);
        root.append(&head);

        let list = GtkBox::new(Orientation::Vertical, 6);
        let scroller = ScrolledWindow::builder().vexpand(true).hscrollbar_policy(PolicyType::Never).child(&list).build();
        root.append(&scroller);
        window.set_child(Some(&root));

        let offset = Rc::new(Cell::new(0));
        let me = Rc::new(RefCell::new(DayWindow { window, store, offset: offset.clone(), title, total, next: next.clone(), today_btn: today_btn.clone(), list }));

        {
            let (me2, off) = (me.clone(), offset.clone());
            prev.connect_clicked(move |_| {
                off.set(off.get() - 1);
                me2.borrow().render();
            });
        }
        {
            let (me2, off) = (me.clone(), offset.clone());
            next.connect_clicked(move |_| {
                if off.get() < 0 {
                    off.set(off.get() + 1);
                }
                me2.borrow().render();
            });
        }
        {
            let (me2, off) = (me.clone(), offset.clone());
            today_btn.connect_clicked(move |_| {
                off.set(0);
                me2.borrow().render();
            });
        }
        me.borrow().render();
        me
    }

    /// Show the given day (0 = today, negative = past) and bring the window up.
    pub fn show_day(&self, offset: i32) {
        self.offset.set(offset.min(0));
        self.render();
        self.window.present();
    }

    pub fn render(&self) {
        let off = self.offset.get();
        let today = self.store.today();
        let day = crate::periods::day(today, off).start.format("%Y-%m-%d").to_string();
        self.title.set_text(&crate::periods::long_day_label(today, off));
        self.next.set_sensitive(off < 0);
        self.today_btn.set_visible(off != 0);

        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut entries: Vec<_> = self.store.times.lock().unwrap().iter().filter(|t| t.day == day).cloned().collect();
        entries.sort_by(|a, b| b.id.cmp(&a.id));
        let total: f64 = entries.iter().map(|e| e.hours).sum();
        self.total.set_text(&format!("{:.2}h", total));

        if entries.is_empty() {
            let l = Label::new(Some(if off == 0 { "No time logged today" } else { "No time logged on this day" }));
            l.add_css_class("dim-label");
            l.set_vexpand(true);
            self.list.append(&l);
            return;
        }
        for e in entries {
            let r = GtkBox::new(Orientation::Horizontal, 6);
            let name = self.store.item_name(e.item_id).unwrap_or_else(|| "(not in current list)".to_string());
            let link = Button::with_label(&format!("#{} — {}", e.item_id, name));
            link.add_css_class("flat");
            link.set_tooltip_text(Some("Open in TargetProcess"));
            if let Some(lbl) = link.child().and_downcast::<Label>() {
                lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lbl.set_max_width_chars(28);
            }
            {
                let store = self.store.clone();
                let id = e.item_id;
                link.connect_clicked(move |_| store.open_in_tp(id));
            }
            let hrs = Entry::builder().text(format!("{}", e.hours).as_str()).max_width_chars(5).build();
            let note = Entry::builder().text(e.description.as_str()).hexpand(true).build();
            let save = Button::with_label("Save");
            save.add_css_class("suggested-action");
            {
                let store = self.store.clone();
                let (hrs2, note2) = (hrs.clone(), note.clone());
                let (tid, d) = (e.id, e.day.clone());
                save.connect_clicked(move |_| {
                    let h: f64 = hrs2.text().replace(',', ".").parse().unwrap_or(0.0);
                    if h > 0.0 {
                        store.update_time(tid, h, note2.text().to_string(), d.clone());
                    }
                });
            }
            let del = Button::from_icon_name("user-trash-symbolic");
            del.add_css_class("destructive-action");
            {
                let store = self.store.clone();
                let tid = e.id;
                let win = self.window.clone();
                del.connect_clicked(move |_| {
                    let dialog = gtk::AlertDialog::builder()
                        .message("Delete this time entry?")
                        .buttons(["Cancel", "Delete"])
                        .cancel_button(0)
                        .default_button(0)
                        .build();
                    let store2 = store.clone();
                    dialog.choose(Some(&win), gtk::gio::Cancellable::NONE, move |res| {
                        if matches!(res, Ok(1)) {
                            store2.delete_time(tid);
                        }
                    });
                });
            }
            r.append(&link);
            r.append(&hrs);
            r.append(&note);
            r.append(&save);
            r.append(&del);
            self.list.append(&r);
        }
    }
}
