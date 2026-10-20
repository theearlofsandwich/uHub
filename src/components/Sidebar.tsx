import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from "react";
import type { BarPosition, IconSize, Service } from "../services/types";

interface Props {
  services: Service[];
  /** The window edge the bar sits on; Rust places the webview to match. */
  position: BarPosition;
  /** Show names next to the icons; a left or right bar never does. */
  labels: boolean;
  iconSize: IconSize;
  /** Gap between items, in px. */
  spacing: number;
  /** The last failed command, shown as a warning in the bar. */
  error: { kind?: string; message: string } | null;
  onDismissError: () => void;
  onSelect: (id: string) => void;
  /** Right-click on a service; `anchor` is its item. */
  onMenu: (service: Service, anchor: DOMRect) => void;
  /** Save a new order, listing every service once; rejects if it wasn't saved. */
  onReorder: (ids: string[]) => Promise<void>;
  /** Open the "Add app" modal. */
  onAdd: () => void;
  /** Open the settings modal. */
  onSettings: () => void;
}

/** Icon edge in px. The bar's thickness follows from it: `bar_thickness` in src-tauri/src/platform/mod.rs. */
const iconPx: Record<IconSize, number> = {
  "extra-small": 16,
  small: 20,
  medium: 28,
  large: 36,
  "extra-large": 44,
};

/** How far, in px, a press must move before it drags instead of clicking. */
const DRAG_THRESHOLD = 5;

/** `services` in `order`, dropping ids that are gone and appending any it lacks. */
function arrange(services: Service[], order: string[] | null): Service[] {
  if (!order) return services;
  const byId = new Map(services.map((s) => [s.id, s]));
  const listed = order.flatMap((id) => byId.get(id) ?? []);
  return [...listed, ...services.filter((s) => !order.includes(s.id))];
}

