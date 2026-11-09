//! Shared "logged / target" display for every Day / Week / Month total:
//! "32.00h / 40h", green once the target is met, red below it. A period with
//! no working days (target 0) shows the hours in grey with an "off" note.

use gtk::prelude::*;
use gtk::Label;

const CSS: &str = "
.ta-target-ok { color: #2d9d5a; }
.ta-target-miss { color: #d23b3b; }
.ta-target-off { color: alpha(@theme_fg_color, 0.55); }
";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Met,
    Short,
    Off,
}

impl Status {
    fn class(self) -> &'static str {
        match self {
            Status::Met => "ta-target-ok",
            Status::Short => "ta-target-miss",
            Status::Off => "ta-target-off",
        }
    }
}

/// "40h", or "37.5h" when the target isn't whole.
pub fn short(h: f64) -> String {
    if h == h.round() {
        format!("{:.0}h", h)
    } else {
        format!("{:.1}h", h)
    }
}

pub fn status(value: f64, target: f64) -> Status {
    if target <= 0.0 {
        Status::Off
    } else if value >= target - 0.005 {
        Status::Met
    } else {
        Status::Short
    }
}

pub fn text(value: f64, target: f64, off_label: &str) -> String {
    match status(value, target) {
        Status::Off => format!("{:.2}h · {}", value, off_label),
        _ => format!("{:.2}h / {}", value, short(target)),
    }
}

fn css_once() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(CSS);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
}

/// Set `label` to `prefix` + the formatted value and colour it by status.
pub fn apply(label: &Label, prefix: &str, value: f64, target: f64, off_label: &str) {
    css_once();
    label.set_text(&format!("{}{}", prefix, text(value, target, off_label)));
    for s in [Status::Met, Status::Short, Status::Off] {
        label.remove_css_class(s.class());
    }
    label.add_css_class(status(value, target).class());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_status() {
        assert_eq!(short(40.0), "40h");
        assert_eq!(short(37.5), "37.5h");
        assert_eq!(text(32.0, 40.0, "off"), "32.00h / 40h");
        assert_eq!(text(0.0, 0.0, "Day off"), "0.00h · Day off");
        assert_eq!(status(39.996, 40.0), Status::Met);
        assert_eq!(status(39.9, 40.0), Status::Short);
        assert_eq!(status(5.0, 0.0), Status::Off);
    }
}
