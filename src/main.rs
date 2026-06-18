//! TimeAgent — native Linux tray app (Rust + GTK4 + ksni).
//!
//! Threading model:
//!   - GTK runs on the main thread (gtk::Application).
//!   - A tokio runtime runs on a background thread for all TP HTTP + the
//!     meeting watcher.
//!   - async → UI updates flow over a glib channel (Store holds the sender;
//!     main attaches the receiver to the GTK main context).
//!   - The ksni tray runs on its own thread and forwards menu clicks over an
//!     std mpsc channel that we poll on a glib timeout.

mod holidays;
mod mic;
mod models;
mod settings;
mod store;
mod tpclient;
mod ui_prompt;
mod ui_settings;
mod ui_tasks;
mod watcher;

use gtk::prelude::*;
use gtk::{gio, glib, Application};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use store::{Store, Update};

const APP_ID: &str = "net.omnevo.timeagent";

#[derive(Clone)]
enum TrayCmd {
    OpenTasks,
    OpenSettings,
    Split,
    StopTracking,
    Refresh,
    Quit,
}

fn main() {
    // tokio runtime on a dedicated thread; we hand its Handle to the Store.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio");
    let rt_handle = rt.handle().clone();
    // Keep the runtime alive for the process lifetime.
    std::mem::forget(rt);

    let app = Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::IS_SERVICE).build();

    app.connect_activate(move |app| {
        // glib channel: async ops → GTK main loop.
        let (tx, rx) = glib::MainContext::channel::<Update>(glib::Priority::DEFAULT);
        let store = Store::new(tx, rt_handle.clone());

        // Hold open windows so they aren't dropped.
        let tasks_win: Rc<RefCell<Option<Rc<RefCell<ui_tasks::TasksWindow>>>>> = Rc::new(RefCell::new(None));
        let settings_win: Rc<RefCell<Option<gtk::Window>>> = Rc::new(RefCell::new(None));

        // Hold the app active with no visible window (tray app).
        let _hold = app.hold();

        // ---- tray (own thread) + command channel polled on a glib timeout ----
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<TrayCmd>();
        let tray = TrayMenu { in_meeting: Arc::new(std::sync::atomic::AtomicBool::new(false)), cmd: cmd_tx.clone() };
        let in_meeting_flag = tray.in_meeting.clone();
        let service = ksni::TrayService::new(tray);
        let tray_handle = service.handle();
        service.spawn();

        // ---- watcher ----
        let wh = watcher::start();
        let evt_tx_for_actions = wh.action_sender();
        let wh_inner = wh.clone_inner();

        // Forward watcher events into the Store's glib channel via the runtime.
        {
            let store_tx = store.sender();
            let inflag = in_meeting_flag.clone();
            let trayh = tray_handle.clone();
            let mut wh = wh; // move the handle (owns the events receiver)
            rt_handle.spawn(async move {
                while let Some(ev) = wh.events.recv().await {
                    match ev {
                        watcher::MeetingEvent::State { in_meeting, .. } => {
                            inflag.store(in_meeting, std::sync::atomic::Ordering::SeqCst);
                            trayh.update(|_t: &mut TrayMenu| {});
                            let _ = store_tx.send(Update::MeetingState(in_meeting));
                        }
                        watcher::MeetingEvent::Ended { start, end } => {
                            // Convert monotonic Instants to wall-clock ms.
                            let now_inst = std::time::Instant::now();
                            let now_ms = chrono::Utc::now().timestamp_millis();
                            let to_ms = |i: std::time::Instant| now_ms - now_inst.saturating_duration_since(i).as_millis() as i64;
                            let _ = store_tx.send(Update::MeetingEnded { start_ms: to_ms(start), end_ms: to_ms(end) });
                        }
                    }
                }
            });
        }

        // ---- glib channel receiver: apply updates / open prompt ----
        {
            let tasks_win = tasks_win.clone();
            let store2 = store.clone();
            let wh_inner_busy = wh_inner.clone();
            rx.attach(None, move |u| {
                match u {
                    Update::Items | Update::Times => {
                        if let Some(tw) = tasks_win.borrow().as_ref() {
                            tw.borrow().render();
                        }
                    }
                    Update::Status(_s) => { /* could surface in a window status bar */ }
                    Update::UserInfo => {}
                    Update::MeetingState(_) => {}
                    Update::MeetingEnded { start_ms, end_ms } => {
                        ui_prompt::present(store2.clone(), start_ms, end_ms);
                        watcher::clear_busy(&wh_inner_busy);
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // ---- tray command pump (glib timeout, main thread) ----
        {
            let app = app.clone();
            let store = store.clone();
            let tasks_win = tasks_win.clone();
            let settings_win = settings_win.clone();
            let evt_tx = evt_tx_for_actions.clone();
            let wh_inner = wh_inner.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        TrayCmd::Quit => { app.quit(); }
                        TrayCmd::Refresh => store.refresh(),
                        TrayCmd::Split => watcher::split_with(&wh_inner, &evt_tx),
                        TrayCmd::StopTracking => watcher::stop_with(&wh_inner, &evt_tx),
                        TrayCmd::OpenTasks => {
                            if tasks_win.borrow().is_none() {
                                let tw = ui_tasks::TasksWindow::new(store.clone());
                                *tasks_win.borrow_mut() = Some(tw);
                            }
                            if let Some(tw) = tasks_win.borrow().as_ref() {
                                tw.borrow().window.present();
                            }
                        }
                        TrayCmd::OpenSettings => {
                            let mut sw = settings_win.borrow_mut();
                            if sw.is_none() {
                                *sw = Some(ui_settings::open(store.clone()));
                            }
                            if let Some(w) = sw.as_ref() { w.present(); }
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // ---- startup ----
        store.ensure_user();
        store.refresh();
        store.log_recurring_if_due();
        if !store.settings.lock().unwrap().is_configured() {
            let _ = cmd_tx.send(TrayCmd::OpenSettings);
        }
    });

    app.run();
}

// ---- ksni tray definition ----
struct TrayMenu {
    in_meeting: Arc<std::sync::atomic::AtomicBool>,
    cmd: std::sync::mpsc::Sender<TrayCmd>,
}

impl ksni::Tray for TrayMenu {
    fn icon_name(&self) -> String {
        if self.in_meeting.load(std::sync::atomic::Ordering::SeqCst) { "appointment-soon".into() } else { "appointment-new".into() }
    }
    fn title(&self) -> String { "TimeAgent".into() }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let mut items: Vec<ksni::MenuItem<Self>> = vec![];
        if self.in_meeting.load(std::sync::atomic::Ordering::SeqCst) {
            items.push(StandardItem { label: "⏹▶ Split meeting".into(), activate: Box::new(|t: &mut TrayMenu| { let _ = t.cmd.send(TrayCmd::Split); }), ..Default::default() }.into());
            items.push(StandardItem { label: "⏹ Stop tracking".into(), activate: Box::new(|t: &mut TrayMenu| { let _ = t.cmd.send(TrayCmd::StopTracking); }), ..Default::default() }.into());
            items.push(MenuItem::Separator);
        }
        items.push(StandardItem { label: "Open tasks…".into(), activate: Box::new(|t: &mut TrayMenu| { let _ = t.cmd.send(TrayCmd::OpenTasks); }), ..Default::default() }.into());
        items.push(StandardItem { label: "Settings…".into(), activate: Box::new(|t: &mut TrayMenu| { let _ = t.cmd.send(TrayCmd::OpenSettings); }), ..Default::default() }.into());
        items.push(StandardItem { label: "Refresh".into(), activate: Box::new(|t: &mut TrayMenu| { let _ = t.cmd.send(TrayCmd::Refresh); }), ..Default::default() }.into());
        items.push(MenuItem::Separator);
        items.push(StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut TrayMenu| { let _ = t.cmd.send(TrayCmd::Quit); }), ..Default::default() }.into());
        items
    }
}

