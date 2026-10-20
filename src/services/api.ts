import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { NewService, Performance, Recipe, Service, SiteInfo, Settings } from "./types";

/** Initial snapshot only; later changes arrive via `onServicesChanged`. */
export const listServices = () => invoke<Service[]>("list_services");
export const listRecipes = () => invoke<Recipe[]>("list_recipes");
/** Switch to `id`; a hibernated one shows its "App hibernated" page instead of loading. */
export const activateService = (id: string) => invoke<void>("activate_service", { id });
/** Switch to `id`, loading it even if it is hibernated. */
export const wakeService = (id: string) => invoke<void>("wake_service", { id });
/** Hibernate `id` now, or wake it (in the background, unless its "App hibernated" page is on screen). */
export const setHibernated = (id: string, hibernated: boolean) => invoke<void>("set_hibernated", { id, hibernated });
/** Resolves to the new service's id. */
export const addService = (service: NewService) => invoke<string>("add_service", { service });
/** Fetches the page at `url` (from Rust: the shell can't) for a default name and icon. */
export const inspectSite = (url: string) => invoke<SiteInfo>("inspect_site", { url });
/** Let the shell cover the whole window, above the service, while it shows a modal. */
export const setOverlay = (open: boolean) => invoke<void>("set_overlay", { open });
/** Save the "Edit app" form; a loaded service may be recreated (see webviews.rs). */
export const updateService = (id: string, service: NewService) => invoke<void>("update_service", { id, service });
/** These two act on a loaded service where it is, without switching to it. */
export const goHome = (id: string) => invoke<void>("go_home", { id });
export const reloadService = (id: string) => invoke<void>("reload_service", { id });
/** Disabling unloads the service. */
export const setEnabled = (id: string, enabled: boolean) => invoke<void>("set_enabled", { id, enabled });
export const setAudio = (id: string, audio: boolean) => invoke<void>("set_audio", { id, audio });
export const setNotifications = (id: string, notifications: boolean) =>
  invoke<void>("set_notifications", { id, notifications });
export const removeService = (id: string, deleteProfile: boolean) =>
  invoke<void>("remove_service", { id, deleteProfile });
export const renameService = (id: string, name: string) => invoke<void>("rename_service", { id, name });
/** `ids` must list every service exactly once. */
export const reorderServices = (ids: string[]) => invoke<void>("reorder_services", { ids });
export const setKeepAlive = (id: string, keepAlive: boolean) => invoke<void>("set_keep_alive", { id, keepAlive });
export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<void>("set_settings", { settings });
/** Whether uHub starts at login; kept by the OS, not in `Settings`. */
export const getAutostart = () => invoke<boolean>("get_autostart");
export const setAutostart = (enabled: boolean) => invoke<void>("set_autostart", { enabled });
export const openDevtools = () => invoke<void>("open_devtools");
/** Memory and CPU use now, overall and by profile; CPU is measured since the previous call. */
export const getPerformance = () => invoke<Performance>("get_performance");

/** A keyboard shortcut for the shell to carry out (see shortcuts.rs). Resolves to an unlisten fn. */
export const onShortcut = (cb: (name: "settings") => void) => listen<"settings">("shortcut", (e) => cb(e.payload));

/** Full snapshot pushed by Rust on every state change (see webviews.rs). Resolves to an unlisten fn. */
export const onServicesChanged = (cb: (services: Service[]) => void) =>
  listen<Service[]>("services-changed", (e) => cb(e.payload));
