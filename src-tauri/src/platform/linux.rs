//! WebKitGTK: GTK layout shim and the native signals service webviews report through.

mod layout;

use super::ServiceEvents;
use crate::services::PermissionKind;
use std::sync::Arc;
use tauri::{webview::WebviewBuilder, Runtime, Webview, Window, WindowEvent};
use webkit2gtk::{
    glib::prelude::*, GeolocationPermissionRequest, NotificationExt as _, NotificationPermissionRequest,
    PermissionRequest, PermissionRequestExt, UserMediaPermissionRequest, UserMediaPermissionRequestExt, WebView,
    WebViewExt,
};

pub fn place_shell<R: Runtime>(shell: &Webview<R>) -> tauri::Result<()> {
    shell.with_webview(|pw| layout::place_shell(&pw.inner()))
}

/// Everything is connected to the GTK webview itself, in `attach_service`.
pub fn service_builder<R: Runtime, E: ServiceEvents>(
    builder: WebviewBuilder<R>,
    _events: &Arc<E>,
) -> WebviewBuilder<R> {
    builder
}

pub fn attach_service<R: Runtime, E: ServiceEvents>(webview: &Webview<R>, events: Arc<E>) -> tauri::Result<()> {
    webview.with_webview(move |pw| {
        let view = pw.inner();
        layout::place_service(&view);
        connect_signals(&view, events);
    })
}

/// GTK sizes the webviews (see layout.rs).
pub fn on_window_event<R: Runtime>(_window: &Window<R>, _event: &WindowEvent) {}

fn permission_kind(req: &PermissionRequest) -> PermissionKind {
    if req.is::<NotificationPermissionRequest>() {
        PermissionKind::Notifications
    } else if req.is::<GeolocationPermissionRequest>() {
        PermissionKind::Geolocation
    } else if let Some(media) = req.downcast_ref::<UserMediaPermissionRequest>() {
        // Neither audio nor video: screen capture (WebKit >= 2.34), not covered by user_media.
        if media.is_for_audio_device() || media.is_for_video_device() {
            PermissionKind::UserMedia
        } else {
            PermissionKind::Other("display capture")
        }
    } else {
        PermissionKind::Other(req.type_().name())
    }
}

/// Connect the native signals the shell needs from a service webview.
fn connect_signals<E: ServiceEvents>(view: &WebView, events: Arc<E>) {
    let e = events.clone();
    view.connect_web_process_terminated(move |_, reason| e.crashed(&format!("{reason:?}")));

    if events.watches_title() {
        let e = events.clone();
        view.connect_title_notify(move |v| e.title_changed(&v.title().unwrap_or_default()));
    }

    let e = events.clone();
    view.connect_show_notification(move |_, n| {
        let text = |s: Option<webkit2gtk::glib::GString>| s.map(String::from).unwrap_or_default();
        e.notification(text(n.title()), text(n.body()));
        true // handled: no WebKit-side notification
    });

    view.connect_permission_request(move |_, req| {
        if events.permission(permission_kind(req)) {
            req.allow();
        } else {
            req.deny();
        }
        true
    });
}
