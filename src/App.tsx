import { useCallback, useEffect, useRef, useState } from "react";
import { AppModal } from "./components/AppModal";
import { Hibernated, isHibernated } from "./components/Hibernated";
import { PerformanceModal } from "./components/PerformanceModal";
import { RecipePicker } from "./components/RecipePicker";
import { SettingsModal } from "./components/SettingsModal";
import { showNotificationMenu, showServiceMenu } from "./components/serviceMenu";
import { Sidebar } from "./components/Sidebar";
import {
  activateService,
  addService,
  getSettings,
  goHome,
  listRecipes,
  listServices,
  onServicesChanged,
  onShortcut,
  reloadService,
  removeService,
  reorderServices,
  setAudio,
  setEnabled,
  setHibernated,
  setNotifications,
  setOverlay,
  setSettings as saveSettings,
  updateService,
  wakeService,
} from "./services/api";
import type { CommandError, NewService, Recipe, Service, Settings } from "./services/types";
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

type Modal = Service | "new" | { adding: Recipe } | "settings" | "performance";

export default function App() {
  const [services, setServices] = useState<Service[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  /** What Rust last saved; `settings` may be ahead of it while a slider is dragged. */
  const savedSettings = useRef<Settings | null>(null);
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [error, setError] = useState<{ kind?: string; message: string } | null>(null);
  /** The open modal: "new" picks an app to add, `adding` adds it, a service edits it. */
  const [modal, setModal] = useState<Modal | null>(null);

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
    listRecipes()
      .then(setRecipes)
      .catch((e) => setError(toError(e)));
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
  const openModal = useCallback(
    (m: Modal) => void run(setOverlay(true).then(() => setModal(m))),
    [run],
  );
  // Rust pauses shortcuts while a modal is open, so this never replaces one.
  useEffect(() => {
    const unlisten = onShortcut((name) => name === "settings" && openModal("settings"));
    return () => void unlisten.then((f) => f());
  }, [openModal]);
  const closeModal = useCallback(() => {
    setModal(null);
    void run(setOverlay(false));
  }, [run]);
  const submitAdd = async (service: NewService) => {
    const id = await addService(service);
    closeModal();
    if (service.enabled) await run(wakeService(id));
  };
  const submitEdit = (id: string) => async (service: NewService) => {
    await updateService(id, service);
    closeModal();
  };
  const submitRemove = (id: string) => async (deleteProfile: boolean) => {
    await removeService(id, deleteProfile);
    closeModal();
  };

  // Recipes load at startup; until then an app can't be edited.
  const editedRecipe = typeof modal === "object" && modal !== null && "id" in modal ? recipes.find((r) => r.id === modal.recipe) : undefined;
  const position = settings?.bar_position ?? "left";
  const iconSize = settings?.icon_size ?? "medium";
  const hibernated = services.find((s) => s.shown && isHibernated(s));
  // The switches show Rust's current state, not the last snapshot: one sent
  // by a change still in progress (hibernating, say) may not have arrived
  // yet. list_services waits for that change to finish.
  const openMenu = (shown: Service, anchor: DOMRect) =>
    void listServices()
      .then((list) => {
        const s = list.find((x) => x.id === shown.id);
        if (!s) return;
        return showServiceMenu(
          s,
          anchor,
          {
            home: () => void run(goHome(s.id)),
            reload: () => void run(reloadService(s.id)),
            setEnabled: (on) => void run(setEnabled(s.id, on)),
            setAudio: (on) => void run(setAudio(s.id, on)),
            setNotifications: (on) => void run(setNotifications(s.id, on)),
            setHibernated: (on) => void run(setHibernated(s.id, on)),
            edit: () => openModal(s),
          },
          position,
        );
      })
      .catch((e) => setError(toError(e)));

  // The bell's switches save like any setting; a failure shows in the bar.
  const openNotificationMenu = (anchor: DOMRect) => {
    if (!settings) return;
    const toggle = (change: Partial<Settings>) => void run(changeSettings({ ...settings, ...change }));
    void showNotificationMenu(
      settings,
      anchor,
      { setNotifications: (notifications) => toggle({ notifications }), setAudio: (audio) => toggle({ audio }) },
      position,
    ).catch((e) => setError(toError(e)));
  };

  return (
    <>
      {/* Not before the settings: Rust may have already put the bar on another edge. */}
      {(settings || error) && (
        <Sidebar
          services={services}
          position={position}
          labels={settings?.bar_labels ?? false}
          iconSize={iconSize}
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
          notifications={settings?.notifications ?? true}
          audio={settings?.audio ?? true}
          onNotificationMenu={openNotificationMenu}
          onPerformance={() => openModal("performance")}
          onSettings={() => openModal("settings")}
        />
      )}
      {hibernated && (
        <Hibernated service={hibernated} position={position} iconSize={iconSize} onWake={() => void run(wakeService(hibernated.id))} />
      )}
      {modal === "new" && <RecipePicker recipes={recipes} onPick={(r) => setModal({ adding: r })} onClose={closeModal} />}
      {typeof modal === "object" && modal !== null && "adding" in modal && (
        <AppModal
          recipe={modal.adding}
          recipes={recipes}
          onSubmit={submitAdd}
          onBack={() => setModal("new")}
          onClose={closeModal}
        />
      )}
      {modal === "performance" && <PerformanceModal services={services} onClose={closeModal} />}
      {modal === "settings" && <SettingsModal settings={settings} onPreview={setSettings} onChange={changeSettings} onClose={closeModal} />}
      {editedRecipe && typeof modal === "object" && modal !== null && "id" in modal && (
        <AppModal
          key={modal.id}
          recipe={editedRecipe}
          recipes={recipes}
          service={modal}
          onSubmit={submitEdit(modal.id)}
          onRemove={submitRemove(modal.id)}
          onClose={closeModal}
        />
      )}
    </>
  );
}
