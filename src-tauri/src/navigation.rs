//! Native navigation policy. Everything security-relevant lives here, not in
//! injected JS. Handlers are installed on the service webview via wry's
//! WebKitGTK `decide-policy` / `create` signals.

use tauri::{
    webview::{NewWindowFeatures, NewWindowResponse},
    AppHandle, Runtime, Url,
};
use tauri_plugin_opener::OpenerExt;

pub fn host_allowed(host: &str, allowed: &[&str]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowed.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Same-site navigation policy for a service.
pub fn is_internal(url: &Url, allowed: &[&str]) -> bool {
    match url.scheme() {
        "about" | "blob" => true,
        "http" | "https" => url.host_str().is_some_and(|h| host_allowed(h, allowed)),
        _ => false,
    }
}

/// Hand a URL to the system default handler. Only web/mail schemes are forwarded
/// (never file:, javascript:, custom schemes).
pub fn open_external<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    if !matches!(url.scheme(), "http" | "https" | "mailto") {
        log::warn!("refusing to open external url with scheme {}: {url}", url.scheme());
        return;
    }
    log::info!("opening externally: {url}");
    if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
        log::error!("failed to open {url} externally: {e}");
    }
}

/// Top-level *and subframe* navigations: WebKitGTK routes every
/// NavigationAction through here and the URL carries no frame or user-gesture
/// info, so a third-party iframe (YouTube embed, sign-in widget) is
/// indistinguishable from a link click. Off-site URLs are therefore only
/// denied and logged, never opened externally — that would spawn browser tabs
/// with no click. User clicks on off-site links are turned into
/// `target=_blank` by the shared init script and arrive via [`on_new_window`].
pub fn on_navigation(
    id: String,
    allowed: &'static [&'static str],
) -> impl Fn(&Url) -> bool + Send + 'static {
    move |url| {
        if is_internal(url, allowed) {
            log::debug!("[{id}] navigate: {url}");
            true
        } else {
            log::info!("[{id}] blocked navigation to {url}");
            false
        }
    }
}

/// `target=_blank` / `window.open()` (including off-site link clicks rewritten
/// by the init script): a real user-facing new-window request, so it is sent to
/// the system browser. Never creates a window inside the app. (Breaks
/// popup-based OAuth flows — a known trade-off documented in the README.)
pub fn on_new_window<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> impl Fn(Url, NewWindowFeatures) -> NewWindowResponse<R> + Send + 'static {
    move |url, _features| {
        log::info!("[{id}] new-window request: {url}");
        open_external(&app, &url);
        NewWindowResponse::Deny
    }
}

/// The shell itself may only show the bundled app / dev server.
pub fn shell_navigation_allowed(url: &Url) -> bool {
    matches!(url.scheme(), "tauri" | "about")
        || matches!(url.host_str(), Some("localhost" | "tauri.localhost"))
}

#[cfg(test)]
mod tests {
    use super::*;
    const WA: &[&str] = &["whatsapp.com", "whatsapp.net"];

    fn internal(u: &str) -> bool {
        is_internal(&u.parse().unwrap(), WA)
    }

    #[test]
    fn allows_own_domains() {
        assert!(internal("https://web.whatsapp.com/"));
        assert!(internal("https://whatsapp.com/x"));
        assert!(internal("https://a.b.whatsapp.net/"));
        assert!(internal("about:blank"));
    }

    #[test]
    fn blocks_others() {
        assert!(!internal("https://www.bbc.co.uk/"));
        assert!(!internal("https://github.com/"));
        assert!(!internal("https://evilwhatsapp.com/"));
        assert!(!internal("https://whatsapp.com.evil.io/"));
        assert!(!internal("file:///etc/passwd"));
        assert!(!internal("javascript:alert(1)"));
    }
}
