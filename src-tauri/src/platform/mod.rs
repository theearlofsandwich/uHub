//! Platform-specific webview layout and native service events, behind one
//! interface so webviews.rs and lib.rs need no cfgs.
//!
//! - **Linux** (linux.rs): WebKitGTK. Layout is a GTK re-parenting shim (see
//!   linux/layout.rs); crash, title, notification and permission events come
//!   from WebKitGTK signals.
//! - **macOS / Windows** (other.rs): WKWebView / WebView2. Child webviews are
//!   placed with `set_bounds` and re-placed when the window resizes. Title and
//!   permission events come from tauri's cross-platform builder hooks.
//!
//! Not implemented outside Linux yet, and where it would hook in:
//!
//! - **Crash detection** (`ServiceEvents::crashed`): tauri has no hook. WebView2:
//!   `ICoreWebView2::add_ProcessFailed` (render process exited/unresponsive), via
//!   `with_webview` + `controller().CoreWebView2()`. WKWebView:
//!   `WKNavigationDelegate webViewWebContentProcessDidTerminate:`, but wry owns
//!   the navigation delegate, so this needs wry support (or wrapping its delegate).
//!   Until then a crashed service just shows a blank or "sad" page and stays ACTIVE.
//! - **Notifications** (`ServiceEvents::notification`): WebView2's
//!   `ICoreWebView2_24::add_NotificationReceived` (newer runtimes) would let us
//!   take over; without it, a service whose policy allows notifications gets
//!   WebView2's own notification UI (no "Service: title" prefix, clicking it
//!   doesn't switch to the service). WKWebView on macOS doesn't expose the Web
//!   Notifications API to pages at all; it would need an init-script shim for
//!   `window.Notification` plus a channel back that isn't IPC (service pages
//!   have none by design).
//! - **Permissions**: the policy (`Service::allows`) is applied through tauri's
//!   `on_permission_request`, which only sees what wry forwards. WebView2 forwards
//!   everything. WKWebView only forwards camera/microphone; geolocation follows
//!   the system's location prompt (and needs an Info.plist usage string), and
//!   there are no web notifications to ask for.
//! - **Unread badges** work everywhere (`on_document_title_changed` on non-Linux).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(not(target_os = "linux"))]
mod other;
#[cfg(not(target_os = "linux"))]
use other as imp;

use crate::services::PermissionKind;
use std::sync::Arc;
use tauri::{webview::WebviewBuilder, Runtime, Webview, Window, WindowEvent};

/// Width of the shell (sidebar) webview in logical pixels. The shell's CSS
/// doesn't repeat it: the sidebar fills its own webview's viewport.
pub const SIDEBAR_WIDTH: i32 = 220;

/// What a service webview reports through native events. All methods are
/// called on the main thread, so they must not wait for the registry lock.
pub trait ServiceEvents: Send + Sync + 'static {
    /// Whether to watch the page title at all.
    fn watches_title(&self) -> bool;
    fn title_changed(&self, title: &str);
    /// The web process died; `reason` is only for the log.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "no non-Linux hook yet, see module docs"))]
    fn crashed(&self, reason: &str);
    /// A web notification the page shows.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "no non-Linux hook yet, see module docs"))]
    fn notification(&self, title: String, body: String);
    /// Decide a permission request: `true` to allow.
    fn permission(&self, kind: PermissionKind) -> bool;
}

/// Lay out the shell as the sidebar, at the window's left edge.
pub fn place_shell<R: Runtime>(shell: &Webview<R>) -> tauri::Result<()> {
    imp::place_shell(shell)
}

/// Builder-time hooks for a service webview's events (before `add_child`).
pub fn service_builder<R: Runtime, E: ServiceEvents>(
    builder: WebviewBuilder<R>,
    events: &Arc<E>,
) -> WebviewBuilder<R> {
    imp::service_builder(builder, events)
}

/// Lay out a just-created service webview right of the sidebar and connect the
/// event hooks that need the native webview.
pub fn attach_service<R: Runtime, E: ServiceEvents>(webview: &Webview<R>, events: Arc<E>) -> tauri::Result<()> {
    imp::attach_service(webview, events)
}

/// Window event handler: keeps the layout in step with the window's size.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    imp::on_window_event(window, event)
}
