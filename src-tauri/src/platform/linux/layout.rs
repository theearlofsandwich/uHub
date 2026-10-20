//! Linux layout shim.
//!
//! FINDING: on Linux, wry's `add_child` packs every child webview into the
//! window's vertical `GtkBox` (`pack_start`); `set_position`/`set_size`/
//! `set_bounds` only take effect for a `GtkFixed` parent, so they are no-ops
//! here and children simply stack vertically. To get "bar | service" (on any
//! edge) we re-parent the webviews ourselves and let GTK do the sizing. That also
//! makes window resize and HiDPI scaling GTK's problem. (Other platforms
//! honour `set_bounds`, see platform/other.rs.)
//!
//! The services go in a `GtkBox` (the row) beside a bar-sized spacer, and the
//! shell floats above that row in a `GtkOverlay`: collapsed it covers just the
//! spacer, expanded (`set_expanded`, while the shell shows a modal) the
//! whole window. Its background is transparent (lib.rs), so the service stays
//! visible behind the modal's backdrop. Input outside the shell's allocation
//! goes to the row as usual. Moving the bar (`relayout`) turns the row and
//! packs the spacer at its start (left, top) or end (right, bottom).
//!
//! These functions must run on the GTK main thread (use `Webview::with_webview`).

use crate::platform::bar;
use crate::services::BarPosition;
use gtk::prelude::*;
use webkit2gtk::WebView;

const OVERLAY_NAME: &str = "uhub-overlay";
const SPACER_NAME: &str = "uhub-bar-spacer";

/// Detach `view` from the window's vbox and return the overlay and its row,
/// created on first use. `None` if `view` isn't in the vbox (already placed).
fn take(view: &WebView) -> Option<(gtk::Overlay, gtk::Box)> {
    let vbox = view.parent()?.downcast::<gtk::Box>().ok()?;
    if vbox.parent().is_some_and(|p| p.widget_name() == OVERLAY_NAME) {
        return None;
    }
    let overlay = vbox
        .children()
        .into_iter()
        .find(|c| c.widget_name() == OVERLAY_NAME)
        .and_then(|c| c.downcast::<gtk::Overlay>().ok())
        .unwrap_or_else(|| new_overlay(&vbox));
    let row = overlay.child()?.downcast::<gtk::Box>().ok()?;
    vbox.remove(view);
    Some((overlay, row))
}

fn new_overlay(vbox: &gtk::Box) -> gtk::Overlay {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_widget_name(SPACER_NAME);
    row.pack_start(&spacer, false, false, 0);
    arrange(&row);

    let overlay = gtk::Overlay::new();
    overlay.set_widget_name(OVERLAY_NAME);
    overlay.add(&row);
    // Sized here rather than from the webview's natural size, which follows its content.
    overlay.connect_get_child_position(|overlay, child| {
        let (width, height) = (overlay.allocated_width(), overlay.allocated_height());
        if child.halign() == gtk::Align::Fill {
            return Some(gtk::Allocation::new(0, 0, width, height));
        }
        let (x, y, w, h) = bar().rect(f64::from(width), f64::from(height));
        Some(gtk::Allocation::new(x as i32, y as i32, w as i32, h as i32))
    });
    vbox.pack_start(&overlay, true, true, 0);
    overlay.show_all();
    overlay
}

pub(super) fn place_shell(view: &WebView) {
    if let Some((overlay, _)) = take(view) {
        size_shell(view);
        view.set_halign(gtk::Align::Start);
        overlay.add_overlay(view);
    }
}

pub(super) fn place_service(view: &WebView) {
    if let Some((_, row)) = take(view) {
        row.pack_start(view, true, true, 0);
    }
}

/// Size the shell's request along the bar's thickness.
fn size_shell(shell: &WebView) {
    let bar = bar();
    match bar.position.is_horizontal() {
        true => shell.set_size_request(-1, bar.thickness),
        false => shell.set_size_request(bar.thickness, -1),
    }
}

/// Turn `row` and place its spacer for the bar's position and thickness.
fn arrange(row: &gtk::Box) {
    let bar = bar();
    let Some(spacer) = row.children().into_iter().find(|c| c.widget_name() == SPACER_NAME) else {
        return;
    };
    if bar.position.is_horizontal() {
        row.set_orientation(gtk::Orientation::Vertical);
        spacer.set_size_request(-1, bar.thickness);
    } else {
        row.set_orientation(gtk::Orientation::Horizontal);
        spacer.set_size_request(bar.thickness, -1);
    }
    let pack = match bar.position {
        BarPosition::Left | BarPosition::Top => gtk::PackType::Start,
        BarPosition::Right | BarPosition::Bottom => gtk::PackType::End,
    };
    row.set_child_packing(&spacer, false, false, 0, pack);
    row.reorder_child(&spacer, 0);
}

/// The bar moved or changed size (`platform::set_bar`): lay out the row and
/// the shell again. Nothing to do before the shell is placed.
pub(super) fn relayout(shell: &WebView) {
    let Some(overlay) = shell.parent().and_then(|p| p.downcast::<gtk::Overlay>().ok()) else {
        return;
    };
    if let Some(row) = overlay.child().and_then(|c| c.downcast::<gtk::Box>().ok()) {
        arrange(&row);
    }
    size_shell(shell);
    overlay.queue_resize();
}

/// Let the shell cover the whole window (`true`) or just the bar.
pub(super) fn set_expanded(shell: &WebView, expanded: bool) {
    shell.set_halign(if expanded { gtk::Align::Fill } else { gtk::Align::Start });
    // FINDING: changing an overlay child's halign doesn't make GTK re-run the
    // overlay's allocation (the child's size request is unchanged), so
    // get-child-position isn't asked again: force it.
    if let Some(overlay) = shell.parent() {
        overlay.queue_allocate();
    }
    if expanded {
        // Keys go to the modal, not the service that had focus.
        shell.grab_focus();
    }
}
