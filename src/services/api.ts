import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { NewService, Recipe, Service, SiteInfo, Settings } from "./types";

/** Initial snapshot only; later changes arrive via `onServicesChanged`. */
export const listServices = () => invoke<Service[]>("list_services");
export const listRecipes = () => invoke<Recipe[]>("list_recipes");
export const activateService = (id: string) => invoke<void>("activate_service", { id });
export const unloadService = (id: string) => invoke<void>("unload_service", { id });
/** Resolves to the new service's id. */
export const addService = (service: NewService) => invoke<string>("add_service", { service });
/** Fetches the page at `url` (from Rust: the shell can't) for a default name and icon. */
export const inspectSite = (url: string) => invoke<SiteInfo>("inspect_site", { url });
/** Let the shell cover the whole window, above the service, while it shows a modal. */
export const setOverlay = (open: boolean) => invoke<void>("set_overlay", { open });
export const removeService = (id: string, deleteProfile: boolean) =>
  invoke<void>("remove_service", { id, deleteProfile });
export const renameService = (id: string, name: string) => invoke<void>("rename_service", { id, name });
/** `ids` must list every service exactly once. */
export const reorderServices = (ids: string[]) => invoke<void>("reorder_services", { ids });
export const setKeepAlive = (id: string, keepAlive: boolean) => invoke<void>("set_keep_alive", { id, keepAlive });
export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<void>("set_settings", { settings });
export const openDevtools = () => invoke<void>("open_devtools");

/** Full snapshot pushed by Rust on every state change (see webviews.rs). Resolves to an unlisten fn. */
export const onServicesChanged = (cb: (services: Service[]) => void) =>
  listen<Service[]>("services-changed", (e) => cb(e.payload));
