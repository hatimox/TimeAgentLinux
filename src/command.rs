//! Commands sent from the tray menu and app windows to the main-loop command
//! pump (see main.rs). Shared so windows can trigger the same actions as the
//! tray — important on hosts where the tray's own menu doesn't render.

#[derive(Clone)]
pub enum TrayCmd {
    /// Show the popup, anchored near the given screen coords (the tray click).
    ShowPopup { x: i32, y: i32 },
    OpenTasks,
    OpenSettings,
    Split,
    StopTracking,
    Refresh,
    Quit,
}
