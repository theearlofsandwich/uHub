import type { CSSProperties } from "react";
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
  onAdd,
  onSettings,
}: Props) {
  const horizontal = position === "top" || position === "bottom";
  const showLabels = horizontal && labels;
  const style = { "--icon-size": `${iconPx[iconSize]}px`, "--icon-gap": `${spacing}px` } as CSSProperties;

  return (
    <aside
      className={`sidebar ${position} ${horizontal ? "horizontal" : "vertical"}${showLabels ? " labels" : ""}`}
      style={style}
    >
      {showLabels && <h1>uHub</h1>}
      <ul className="services">
        {services.map((s) => (
          <li key={s.id}>
            <button
              className={"service" + (s.state === "active" ? " selected" : "")}
              // Not `disabled`: that would swallow the right-click that re-enables it.
              aria-disabled={!s.enabled}
              aria-label={showLabels ? undefined : s.name}
              // Primary button only: a right-click opens the menu, it doesn't switch.
              onClick={(e) => e.button === 0 && s.enabled && onSelect(s.id)}
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
              <span className="service-icon">
                {s.icon ? <img src={s.icon} alt="" /> : <span className="icon-letter">{s.name[0]?.toUpperCase()}</span>}
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
