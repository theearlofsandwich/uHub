export type Lifecycle = "active" | "suspended" | "unloaded" | "crashed";

/** Mirrors `ServiceInfo` in src-tauri/src/services.rs (Rust is the source of truth). */
export interface Service {
  /** Instance id: the only thing the shell ever sends back to Rust. */
  id: string;
  name: string;
  /** Recipe id, see `Recipe`. */
  recipe: string;
  /** For a recipe with a `team` field. */
  team: string | null;
  url: string;
  allowedDomains: string[];
  /** Storage profile; distinct profiles => isolated cookies/storage. */
  profile: string;
  enabled: boolean;
  /** A data: URL. */
  icon: string | null;
  state: Lifecycle;
  /** Unread count from the page title; 0 while unloaded. */
  unread: number;
  /** Never hibernated: stays loaded in the background (e.g. for notifications). */
  keepAlive: boolean;
  /** null: the app-wide `Settings.hibernate_after_minutes`. */
  hibernateAfterMinutes: number | null;
  /** The page may show (desktop) notifications. */
  notifications: boolean;
  /** false: the webview is muted. */
  audio: boolean;
  /** Show `playingAudio` in the sidebar. */
  mediaIndicator: boolean;
  /** Show `unread` in the sidebar (and count it in the tray). */
  unreadBadge: boolean;
  /** Off-site links open in a uHub window instead of the browser. */
  openLinksInside: boolean;
  /** Stays unloaded when uHub starts; otherwise (if enabled) it loads in the background. */
  startHibernated: boolean;
  /** The page is playing audio right now (only reported on Linux). */
  playingAudio: boolean;
}

/** Mirrors `NewService` in src-tauri/src/services.rs: what "Add app" creates and "Edit app" saves. */
export interface NewService {
  name: string;
  /** Recipe id. */
  recipe: string;
  /** Required by a recipe with a `team` field, else null. */
  team: string | null;
  /** http(s); null for the recipe's own. Required by a self-hosted recipe. */
  url: string | null;
  /** A data: URL; null for none. */
  icon: string | null;
  notifications: boolean;
  audio: boolean;
  mediaIndicator: boolean;
  unreadBadge: boolean;
  enabled: boolean;
  hibernation: boolean;
  hibernateAfterMinutes: number;
  openLinksInside: boolean;
  startHibernated: boolean;
}

/** Mirrors `SiteInfo` in src-tauri/src/favicon.rs. */
export interface SiteInfo {
  /** A short name from the page title. */
  name: string | null;
  /** The site's icon as a data: URL. */
  icon: string | null;
}

/** Mirrors `Settings` in src-tauri/src/services.rs; snake_case as stored in services.json. */
export interface Settings {
  /** Unload background services (unless kept alive) after this many minutes; 0 = never. */
  hibernate_after_minutes: number;
  /** At most this many background services (not counting kept-alive ones); null = no limit. */
  max_suspended: number | null;
  /** Closing the window hides it to the tray instead of quitting (if there is a tray). */
  close_to_tray: boolean;
  /** The window edge the app bar sits on. */
  bar_position: BarPosition;
  /** App names next to their icons; only used by a top or bottom bar. */
  bar_labels: boolean;
  icon_size: IconSize;
  /** Gap between the bar's icons in px, 0 to 16 (MAX_ICON_SPACING in services.rs). */
  icon_spacing: number;
  /** `#rrggbb`: the window's title bar and the shell's accents. */
  theme_color: string;
  /** Off: no service shows desktop notifications, whatever its own setting. */
  notifications: boolean;
  /** Off: every service is muted, whatever its own setting. */
  audio: boolean;
}

export type BarPosition = "left" | "top" | "bottom" | "right";
export type IconSize = "extra-small" | "small" | "medium" | "large" | "extra-large";

/** Mirrors `RecipeInfo` in src-tauri/src/services.rs: a site services can be created from. */
export interface Recipe {
  id: string;
  name: string;
  /** With `team`, "{team}" stands for the team; empty if self-hosted. */
  url: string;
  allowedDomains: string[];
  /** A data: URL. */
  icon: string | null;
  team: TeamField | null;
  hosting: Hosting;
  /** A note for the add/edit form. */
  message: string | null;
}

/** Mirrors `TeamField`: the team input, shown as `prefix` [input] `suffix`. */
export interface TeamField {
  label: string;
  prefix: string;
  suffix: string;
}

/**
 * Mirrors `Hosting`. "cloud": only at the recipe's URL (or elsewhere on its
 * domains); "both": that or the user's own server; "selfHosted": only the user's own.
 */
export type Hosting = "cloud" | "both" | "selfHosted";

/** Recipe id of an app added by its address: any site. */
export const CUSTOM = "custom";

/** Mirrors `Error::kind` in src-tauri/src/error.rs. */
export type ErrorKind =
  | "unknownService"
  | "serviceDisabled"
  | "webviewMissing"
  | "createFailed"
  | "invalidConfig"
  | "fetch"
  | "autostart"
  | "io"
  | "tauri";

/** What a failed command rejects with (see src-tauri/src/error.rs). */
export interface CommandError {
  kind: ErrorKind;
  message: string;
}