/** The app bar: what the shell webview shows while no modal covers the window. */
export function Sidebar({
  services,
  position,
  labels,
  iconSize,
  spacing,
  error,
  onDismissError,
  onSelect,
  onMenu,
  onReorder,
  onAdd,
  onSettings,
}: Props) {
  const horizontal = position === "top" || position === "bottom";
  const showLabels = horizontal && labels;
  const style = { "--icon-size": `${iconPx[iconSize]}px`, "--icon-gap": `${spacing}px` } as CSSProperties;

  const list = useRef<HTMLUListElement>(null);
  /** The service being dragged. */
  const [dragging, setDragging] = useState<string | null>(null);
  /** The order shown while dragging, and after a drop until Rust's snapshot has it. */
  const [order, setOrder] = useState<string[] | null>(null);
  /** A drag ends with a click on the button it started on, which mustn't switch to it. */
  const suppressClick = useRef(false);

  useEffect(() => {
    if (!dragging) setOrder(null);
    // Only a new snapshot replaces the order a drop left behind.
  }, [services]);

  // Window listeners rather than pointer capture: reordering moves the
  // button's DOM node, which would drop the capture.
  const press = (e: ReactPointerEvent, id: string) => {
    if (e.button !== 0) return;
    suppressClick.current = false;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let ids: string[] | null = null;

    const move = (ev: PointerEvent) => {
      if (!ids) {
        if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < DRAG_THRESHOLD) return;
        setDragging(id);
      }
      // The dragged item goes before the first other item whose middle is past the pointer.
      const others = [...(list.current?.querySelectorAll<HTMLElement>("li[data-id]") ?? [])].filter(
        (li) => li.dataset.id !== id,
      );
      const at = horizontal ? ev.clientX : ev.clientY;
      const index = others.filter((li) => {
        const r = li.getBoundingClientRect();
        return (horizontal ? r.left + r.width / 2 : r.top + r.height / 2) < at;
      }).length;
      const next = others.map((li) => li.dataset.id!);
      next.splice(index, 0, id);
      if (next.join() !== ids?.join()) setOrder(next);
      ids = next;
    };
    const end = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      if (!ids) return;
      setDragging(null);
      suppressClick.current = true;
      if (ev.type === "pointercancel" || ids.join() === services.map((s) => s.id).join()) setOrder(null);
      else onReorder(ids).catch(() => setOrder(null));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };

  return (
    <aside
      className={`sidebar ${position} ${horizontal ? "horizontal" : "vertical"}${showLabels ? " labels" : ""}`}
      style={style}
    >
      {showLabels && <h1>uHub</h1>}
      <ul className={"services" + (dragging ? " dragging" : "")} ref={list}>
        {arrange(services, order).map((s) => (
          <li key={s.id} data-id={s.id} className={s.id === dragging ? "dragged" : undefined}>
            <button
              className={"service" + (s.state === "active" ? " selected" : "")}
              // Not `disabled`: that would swallow the right-click that re-enables it.
              aria-disabled={!s.enabled}
              aria-label={showLabels ? undefined : s.name}
              // Primary button only: a right-click opens the menu, it doesn't switch.
              onClick={(e) => e.button === 0 && !suppressClick.current && s.enabled && onSelect(s.id)}
              onPointerDown={(e) => press(e, s.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                const anchor = e.currentTarget.getBoundingClientRect();
                // GTK fires this on press, and a menu that opens under the pointer
                // takes the release as a pick: wait for the button to come up.
                // (A keyboard-opened menu has no button held.)
                if (e.buttons & 2) window.addEventListener("mouseup", () => onMenu(s, anchor), { once: true });
                else onMenu(s, anchor);
              }}
              title={
                !s.enabled
                  ? `${s.name} is disabled (right-click to enable)`
                  : s.state === "crashed"
                    ? `${s.name} crashed: click to reload`
                    : `${s.name}\n${s.url}\nprofile: ${s.profile}`
              }
            >
              <span className={s.icon ? "service-icon has-image" : "service-icon"}>
                {s.icon ? <img src={s.icon} alt="" draggable={false} /> : <span className="icon-letter">{s.name[0]?.toUpperCase()}</span>}
                <span className={"dot " + s.state} />
              </span>
              {showLabels && <span className="name">{s.name}</span>}
              {s.mediaIndicator && s.playingAudio && (
                <span className="playing" role="img" aria-label="Playing audio" title="Playing audio">
                  <SpeakerIcon />
                </span>
              )}
              {s.unreadBadge && s.unread > 0 && (
                <span className="badge" aria-label={`${s.unread} unread`}>
                  {s.unread > 99 ? "99+" : s.unread}
                </span>
              )}
            </button>
          </li>
        ))}
        <li>
          <button className="service add-app" onClick={onAdd} title="Add app" aria-label={showLabels ? undefined : "Add app"}>
            <span className="service-icon">+</span>
            {showLabels && <span className="name">Add app</span>}
          </button>
        </li>
      </ul>

      <div className="bottom">
        {error && (
          <button
            className="service error"
            onClick={onDismissError}
            title={`${import.meta.env.DEV && error.kind ? `[${error.kind}] ` : ""}${error.message}\n(click to dismiss)`}
            aria-label={`Error: ${error.message}`}
          >
            <span className="service-icon">!</span>
          </button>
        )}
        <button className="service" onClick={onSettings} title="Settings" aria-label={showLabels ? undefined : "Settings"}>
          <span className="service-icon plain">
            <GearIcon />
          </span>
          {showLabels && <span className="name">Settings</span>}
        </button>
      </div>
    </aside>
  );
}

/** A speaker with sound waves, sized by the surrounding font. */
function SpeakerIcon() {
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" fill="currentColor" aria-hidden="true">
      <path d="M2 6h2.5L8 3v10L4.5 10H2z" />
      <path d="M10.5 5.5a3.5 3.5 0 0 1 0 5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <path d="M12.3 3.7a6 6 0 0 1 0 8.6" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

/** A cog, sized by the surrounding font. */
function GearIcon() {
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
      <circle cx="8" cy="8" r="2.2" />
      <path
        strokeLinejoin="round"
        d="M6.9 1.5h2.2l.4 1.8 1.2.7 1.8-.6 1.1 1.9-1.4 1.2v1.4l1.4 1.2-1.1 1.9-1.8-.6-1.2.7-.4 1.8H6.9l-.4-1.8-1.2-.7-1.8.6-1.1-1.9 1.4-1.2V7.3L2.4 6.1l1.1-1.9 1.8.6 1.2-.7z"
      />
    </svg>
  );
}
