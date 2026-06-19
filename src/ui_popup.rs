//! Tray popup — the app's primary surface, modeled on the macOS popover. Shows
//! the signed-in user, live meeting status + elapsed timer, Split/Stop, the
//! Today/Week/Month logged-hours summary, and quick actions. On GNOME the host
//! tray menu doesn't render, so this window (opened from the tray's Activate)
//! is how the user gets at everything.

use crate::command::TrayCmd;
use crate::store::Store;
use chrono::Datelike;
use gtk::prelude::*;
use gtk::{glib, Align, Box as GtkBox, Button, Label, Orientation, Window};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::Arc;

const CSS: &str = "
.ta-popup { background: @theme_bg_color; }
.ta-avatar {
  min-width: 44px; min-height: 44px;
  border-radius: 22px;
  background: #2d9d5a; color: white;
  font-weight: bold; font-size: 16px;
}
.ta-name { font-weight: bold; font-size: 15px; }
.ta-status-active { color: #d23b3b; font-weight: bold; }
.ta-status-idle { color: alpha(@theme_fg_color, 0.6); }
.ta-meeting { border: 1px solid alpha(#d23b3b, 0.4); border-radius: 10px; padding: 4px; }
.ta-meeting-idle { border: 1px solid alpha(@theme_fg_color, 0.15); border-radius: 10px; padding: 4px; }
.ta-card { border-radius: 10px; padding: 10px; background: alpha(@theme_fg_color, 0.06); }
.ta-card-today { background: alpha(#e8a33d, 0.18); }
.ta-card-week { background: alpha(#3d7de8, 0.16); }
.ta-figure { font-size: 20px; font-weight: bold; }
.ta-caption { font-size: 11px; color: alpha(@theme_fg_color, 0.65); }
";

pub struct Popup {
    pub window: Window,
    store: Arc<Store>,
    avatar: Label,
    name_lbl: Label,
    status_lbl: Label,
    meeting_box: GtkBox,
    today_lbl: Label,
    week_lbl: Label,
    month_lbl: Label,
    month_name: Label,
    footer: Label,
    month_offset: Rc<Cell<i32>>,
}

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

fn card(class: &str, figure: &Label, caption: &str) -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 2);
    b.add_css_class("ta-card");
    b.add_css_class(class);
    b.set_hexpand(true);
    let cap = Label::new(Some(caption));
    cap.add_css_class("ta-caption");
    cap.set_halign(Align::Start);
    figure.add_css_class("ta-figure");
    figure.set_halign(Align::Start);
    b.append(&cap);
    b.append(figure);
    b
}

impl Popup {
    pub fn new(store: Arc<Store>, cmd: Sender<TrayCmd>) -> Rc<std::cell::RefCell<Popup>> {
        css_once();
        let window = Window::builder()
            .title("TimeAgent")
            .decorated(false)
            .resizable(false)
            .default_width(320)
            .hide_on_close(true)
            .build();
        window.add_css_class("ta-popup");

        let root = GtkBox::new(Orientation::Vertical, 10);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);

        // header: avatar + name + status
        let header = GtkBox::new(Orientation::Horizontal, 10);
        let avatar = Label::new(Some("?"));
        avatar.add_css_class("ta-avatar");
        let head_text = GtkBox::new(Orientation::Vertical, 2);
        let name_lbl = Label::new(Some("Not signed in"));
        name_lbl.add_css_class("ta-name");
        name_lbl.set_halign(Align::Start);
        let status_lbl = Label::new(Some("Not in a meeting"));
        status_lbl.set_halign(Align::Start);
        head_text.append(&name_lbl);
        head_text.append(&status_lbl);
        header.append(&avatar);
        header.append(&head_text);
        root.append(&header);

        // meeting actions
        let meeting_box = GtkBox::new(Orientation::Horizontal, 8);
        meeting_box.set_homogeneous(true);
        meeting_box.add_css_class("ta-meeting-idle");
        let split_btn = Button::with_label("✂  Split");
        let stop_btn = Button::with_label("⏹  Stop");
        meeting_box.append(&split_btn);
        meeting_box.append(&stop_btn);
        root.append(&meeting_box);

        // today / week cards
        let cards = GtkBox::new(Orientation::Horizontal, 8);
        cards.set_homogeneous(true);
        let today_lbl = Label::new(Some("0.00h"));
        let week_lbl = Label::new(Some("0.00h"));
        cards.append(&card("ta-card-today", &today_lbl, "☀  TODAY"));
        cards.append(&card("ta-card-week", &week_lbl, "🗓  WEEK"));
        root.append(&cards);

        // month total with navigation
        let month_row = GtkBox::new(Orientation::Horizontal, 8);
        month_row.add_css_class("ta-card");
        let prev = Button::from_icon_name("go-previous-symbolic");
        prev.add_css_class("flat");
        let next = Button::from_icon_name("go-next-symbolic");
        next.add_css_class("flat");
        let month_center = GtkBox::new(Orientation::Vertical, 0);
        month_center.set_hexpand(true);
        let month_lbl = Label::new(Some("0.00h"));
        month_lbl.add_css_class("ta-figure");
        let month_name = Label::new(Some(""));
        month_name.add_css_class("ta-caption");
        month_center.append(&month_lbl);
        month_center.append(&month_name);
        month_row.append(&prev);
        month_row.append(&month_center);
        month_row.append(&next);
        root.append(&month_row);

        // open tasks
        let open_tasks = Button::with_label("🗒  Open tasks…");
        root.append(&open_tasks);

        // settings / refresh / quit
        let actions = GtkBox::new(Orientation::Horizontal, 8);
        actions.set_homogeneous(true);
        let settings_btn = Button::with_label("⚙  Settings");
        let refresh_btn = Button::with_label("⟳  Refresh");
        let quit_btn = Button::from_icon_name("system-shutdown-symbolic");
        quit_btn.set_tooltip_text(Some("Quit TimeAgent"));
        actions.append(&settings_btn);
        actions.append(&refresh_btn);
        actions.append(&quit_btn);
        root.append(&actions);

        let footer = Label::new(Some(""));
        footer.add_css_class("ta-caption");
        footer.set_halign(Align::Start);
        root.append(&footer);

        window.set_child(Some(&root));

        let month_offset = Rc::new(Cell::new(0));
        let me = Rc::new(std::cell::RefCell::new(Popup {
            window,
            store: store.clone(),
            avatar,
            name_lbl,
            status_lbl,
            meeting_box,
            today_lbl,
            week_lbl,
            month_lbl,
            month_name,
            footer,
            month_offset: month_offset.clone(),
        }));

        // button wiring (route through the shared command pump)
        let send = |cmd: &Sender<TrayCmd>, c: TrayCmd| {
            let _ = cmd.send(c);
        };
        {
            let cmd = cmd.clone();
            split_btn.connect_clicked(move |_| send(&cmd, TrayCmd::Split));
        }
        {
            let cmd = cmd.clone();
            stop_btn.connect_clicked(move |_| send(&cmd, TrayCmd::StopTracking));
        }
        {
            let cmd = cmd.clone();
            open_tasks.connect_clicked(move |_| send(&cmd, TrayCmd::OpenTasks));
        }
        {
            let cmd = cmd.clone();
            settings_btn.connect_clicked(move |_| send(&cmd, TrayCmd::OpenSettings));
        }
        {
            let cmd = cmd.clone();
            refresh_btn.connect_clicked(move |_| send(&cmd, TrayCmd::Refresh));
        }
        {
            let cmd = cmd.clone();
            quit_btn.connect_clicked(move |_| send(&cmd, TrayCmd::Quit));
        }
        // month navigation
        {
            let me2 = me.clone();
            let off = month_offset.clone();
            prev.connect_clicked(move |_| {
                off.set(off.get() - 1);
                me2.borrow().refresh();
            });
        }
        {
            let me2 = me.clone();
            let off = month_offset.clone();
            next.connect_clicked(move |_| {
                off.set(off.get() + 1);
                me2.borrow().refresh();
            });
        }

        // dismiss like a popover: hide when it loses focus
        {
            let me2 = me.clone();
            me.borrow().window.connect_is_active_notify(move |w| {
                if !w.is_active() {
                    w.set_visible(false);
                    me2.borrow().month_offset.set(0);
                }
            });
        }

        // live tick: while visible, refresh the elapsed timer + figures each second
        {
            let me2 = me.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
                if me2.borrow().window.is_visible() {
                    me2.borrow().refresh();
                }
                glib::ControlFlow::Continue
            });
        }

        me
    }

    fn target_month(&self) -> chrono::NaiveDate {
        let today = chrono::Local::now().date_naive();
        add_months(today.with_day(1).unwrap(), self.month_offset.get())
    }

    pub fn refresh(&self) {
        // identity
        let (id, name) = self.store.user_identity();
        if id != 0 && !name.is_empty() {
            self.name_lbl.set_text(&name);
            self.avatar.set_text(&initials(&name));
        }

        // meeting status — the Split/Stop box only exists while in a meeting
        match self.store.meeting_elapsed() {
            Some(d) => {
                let secs = d.as_secs();
                self.status_lbl
                    .set_text(&format!("🔴 In meeting · {}", fmt_hms(secs)));
                self.status_lbl.remove_css_class("ta-status-idle");
                self.status_lbl.add_css_class("ta-status-active");
                self.meeting_box.set_visible(true);
            }
            None => {
                self.status_lbl.set_text("Not in a meeting");
                self.status_lbl.remove_css_class("ta-status-active");
                self.status_lbl.add_css_class("ta-status-idle");
                self.meeting_box.set_visible(false);
            }
        }

        // hours
        let month = self.target_month();
        let (today, week, m) = self.store.hours_summary(month);
        self.today_lbl.set_text(&format!("{:.2}h", today));
        self.week_lbl.set_text(&format!("{:.2}h", week));
        self.month_lbl.set_text(&format!("{:.2}h", m));
        self.month_name
            .set_text(&format!("{} {}", month_name(month.month()), month.year()));

        self.footer.set_text(&self.store.last_status());
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

fn fmt_hms(secs: u64) -> String {
    format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

fn month_name(m: u32) -> &'static str {
    [
        "January", "February", "March", "April", "May", "June", "July", "August",
        "September", "October", "November", "December",
    ][(m as usize - 1).min(11)]
}

/// Shift a (day-1) date by `delta` months.
fn add_months(date: chrono::NaiveDate, delta: i32) -> chrono::NaiveDate {
    let mut y = date.year();
    let mut m = date.month() as i32 - 1 + delta;
    y += m.div_euclid(12);
    m = m.rem_euclid(12);
    chrono::NaiveDate::from_ymd_opt(y, (m + 1) as u32, 1).unwrap_or(date)
}
