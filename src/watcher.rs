//! Meeting detection loop on a tokio interval (not a fragile JS timer). Polls
//! the mic in-process; emits state changes + meeting-ended events. The loop body
//! is fully guarded and the interval always ticks, so it self-heals.

use crate::mic;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub enum MeetingEvent {
    State { in_meeting: bool, session_start: Option<Instant> },
    Ended { start: Instant, end: Instant },
}

#[derive(Default)]
pub struct WatcherState {
    session_start: Option<Instant>,
    last_seen: Option<Instant>,
    suppressed: bool,
    suppressed_at: Option<Instant>,
    busy: bool,
}

pub struct WatcherHandle {
    pub events: mpsc::UnboundedReceiver<MeetingEvent>,
    tx: mpsc::UnboundedSender<MeetingEvent>,
    inner: Arc<Mutex<WatcherState>>,
}

const MIN_SECS: u64 = 60;
const IDLE: Duration = Duration::from_secs(8);
const ACTIVE: Duration = Duration::from_secs(3);
const SUPPRESS_MAX: Duration = Duration::from_secs(90);

pub fn start(rt: &tokio::runtime::Handle) -> WatcherHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let inner = Arc::new(Mutex::new(WatcherState::default()));
    let st = inner.clone();
    let tx2 = tx.clone();
    // Spawn via the Handle: start() runs on the GTK main thread, which is not
    // inside the runtime context that the free `tokio::spawn` requires.
    rt.spawn(async move {
        loop {
            let in_meeting = st.lock().unwrap().session_start.is_some();
            tokio::time::sleep(if in_meeting { ACTIVE } else { IDLE }).await;
            let active = tokio::task::spawn_blocking(mic::mic_in_use).await.unwrap_or(false);
            poll(&st, &tx2, active);
        }
    });
    WatcherHandle { events: rx, tx, inner }
}

fn poll(st: &Arc<Mutex<WatcherState>>, tx: &mpsc::UnboundedSender<MeetingEvent>, active: bool) {
    let now = Instant::now();
    let mut s = st.lock().unwrap();
    if s.busy {
        return;
    }
    if s.suppressed {
        let expired = s.suppressed_at.map(|a| now.duration_since(a) > SUPPRESS_MAX).unwrap_or(true);
        if !active || expired {
            s.suppressed = false;
            s.suppressed_at = None;
        } else {
            let _ = tx.send(MeetingEvent::State { in_meeting: false, session_start: None });
            return;
        }
    }
    if active {
        if s.session_start.is_none() {
            s.session_start = Some(now);
            let _ = tx.send(MeetingEvent::State { in_meeting: true, session_start: Some(now) });
        }
        s.last_seen = Some(now);
    } else if let Some(start) = s.session_start {
        let seen = s.last_seen.unwrap_or(start);
        s.session_start = None;
        s.last_seen = None;
        let _ = tx.send(MeetingEvent::State { in_meeting: false, session_start: None });
        let end = if now.duration_since(seen) > IDLE * 3 { seen } else { now };
        if end.duration_since(start).as_secs() >= MIN_SECS {
            s.busy = true;
            let _ = tx.send(MeetingEvent::Ended { start, end });
        }
    }
}

impl WatcherHandle {
    pub fn action_sender(&self) -> mpsc::UnboundedSender<MeetingEvent> {
        self.tx.clone()
    }
    pub fn clone_inner(&self) -> Arc<Mutex<WatcherState>> {
        self.inner.clone()
    }
}

pub fn clear_busy(inner: &Arc<Mutex<WatcherState>>) {
    inner.lock().unwrap().busy = false;
}

fn split_state(inner: &Arc<Mutex<WatcherState>>, tx: &mpsc::UnboundedSender<MeetingEvent>, suppress: bool) {
    let now = Instant::now();
    let mut s = inner.lock().unwrap();
    if let Some(start) = s.session_start.take() {
        s.last_seen = None;
        if suppress {
            s.suppressed = true;
            s.suppressed_at = Some(now);
        }
        let _ = tx.send(MeetingEvent::State { in_meeting: false, session_start: None });
        if now.duration_since(start).as_secs() >= MIN_SECS {
            s.busy = true;
            let _ = tx.send(MeetingEvent::Ended { start, end: now });
        }
    }
}

// Convenience for main: operate on inner directly.
pub fn split_with(inner: &Arc<Mutex<WatcherState>>, tx: &mpsc::UnboundedSender<MeetingEvent>) {
    split_state(inner, tx, false);
}
pub fn stop_with(inner: &Arc<Mutex<WatcherState>>, tx: &mpsc::UnboundedSender<MeetingEvent>) {
    split_state(inner, tx, true);
}
