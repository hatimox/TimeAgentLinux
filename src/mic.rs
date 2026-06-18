//! Microphone-in-use detection via PulseAudio / PipeWire's pulse shim, queried
//! in-process through libpulse (no `pactl` subprocess — that per-poll spawn was
//! the Electron Linux freeze source). We enumerate source-outputs (capture
//! streams) and report "in use" if any live capture is reading from a real input
//! (non-`.monitor`) source.
//!
//! libpulse's API is callback/mainloop based; we run a short-lived threaded
//! mainloop, gather the data, and return a bool. This is synchronous and cheap.

use libpulse_binding as pulse;
use pulse::context::{Context, FlagSet as CtxFlags};
use pulse::mainloop::threaded::Mainloop;
use std::cell::RefCell;
use std::rc::Rc;

pub fn mic_in_use() -> bool {
    match query() {
        Ok(v) => v,
        Err(_) => false, // unknown -> treat as not in use; process fallback could be added
    }
}

fn query() -> Result<bool, String> {
    let mut ml = Mainloop::new().ok_or("no mainloop")?;
    let mut ctx = Context::new(&ml, "TimeAgent").ok_or("no context")?;
    ctx.connect(None, CtxFlags::NOFLAGS, None).map_err(|e| e.to_string())?;

    ml.start().map_err(|e| e.to_string())?;

    // Wait for the context to be ready.
    loop {
        match ctx.get_state() {
            pulse::context::State::Ready => break,
            pulse::context::State::Failed | pulse::context::State::Terminated => {
                ml.stop();
                return Err("context failed".into());
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }

    // 1) Collect monitor source indices (so we can exclude desktop-audio capture).
    let monitors: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(vec![]));
    let done1 = Rc::new(RefCell::new(false));
    {
        let monitors = monitors.clone();
        let done = done1.clone();
        let intro = ctx.introspect();
        intro.get_source_info_list(move |res| match res {
            pulse::callbacks::ListResult::Item(item) => {
                let is_monitor = item.monitor_of_sink.is_some()
                    || item.name.as_deref().map(|n| n.ends_with(".monitor")).unwrap_or(false);
                if is_monitor {
                    monitors.borrow_mut().push(item.index);
                }
            }
            pulse::callbacks::ListResult::End | pulse::callbacks::ListResult::Error => {
                *done.borrow_mut() = true;
            }
        });
    }
    wait_for(&done1);

    // 2) Any live (non-corked) source-output from a non-monitor source = mic in use.
    let active = Rc::new(RefCell::new(false));
    let done2 = Rc::new(RefCell::new(false));
    {
        let active = active.clone();
        let monitors = monitors.clone();
        let done = done2.clone();
        let intro = ctx.introspect();
        intro.get_source_output_info_list(move |res| match res {
            pulse::callbacks::ListResult::Item(item) => {
                let from_monitor = monitors.borrow().contains(&item.source);
                if !item.corked && !from_monitor {
                    *active.borrow_mut() = true;
                }
            }
            pulse::callbacks::ListResult::End | pulse::callbacks::ListResult::Error => {
                *done.borrow_mut() = true;
            }
        });
    }
    wait_for(&done2);

    ml.stop();
    let v = *active.borrow();
    Ok(v)
}

fn wait_for(done: &Rc<RefCell<bool>>) {
    let start = std::time::Instant::now();
    while !*done.borrow() {
        if start.elapsed() > std::time::Duration::from_secs(4) {
            break; // hard cap so a stuck server can't hang us
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
