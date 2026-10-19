//! Global shortcut that toggles the quick panel. Registration failures are
//! reported to the UI; the shortcut is never silently replaced.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub const QUICK_LABEL: &str = "quick";

#[derive(Debug, Clone, Serialize)]
pub struct ShortcutStatus {
    pub shortcut: String,
    pub registered: bool,
    pub error: Option<String>,
}

pub fn toggle_quick_panel(app: &AppHandle) {
    let Some(win) = app.get_webview_window(QUICK_LABEL) else { return };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        let _ = win.center();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Registers `shortcut`, unregistering `previous` first when given.
pub fn register(app: &AppHandle, shortcut: &str, previous: Option<&str>) -> ShortcutStatus {
    let gs = app.global_shortcut();
    if let Some(prev) = previous {
        let _ = gs.unregister(prev);
    }
    let result = gs.on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle_quick_panel(app);
        }
    });
    let status = match result {
        Ok(()) => ShortcutStatus { shortcut: shortcut.into(), registered: true, error: None },
        Err(e) => ShortcutStatus {
            shortcut: shortcut.into(),
            registered: false,
            error: Some(format!(
                "No se pudo registrar {shortcut}: {e}. Puede que otra aplicación lo esté usando; elige otro atajo en Ajustes."
            )),
        },
    };
    let _ = app.emit("jarvis://shortcut", &status);
    status
}
