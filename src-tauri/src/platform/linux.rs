//! WebKitGTK: GTK layout shim and the native signals service webviews report through.

mod layout;

use super::{text_on, ServiceEvents};
use crate::services::PermissionKind;
use gtk::prelude::*;
use std::{
    cell::{OnceCell, RefCell},
    ptr,
    rc::Rc,
    sync::Arc,
};
use tauri::{webview::WebviewBuilder, AppHandle, Runtime, Webview, Window, WindowEvent};
use webkit2gtk::{
    gio,
    glib::{gobject_ffi, translate::IntoGlib, StaticType},
    ContextMenu, ContextMenuAction, ContextMenuExt, ContextMenuItem, ContextMenuItemExt, GeolocationPermissionRequest,
    HitTestResult, HitTestResultExt, NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType,
    NotificationExt as _, NotificationPermissionRequest, PermissionRequest, PermissionRequestExt, PolicyDecisionExt,
    PolicyDecisionType, URIRequestExt, UserMediaPermissionRequest, UserMediaPermissionRequestExt, WebView, WebViewExt,
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

pub fn attach_service<R: Runtime, E: ServiceEvents>(
    webview: &Webview<R>,
    events: Arc<E>,
    muted: bool,
) -> tauri::Result<()> {
    webview.with_webview(move |pw| {
        let view = pw.inner();
        layout::place_service(&view);
        view.set_is_muted(muted);
        connect_signals(&view, events);
    })
}

pub fn set_muted<R: Runtime>(webview: &Webview<R>, muted: bool) -> tauri::Result<()> {
    webview.with_webview(move |pw| pw.inner().set_is_muted(muted))
}

pub fn relayout<R: Runtime>(shell: &Webview<R>) -> tauri::Result<()> {
    shell.with_webview(|pw| layout::relayout(&pw.inner()))
}

pub fn set_overlay<R: Runtime>(shell: &Webview<R>, open: bool) -> tauri::Result<()> {
    shell.with_webview(move |pw| layout::set_expanded(&pw.inner(), open))
}

thread_local! {
    /// The title bar's style, added to the screen the first time it is set.
    static TITLE_BAR: OnceCell<gtk::CssProvider> = const { OnceCell::new() };
}

/// GTK CSS for every window's title bar, focused or not (`:backdrop`). It
/// only reaches a title bar GTK draws itself (see mod.rs).
pub fn set_title_bar_color<R: Runtime>(app: &AppHandle<R>, color: &str) -> tauri::Result<()> {
    let text = text_on(color);
    let css = format!(
        ".titlebar, .titlebar:backdrop {{ background: {color}; background-image: none; color: {text}; \
         border-color: {color}; box-shadow: none; }}\n\
         .titlebar button, .titlebar .title {{ color: {text}; }}\n\
         .titlebar:backdrop .title {{ opacity: .7; }}"
    );
    app.run_on_main_thread(move || {
        TITLE_BAR.with(|cell| {
            let provider = cell.get_or_init(|| {
                let provider = gtk::CssProvider::new();
                match gtk::gdk::Screen::default() {
                    Some(screen) => gtk::StyleContext::add_provider_for_screen(
                        &screen,
                        &provider,
                        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                    ),
                    None => log::warn!("title bar colour: no screen"),
                }
                provider
            });
            if let Err(e) = provider.load_from_data(css.as_bytes()) {
                log::warn!("title bar colour: {e}");
            }
        })
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

/// A URL "Open in uHub" is loading, let through by the navigation policy
/// once. Main thread only, like every signal handler.
type Requested = Rc<RefCell<Option<String>>>;

/// On a link the service offers it for, put "Open in uHub" right after
/// WebKit's "Open Link in New Window" (or first, if that is missing). It
/// loads the link in this webview, as following a link without a target does.
fn add_open_in_app<E: ServiceEvents>(
    view: &WebView,
    menu: &ContextMenu,
    hit: &HitTestResult,
    events: &Arc<E>,
    requested: &Requested,
) {
    if !hit.context_is_link() {
        return;
    }
    let Some(url) = hit.link_uri().map(String::from) else { return };
    if !events.offers_open_in_app(&url) {
        return;
    }
    let action = gio::SimpleAction::new("open-in-uhub", None);
    let (view, requested) = (view.downgrade(), requested.clone());
    action.connect_activate(move |_, _| {
        if let Some(view) = view.upgrade() {
            requested.replace(Some(url.clone()));
            view.load_uri(&url);
        }
    });
    let item = ContextMenuItem::from_gaction(&action, "Open in uHub", None);
    let at = menu
        .items()
        .iter()
        .position(|i| i.stock_action() == ContextMenuAction::OpenLinkInNewWindow)
        .map_or(0, |i| i + 1);
    menu.insert(&item, at as i32);
}

/// Take over the navigation policy from wry. Its `decide-policy` handler (the
/// builder's `on_navigation`) only sees the URL and stops the emission, so it
/// is disconnected; this one also sees how the navigation came about. Until
/// now wry's applied, which never lets the page off-site.
fn decide_navigations<E: ServiceEvents>(view: &WebView, events: Arc<E>, requested: Requested) {
    // SAFETY: a live WebView and a signal its type has.
    let removed = unsafe {
        let signal = gobject_ffi::g_signal_lookup(c"decide-policy".as_ptr(), WebView::static_type().into_glib());
        gobject_ffi::g_signal_handlers_disconnect_matched(
            view.as_ptr().cast(),
            gobject_ffi::G_SIGNAL_MATCH_ID,
            signal,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if removed != 1 {
        log::warn!("expected one decide-policy handler from wry, removed {removed}");
    }

    view.connect_decide_policy(move |_, decision, kind| {
        if kind != PolicyDecisionType::NavigationAction {
            return false;
        }
        let Some(mut action) = decision.downcast_ref::<NavigationPolicyDecision>().and_then(|d| d.navigation_action())
        else {
            return false;
        };
        let Some(url) = action.request().and_then(|r| r.uri()) else { return false };
        let asked = requested.take().is_some_and(|u| u == url.as_str());
        // Things the user did; iframes loading and scripts navigating are
        // `Other` without a gesture.
        let followed = asked
            || action.is_redirect()
            || match action.navigation_type() {
                NavigationType::LinkClicked => action.is_user_gesture(),
                NavigationType::BackForward | NavigationType::Reload => true,
                _ => false,
            };
        if events.navigation(&url, followed) {
            decision.use_();
        } else {
            decision.ignore();
        }
        true
    });
}

/// Connect the native signals the shell needs from a service webview.
fn connect_signals<E: ServiceEvents>(view: &WebView, events: Arc<E>) {
    let requested = Requested::default();
    decide_navigations(view, events.clone(), requested.clone());

    let e = events.clone();
    view.connect_web_process_terminated(move |_, reason| e.crashed(&format!("{reason:?}")));

    if events.watches_title() {
        let e = events.clone();
        view.connect_title_notify(move |v| e.title_changed(&v.title().unwrap_or_default()));
    }

    if events.watches_audio() {
        let e = events.clone();
        view.connect_is_playing_audio_notify(move |v| e.playing_audio(v.is_playing_audio()));
    }

    let e = events.clone();
    view.connect_context_menu(move |view, menu, _, hit| {
        add_open_in_app(view, menu, hit, &e, &requested);
        false // show the (amended) menu
    });

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
