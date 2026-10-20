import { useCallback, useEffect, useRef, useState } from "react";
import { AppModal } from "./components/AppModal";
import { SettingsModal } from "./components/SettingsModal";
import { showServiceMenu } from "./components/serviceMenu";
import { Sidebar } from "./components/Sidebar";
import {
  activateService,
  addService,
  getSettings,
  goHome,
  listServices,
  onServicesChanged,
  reloadService,
  removeService,
  reorderServices,
  setAudio,
  setEnabled,
  setNotifications,
  setOverlay,
  setSettings as saveSettings,
  updateService,
} from "./services/api";
import type { CommandError, NewService, Service, Settings } from "./services/types";
import "./App.css";

/** Command rejections are `CommandError`s; anything else (e.g. a failed `listen`) has no kind. */
function toError(e: unknown): { kind?: string; message: string } {
  if (typeof e === "object" && e !== null && "message" in e) {
    const { kind, message } = e as Partial<CommandError>;
    return { kind, message: String(message) };
  }
  return { message: String(e) };
}

/** Black or white text, whichever reads better on `color` (`#rrggbb`); the same as `text_on` in src-tauri/src/platform/mod.rs. */
function onColor(color: string): string {
  const channel = (i: number) => parseInt(color.slice(i, i + 2), 16) / 255;
  const luma = 0.299 * channel(1) + 0.587 * channel(3) + 0.114 * channel(5);
  return luma > 0.6 ? "#16181d" : "#ffffff";
}

export default function App() {
  const [services, setServices] = useState<Service[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  /** What Rust last saved; `settings` may be ahead of it while a slider is dragged. */
  const savedSettings = useRef<Settings | null>(null);
  const [error, setError] = useState<{ kind?: string; message: string } | null>(null);
  /** The open modal: "new" adds an app, a service edits it. */
  const [modal, setModal] = useState<Service | "new" | "settings" | null>(null);

  // State comes from "services-changed" events; invoke results only carry errors.
  const run = useCallback(async (op: Promise<void>) => {
    try {
      await op;
      setError(null);
    } catch (e) {
      setError(toError(e));
    }
  }, []);

  useEffect(() => {
    // Subscribe before the initial load; once an event has arrived, the
    // list_services reply may be older than it, so it is dropped.
    let gotEvent = false;
    const unlisten = onServicesChanged((s) => {
      gotEvent = true;
      setServices(s);
    });
    unlisten
      .then(() => listServices())
      .then((s) => {
        if (!gotEvent) setServices(s);
      })
      .catch((e) => setError(toError(e)));
    return () => void unlisten.then((f) => f());
  }, []);

  useEffect(() => {
    getSettings()
      .then((s) => {
        savedSettings.current = s;
        setSettings(s);
      })
      .catch((e) => setError(toError(e)));
  }, []);

  // The accent follows the theme colour, previews included (Rust paints the title bar).
  const themeColor = settings?.theme_color;
  useEffect(() => {
    if (!themeColor) return;
    const root = document.documentElement.style;
    root.setProperty("--accent", themeColor);
    root.setProperty("--on-accent", onColor(themeColor));
  }, [themeColor]);

  // Shown before Rust has saved it (and moved the bar's webview), so the
  // change is immediate; the modal covers the window meanwhile. A failed save
  // goes back to what was saved, dropping any unsaved preview too.
  const changeSettings = async (next: Settings) => {
    setSettings(next);
    try {
      await saveSettings(next);
      savedSettings.current = next;
    } catch (e) {
      setSettings(savedSettings.current);
      throw e;
    }
  };

  // The modal is only rendered once the shell covers the window, and the
  // shell shrinks back only after it is gone, so it never shows squeezed
  // into the bar.
  const openModal = (m: Service | "new" | "settings") => void run(setOverlay(true).then(() => setModal(m)));
  const closeModal = useCallback(() => {
    setModal(null);
    void run(setOverlay(false));
  }, [run]);
  const submitAdd = async (service: NewService) => {
    const id = await addService(service);
    closeModal();
    if (service.enabled) await run(activateService(id));
  };
  const submitEdit = (id: string) => async (service: NewService) => {
    await updateService(id, service);
    closeModal();
  };
  const submitRemove = (id: string) => async (deleteProfile: boolean) => {
    await removeService(id, deleteProfile);
    closeModal();
  };

  const position = settings?.bar_position ?? "left";
  const openMenu = (s: Service, anchor: DOMRect) =>
    void showServiceMenu(
      s,
      anchor,
      {
        home: () => void run(goHome(s.id)),
        reload: () => void run(reloadService(s.id)),
        setEnabled: (on) => void run(setEnabled(s.id, on)),
        setAudio: (on) => void run(setAudio(s.id, on)),
        setNotifications: (on) => void run(setNotifications(s.id, on)),
        edit: () => openModal(s),
      },
      position,
    ).catch((e) => setError(toError(e)));

  return (
    <>
      {/* Not before the settings: Rust may have already put the bar on another edge. */}
      {(settings || error) && (
        <Sidebar
          services={services}
          position={position}
          labels={settings?.bar_labels ?? false}
          iconSize={settings?.icon_size ?? "medium"}
          spacing={settings?.icon_spacing ?? 2}
          error={error}
          onDismissError={() => setError(null)}
          onSelect={(id) => void run(activateService(id))}
          onMenu={openMenu}
          onReorder={(ids) =>
            reorderServices(ids).then(
              () => setError(null),
              (e) => {
                setError(toError(e));
                throw e;
              },
            )
          }
          onAdd={() => openModal("new")}
          onSettings={() => openModal("settings")}
        />
      )}
      {modal === "new" && <AppModal onSubmit={submitAdd} onClose={closeModal} />}
      {modal === "settings" && <SettingsModal settings={settings} onPreview={setSettings} onChange={changeSettings} onClose={closeModal} />}
      {typeof modal === "object" && modal !== null && (
        <AppModal
          key={modal.id}
          service={modal}
          onSubmit={submitEdit(modal.id)}
          onRemove={submitRemove(modal.id)}
          onClose={closeModal}
        />
      )}
    </>
  );
}
