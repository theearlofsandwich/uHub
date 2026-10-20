import { LogicalPosition } from "@tauri-apps/api/dpi";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { BarPosition, Service } from "../services/types";

export interface ServiceMenuActions {
  home: () => void;
  reload: () => void;
  setEnabled: (enabled: boolean) => void;
  setAudio: (audio: boolean) => void;
  setNotifications: (notifications: boolean) => void;
  edit: () => void;
}

/** The last menu shown and its items: native resources, freed when the next one opens. */
let shown: { close: () => Promise<void> }[] = [];

/**
 * A service's right-click menu, dropped down from `anchor` (its item in the
 * app bar) rather than at the pointer. Native rather than HTML: it has to
 * float over the service webview, which the bar-sized shell can't draw over.
 * `position` is the bar's, to place `anchor` in the window.
 */
export async function showServiceMenu(
  s: Service,
  anchor: DOMRect,
  actions: ServiceMenuActions,
  position: BarPosition,
) {
  for (const r of shown) void r.close();
  // Only a loaded page can be sent home or reloaded.
  const loaded = s.state === "active" || s.state === "suspended";
  const items = await Promise.all([
    MenuItem.new({ text: "Go to home", enabled: loaded, action: () => actions.home() }),
    MenuItem.new({ text: "Reload", enabled: loaded, action: () => actions.reload() }),
    PredefinedMenuItem.new({ item: "Separator" }),
    CheckMenuItem.new({ text: "Enable app", checked: s.enabled, action: () => actions.setEnabled(!s.enabled) }),
    CheckMenuItem.new({ text: "Enable sound", checked: s.audio, action: () => actions.setAudio(!s.audio) }),
    CheckMenuItem.new({
      text: "Enable notifications",
      checked: s.notifications,
      action: () => actions.setNotifications(!s.notifications),
    }),
    PredefinedMenuItem.new({ item: "Separator" }),
    MenuItem.new({ text: "Edit app", action: () => actions.edit() }),
  ]);
  const menu = await Menu.new({ items });
  shown = [menu, ...items];
  // The shell's coordinates are the window's, except that a right or bottom
  // bar is offset by the rest of the window.
  let [dx, dy] = [0, 0];
  if (position === "right" || position === "bottom") {
    const win = getCurrentWindow();
    const size = (await win.innerSize()).toLogical(await win.scaleFactor());
    if (position === "right") dx = size.width - window.innerWidth;
    else dy = size.height - window.innerHeight;
  }
  await menu.popup(new LogicalPosition(anchor.left + dx, anchor.bottom + dy));
}
