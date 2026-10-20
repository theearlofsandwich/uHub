import { useCallback, useEffect, useState } from "react";
import { AppModal } from "./components/AppModal";
import { showServiceMenu } from "./components/serviceMenu";
import { Sidebar } from "./components/Sidebar";
import {
  activateService,
  addService,
  getSettings,
  goHome,
  listServices,
  onServicesChanged,
  openDevtools,
  reloadService,
  removeService,
  setAudio,
  setEnabled,
  setKeepAlive,
  setNotifications,
  setOverlay,
  setSettings,
  unloadService,
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

export default function App() {
  const [services, setServices] = useState<Service[]>([]);
  const [settings, setSettingsState] = useState<Settings | null>(null);
  const [error, setError] = useState<{ kind?: string; message: string } | null>(null);
  /** The app modal: "new" to add one, a service to edit it. */
  const [modal, setModal] = useState<Service | "new" | null>(null);

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
    getSettings()
      .then(setSettingsState)
      .catch((e) => setError(toError(e)));
    return () => void unlisten.then((f) => f());
  }, []);

  // The modal is only rendered once the shell covers the window, and the
  // shell shrinks back only after it is gone, so it never shows squeezed
  // into the sidebar.
  const openModal = (m: Service | "new") => void run(setOverlay(true).then(() => setModal(m)));
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
  // Logins (the profile) are kept; the settings panel's "Remove service" can delete them.
  const submitRemove = (id: string) => async () => {
    await removeService(id, false);
    closeModal();
  };

  const openMenu = (s: Service, anchor: DOMRect) =>
    void showServiceMenu(s, anchor, {
      home: () => void run(goHome(s.id)),
      reload: () => void run(reloadService(s.id)),
      setEnabled: (on) => void run(setEnabled(s.id, on)),
      setAudio: (on) => void run(setAudio(s.id, on)),
      setNotifications: (on) => void run(setNotifications(s.id, on)),
      edit: () => openModal(s),
    }).catch((e) => setError(toError(e)));

  return (
    <>
      <Sidebar
        services={services}
        settings={settings}
        onSelect={(id) => void run(activateService(id))}
        onMenu={openMenu}
        onUnload={(id) => void run(unloadService(id))}
        onAdd={() => openModal("new")}
        onRemove={(id, deleteProfile) => void run(removeService(id, deleteProfile))}
        onKeepAlive={(id, keepAlive) => void run(setKeepAlive(id, keepAlive))}
        onSettings={(next) => void run(setSettings(next).then(() => setSettingsState(next)))}
        onDevtools={() => void openDevtools()}
      />
      {error && (
        <div className="error">
          {import.meta.env.DEV && error.kind && <code className="error-kind">{error.kind}</code>}
          {error.message}
        </div>
      )}
      {modal === "new" && <AppModal onSubmit={submitAdd} onClose={closeModal} />}
      {modal && modal !== "new" && (
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
