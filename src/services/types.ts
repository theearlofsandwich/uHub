export type Lifecycle = "active" | "suspended" | "unloaded" | "crashed";

/** Mirrors `ServiceInfo` in src-tauri/src/services.rs (Rust is the source of truth). */
export interface Service {
  /** Instance id: the only thing the shell ever sends back to Rust. */
  id: string;
  name: string;
  /** Recipe id, see `Recipe`. */
  recipe: string;
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
  /** false: the webview is muted. */
  audio: boolean;
  /** Show `playingAudio` in the sidebar. */
  mediaIndicator: boolean;
  /** Show `unread` in the sidebar (and count it in the tray). */
  unreadBadge: boolean;
  /** The page is playing audio right now (only reported on Linux). */
  playingAudio: boolean;
}

/** Mirrors `NewService` in src-tauri/src/services.rs: what "Add app" creates. */
export interface NewService {
  name: string;
  /** http(s); the recipe (WhatsApp, Gmail...) is picked from it. */
  url: string;
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
  /** Closing the window hides it to the tray instead of quitting. */
  close_to_tray: boolean;
}

/** Mirrors `RecipeInfo` in src-tauri/src/services.rs: a site services can be created from. */
export interface Recipe {
  id: string;
  name: string;
  url: string;
  allowedDomains: string[];
  icon: string | null;
}

/** Mirrors `Error::kind` in src-tauri/src/error.rs. */
export type ErrorKind =
  | "unknownService"
  | "serviceDisabled"
  | "webviewMissing"
  | "createFailed"
  | "invalidConfig"
  | "fetch"
  | "io"
  | "tauri";

/** What a failed command rejects with (see src-tauri/src/error.rs). */
export interface CommandError {
  kind: ErrorKind;
  message: string;
}
