//! System tray: closing the window hides it there (unless turned off in the
//! settings), the menu shows/hides the window or quits, and the total unread
//! count is shown on the icon.
//!
//! On Linux (appindicator) a click on the icon always opens the menu and no
//! click event reaches us, and tooltips are not shown: Show/Hide is therefore
//! in the menu, and the count also goes in the title next to the icon.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, Window, WindowEvent,
};

const TRAY_ID: &str = "main";
const MAIN_WINDOW: &str = "main";

/// Mirror of `Settings::close_to_tray` for the close handler, which runs on
/// the main thread and so must not lock `Services`.
struct CloseToTray(AtomicBool);

/// The unread total last shown, so unchanged snapshots cost no main-thread trip.
static SHOWN_UNREAD: AtomicU32 = AtomicU32::new(u32::MAX);

pub fn setup<R: Runtime>(app: &AppHandle<R>, close_to_tray: bool) -> tauri::Result<()> {
    app.manage(CloseToTray(AtomicBool::new(close_to_tray)));
    let toggle = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &quit])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("WebDeck")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn set_close_to_tray<R: Runtime>(app: &AppHandle<R>, on: bool) {
    if let Some(c) = app.try_state::<CloseToTray>() {
        c.0.store(on, Ordering::Relaxed);
    }
}

/// Window event handler: hide instead of close while close-to-tray is on and
/// there is a tray to get the window back from.
pub fn hide_on_close<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event
        && window.label() == MAIN_WINDOW
        && window.try_state::<CloseToTray>().is_some_and(|c| c.0.load(Ordering::Relaxed))
        && window.app_handle().tray_by_id(TRAY_ID).is_some()
    {
        api.prevent_close();
        if let Err(e) = window.hide() {
            log::error!("hiding window to tray failed: {e}");
        }
    }
}

/// Bring the main window up, focused.
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_window(MAIN_WINDOW) else { return };
    let _ = window.unminimize();
    let _ = window.show();
    if let Err(e) = window.set_focus() {
        log::warn!("focusing window failed: {e}");
    }
}

fn toggle_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_window(MAIN_WINDOW) else { return };
    let shown = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if shown {
        if let Err(e) = window.hide() {
            log::error!("hiding window failed: {e}");
        }
    } else {
        show_window(app);
    }
}

/// Show the total unread count on the tray icon. Blocks on the main thread,
/// so it must not be called from it.
pub fn show_unread<R: Runtime>(app: &AppHandle<R>, total: u32) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    if SHOWN_UNREAD.swap(total, Ordering::Relaxed) == total {
        return;
    }
    let tooltip = if total == 0 { "WebDeck".to_owned() } else { format!("WebDeck: {total} unread") };
    let title = (total > 0).then(|| total.to_string());
    if let Err(e) = tray.set_tooltip(Some(tooltip)).and_then(|()| tray.set_title(title)) {
        log::warn!("updating tray failed: {e}");
    }
}
