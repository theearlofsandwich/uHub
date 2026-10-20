//! FINDING: Tauri prepends every plugin's `js_init_script` to *every* webview,
//! remote service webviews included. tauri-plugin-notification's script
//! replaces `window.Notification` with an IPC shim; in a service webview (no
//! capability) its calls are rejected, so pages could no longer show
//! notifications and WebKit would never emit `show-notification`. Plugins we
//! only drive from Rust are therefore registered without their script.

use serde_json::Value as JsonValue;
use tauri::{
    plugin::{Plugin, TauriPlugin},
    webview::PageLoadPayload,
    AppHandle, RunEvent, Runtime, Url, Webview, Window,
};

/// `plugin` with everything but its initialization script.
pub fn without_init_script<R: Runtime>(plugin: TauriPlugin<R>) -> NoInitScript<R> {
    NoInitScript(plugin)
}

pub struct NoInitScript<R: Runtime>(TauriPlugin<R>);

impl<R: Runtime> Plugin<R> for NoInitScript<R> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn initialize(&mut self, app: &AppHandle<R>, config: JsonValue) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }

    // The default `initialization_script_2` (whose type isn't public) wraps this one.
    fn initialization_script(&self) -> Option<String> {
        None
    }

    fn window_created(&mut self, window: Window<R>) {
        self.0.window_created(window)
    }

    fn webview_created(&mut self, webview: Webview<R>) {
        self.0.webview_created(webview)
    }

    fn on_navigation(&mut self, webview: &Webview<R>, url: &Url) -> bool {
        self.0.on_navigation(webview, url)
    }

    fn on_page_load(&mut self, webview: &Webview<R>, payload: &PageLoadPayload<'_>) {
        self.0.on_page_load(webview, payload)
    }

    fn on_event(&mut self, app: &AppHandle<R>, event: &RunEvent) {
        self.0.on_event(app, event)
    }

    fn extend_api(&mut self, invoke: tauri::ipc::Invoke<R>) -> bool {
        self.0.extend_api(invoke)
    }
}
