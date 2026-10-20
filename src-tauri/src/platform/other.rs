//! macOS (WKWebView) and Windows (WebView2): child webviews honour
//! `set_bounds`, so the shell sits at x=0 with `SIDEBAR_WIDTH` and services
//! fill the rest. Unlike GTK nothing re-lays them out on resize, so
//! `on_window_event` does. Crash and notification events have no hook here
//! yet (see the module docs in mod.rs).

use super::{ServiceEvents, SIDEBAR_WIDTH};
use crate::services::PermissionKind;
use std::sync::Arc;
use tauri::{
    webview::{PermissionKind as Native, PermissionResponse, WebviewBuilder},
    LogicalPosition, LogicalSize, PhysicalSize, Rect, Runtime, Webview, Window, WindowEvent,
};

/// The shell's label (lib.rs); every other child webview is a service.
const SHELL: &str = "shell";

fn bounds(is_shell: bool, window: LogicalSize<f64>) -> Rect {
    let sidebar = f64::from(SIDEBAR_WIDTH).min(window.width);
    let (x, width) = if is_shell { (0., sidebar) } else { (sidebar, window.width - sidebar) };
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

pub fn attach_service<R: Runtime, E: ServiceEvents>(webview: &Webview<R>, _events: Arc<E>) -> tauri::Result<()> {
    place(webview, window_size(&webview.window())?)
}

pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
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
