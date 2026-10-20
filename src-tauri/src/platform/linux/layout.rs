//! Linux layout shim.
//!
//! FINDING: on Linux, wry's `add_child` packs every child webview into the
//! window's vertical `GtkBox` (`pack_start`); `set_position`/`set_size`/
//! `set_bounds` only take effect for a `GtkFixed` parent, so they are no-ops
//! here and children simply stack vertically. To get "220px sidebar | service"
//! we re-parent both webviews into one horizontal `GtkBox` and let GTK do the
//! sizing. That also makes window resize and HiDPI scaling GTK's problem.
//! (Other platforms honour `set_bounds`, see platform/other.rs.)
//!
//! These functions must run on the GTK main thread (use `Webview::with_webview`).

use crate::platform::SIDEBAR_WIDTH;
use gtk::prelude::*;
use webkit2gtk::WebView;

const ROW_NAME: &str = "webdeck-row";

/// Detach `view` from the window's vbox (if still there) and return the shared row.
fn take_into_row(view: &WebView) -> Option<gtk::Box> {
    let parent = view.parent()?.downcast::<gtk::Box>().ok()?;
    if parent.widget_name() == ROW_NAME {
        parent.remove(view);
        return Some(parent);
    }
    let vbox = parent;
    let row = vbox
        .children()
        .into_iter()
        .find(|c| c.widget_name() == ROW_NAME)
        .and_then(|c| c.downcast::<gtk::Box>().ok())
        .unwrap_or_else(|| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.set_widget_name(ROW_NAME);
            vbox.pack_start(&row, true, true, 0);
            row.show();
            row
        });
    vbox.remove(view);
    Some(row)
}

pub(super) fn place_shell(view: &WebView) {
    if let Some(row) = take_into_row(view) {
        view.set_size_request(SIDEBAR_WIDTH, -1);
        row.pack_start(view, false, false, 0);
    }
}

pub(super) fn place_service(view: &WebView) {
    if let Some(row) = take_into_row(view) {
        row.pack_start(view, true, true, 0);
    }
}
