//! Platform-specific webview layout and native service events, behind one
//! interface so webviews.rs and lib.rs need no cfgs.
//!
//! - **Linux** (linux.rs): WebKitGTK. Layout is a GTK re-parenting shim (see
//!   linux/layout.rs); crash, title, notification, permission and
//!   playing-audio events come from WebKitGTK signals, and muting is native.
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
//! - **Muting** (`Instance::audio` off): WebView2 has `ICoreWebView2_8::put_IsMuted`
//!   and `add_IsDocumentPlayingAudioChanged` (for `ServiceEvents::playing_audio`);
//!   WKWebView has no public API for either. Not hooked up yet: services play
//!   sound, and no media indicator is shown.
//! - **The shell over a service** (`set_overlay`, for modals): the shell is
//!   resized to the window and re-added to raise it over the services, but its
//!   transparency needs tauri's `macos-private-api` on macOS. Untested.
//! - **Unread badges** work everywhere (`on_document_title_changed` on non-Linux).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(not(target_os = "linux"))]
mod other;
#[cfg(not(target_os = "linux"))]
use other as imp;

use crate::services::{BarPosition, IconSize, PermissionKind};
use std::sync::{Arc, Mutex};
use tauri::{webview::WebviewBuilder, Runtime, Webview, Window, WindowEvent};

/// Where the shell (the app bar) sits: the window edge and its thickness in
/// logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub position: BarPosition,
    pub thickness: i32,
}

impl Bar {
    pub fn new(position: BarPosition, icon_size: IconSize) -> Self {
        Self { position, thickness: bar_thickness(icon_size) }
    }

    /// The bar's (x, y, width, height) in a window of `width` x `height`,
    /// clamped to it.
    pub fn rect(self, width: f64, height: f64) -> (f64, f64, f64, f64) {
        let t = f64::from(self.thickness);
        match self.position {
            BarPosition::Left => (0., 0., t.min(width), height),
            BarPosition::Right => ((width - t).max(0.), 0., t.min(width), height),
            BarPosition::Top => (0., 0., width, t.min(height)),
            BarPosition::Bottom => (0., (height - t).max(0.), width, t.min(height)),
        }
    }
}

/// The bar's thickness for icons of `size`: the icon plus the item's and the
/// bar's padding. The shell's CSS computes the same as `--bar-size`
/// (src/App.css), for when it covers the whole window (`set_overlay`).
fn bar_thickness(size: IconSize) -> i32 {
    let icon = match size {
        IconSize::ExtraSmall => 16,
        IconSize::Small => 20,
        IconSize::Medium => 28,
        IconSize::Large => 36,
        IconSize::ExtraLarge => 44,
    };
    icon + 2 * 8 + 2 * 6
}

static BAR: Mutex<Bar> = Mutex::new(Bar { position: BarPosition::Left, thickness: 56 });

/// The current bar layout (`set_bar`).
pub fn bar() -> Bar {
    *BAR.lock().unwrap_or_else(|e| e.into_inner())
}

/// Move or resize the bar. Before the shell exists (at startup) this only
/// records where `place_shell` puts it; afterwards pass the shell to lay out
/// the window again.
pub fn set_bar<R: Runtime>(bar: Bar, shell: Option<&Webview<R>>) -> tauri::Result<()> {
    *BAR.lock().unwrap_or_else(|e| e.into_inner()) = bar;
    match shell {
        Some(shell) => imp::relayout(shell),
        None => Ok(()),
    }
}

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
    /// Whether to report when the page starts or stops playing audio.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "no non-Linux hook yet, see module docs"))]
    fn watches_audio(&self) -> bool;
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "no non-Linux hook yet, see module docs"))]
    fn playing_audio(&self, playing: bool);
}

/// Lay out the shell as the app bar, at the edge `set_bar` chose.
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

/// Lay out a just-created service webview beside the bar, mute it if
/// `muted`, and connect the event hooks that need the native webview.
pub fn attach_service<R: Runtime, E: ServiceEvents>(
    webview: &Webview<R>,
    events: Arc<E>,
    muted: bool,
) -> tauri::Result<()> {
    imp::attach_service(webview, events, muted)
}

/// Mute or unmute a live service webview.
pub fn set_muted<R: Runtime>(webview: &Webview<R>, muted: bool) -> tauri::Result<()> {
    imp::set_muted(webview, muted)
}

/// Let the shell cover the whole window, above the service (`open`, while it
/// shows a modal), or go back to being the bar.
pub fn set_overlay<R: Runtime>(shell: &Webview<R>, open: bool) -> tauri::Result<()> {
    imp::set_overlay(shell, open)
}

/// Window event handler: keeps the layout in step with the window's size.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    imp::on_window_event(window, event)
}
