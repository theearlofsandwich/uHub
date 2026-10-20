//! macOS (WKWebView) and Windows (WebView2): child webviews honour
//! `set_bounds`, so the shell sits at x=0 with `SIDEBAR_WIDTH` and services
//! fill the rest (or, with `set_overlay`, the shell covers everything). Unlike
//! GTK nothing re-lays them out on resize, so `on_window_event` does. Crash,
//! notification and audio events and muting have no hook here yet (see the
//! module docs in mod.rs).

use super::{ServiceEvents, SIDEBAR_WIDTH};
use crate::services::PermissionKind;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{
    webview::{PermissionKind as Native, PermissionResponse, WebviewBuilder},
    LogicalPosition, LogicalSize, PhysicalSize, Rect, Runtime, Webview, Window, WindowEvent,
};

/// The shell's label (lib.rs); every other child webview is a service.
const SHELL: &str = "shell";
/// The window holding them (lib.rs); others (link windows) lay themselves out.
const MAIN: &str = "main";

/// Whether the shell covers the whole window (`set_overlay`).
static EXPANDED: AtomicBool = AtomicBool::new(false);

fn bounds(is_shell: bool, window: LogicalSize<f64>) -> Rect {
    let sidebar = f64::from(SIDEBAR_WIDTH).min(window.width);
    let (x, width) = match is_shell {
        true if EXPANDED.load(Ordering::Relaxed) => (0., window.width),
        true => (0., sidebar),
        false => (sidebar, window.width - sidebar),
    };
    Rect { position: LogicalPosition::new(x, 0.).into(), size: LogicalSize::new(width, window.height).into() }
}

fn place<R: Runtime>(webview: &Webview<R>, window: LogicalSize<f64>) -> tauri::Result<()> {
    webview.set_bounds(bounds(webview.label() == SHELL, window))
}

fn window_size<R: Runtime>(window: &Window<R>) -> tauri::Result<LogicalSize<f64>> {
    Ok(window.inner_size()?.to_logical(window.scale_factor()?))
}

pub fn place_shell<R: Runtime>(shell: &Webview<R>) -> tauri::Result<()> {
    place(shell, window_size(&shell.window())?)
}

pub fn service_builder<R: Runtime, E: ServiceEvents>(
    builder: WebviewBuilder<R>,
    events: &Arc<E>,
) -> WebviewBuilder<R> {
    let e = events.clone();
    let mut builder = builder.on_permission_request(move |wv, kind| {
        log::debug!("[{}] permission request: {kind}", wv.label());
        if e.permission(permission_kind(kind)) { PermissionResponse::Allow } else { PermissionResponse::Deny }
    });
    if events.watches_title() {
        let e = events.clone();
        builder = builder.on_document_title_changed(move |_, title| e.title_changed(&title));
    }
    builder
}

pub fn attach_service<R: Runtime, E: ServiceEvents>(
    webview: &Webview<R>,
    _events: Arc<E>,
    muted: bool,
) -> tauri::Result<()> {
    if muted {
        log::warn!("[{}] muting is not implemented on this platform", webview.label());
    }
    place(webview, window_size(&webview.window())?)
}

pub fn set_overlay<R: Runtime>(shell: &Webview<R>, open: bool) -> tauri::Result<()> {
    EXPANDED.store(open, Ordering::Relaxed);
    let window = shell.window();
    if open {
        // Later children draw on top: re-adding the shell raises it above the services.
        shell.reparent(&window)?;
    }
    place(shell, window_size(&window)?)
}

pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    let (size, scale): (PhysicalSize<u32>, f64) = match event {
        WindowEvent::Resized(size) => match window.scale_factor() {
            Ok(scale) => (*size, scale),
            Err(e) => return log::warn!("relayout: no scale factor: {e}"),
        },
        WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size, .. } => (*new_inner_size, *scale_factor),
        _ => return,
    };
    let size = size.to_logical(scale);
    for webview in window.webviews() {
        if let Err(e) = place(&webview, size) {
            log::warn!("[{}] relayout failed: {e}", webview.label());
        }
    }
}

fn permission_kind(kind: Native) -> PermissionKind {
    match kind {
        Native::Notifications => PermissionKind::Notifications,
        Native::Microphone | Native::Camera => PermissionKind::UserMedia,
        Native::Geolocation => PermissionKind::Geolocation,
        Native::DisplayCapture => PermissionKind::Other("display capture"),
        _ => PermissionKind::Other("other"),
    }
}
