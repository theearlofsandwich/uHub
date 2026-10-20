import type { CSSProperties } from "react";
import type { BarPosition, IconSize, Service } from "../services/types";
import { iconPx } from "./Sidebar";

interface Props {
  service: Service;
  position: BarPosition;
  iconSize: IconSize;
  onWake: () => void;
}

/** Unloaded, or on screen as hibernated (auto-wake may have loaded it behind the page since). */
export function isHibernated(s: Service): boolean {
  return s.state === "unloaded" || (s.shown && s.state === "suspended");
}

/**
 * In place of a service hibernated while on screen: Rust has the shell cover
 * the window meanwhile (`sync_overlay` in src-tauri/src/webviews.rs), and
 * this fills the part beside the bar. Once the shell is the bar again it
 * lies outside it, so it never shows squeezed.
 */
export function Hibernated({ service, position, iconSize, onWake }: Props) {
  // As `--bar-size` in App.css.
  const style = { "--bar-size": `${iconPx[iconSize] + 2 * 8 + 2 * 6}px` } as CSSProperties;
  return (
    <main className={`hibernated ${position}`} style={style}>
      <span className={service.icon ? "service-icon has-image" : "service-icon"}>
        {service.icon ? <img src={service.icon} alt="" /> : <span className="icon-letter">{service.name[0]?.toUpperCase()}</span>}
      </span>
      <h2>{service.name} hibernated</h2>
      <p>It's unloaded to save memory, and sends no notifications until it wakes.</p>
      <button className="button primary" onClick={onWake} autoFocus>
        Wake up
      </button>
    </main>
  );
}
