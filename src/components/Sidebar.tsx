import { useState } from "react";
import type { Recipe, Service, Settings } from "../services/types";

interface Props {
  services: Service[];
  recipes: Recipe[];
  /** null until loaded. */
  settings: Settings | null;
  onSelect: (id: string) => void;
  onUnload: (id: string) => void;
  onAdd: (recipeId: string, name: string) => void;
  onRemove: (id: string, deleteProfile: boolean) => void;
  onKeepAlive: (id: string, keepAlive: boolean) => void;
  onSettings: (settings: Settings) => void;
  onDevtools: () => void;
}

export function Sidebar({
  services,
  recipes,
  settings: appSettings,
  onSelect,
  onUnload,
  onAdd,
  onRemove,
  onKeepAlive,
  onSettings,
  onDevtools,
}: Props) {
  const [settings, setSettings] = useState(false);
  const active = services.find((s) => s.state === "active");

  return (
    <aside className="sidebar">
      <h1>WebDeck</h1>
      <ul className="services">
        {services
          .filter((s) => s.enabled)
          .map((s) => (
            <li key={s.id}>
              <button
                className={"service" + (s.state === "active" ? " selected" : "")}
                onClick={() => onSelect(s.id)}
                title={s.state === "crashed" ? "Crashed: click to reload" : `${s.url}\nprofile: ${s.profile}`}
              >
                <span className={"dot " + s.state} />
                <span className="name">{s.name}</span>
                {s.unread > 0 && (
                  <span className="badge" aria-label={`${s.unread} unread`}>
                    {s.unread > 99 ? "99+" : s.unread}
                  </span>
                )}
              </button>
            </li>
          ))}
      </ul>

      <div className="bottom">
        {settings && (
          <div className="settings">
            <AddService recipes={recipes} onAdd={onAdd} />
            <RemoveService services={services} onRemove={onRemove} />
            {appSettings && (
              <Background services={services} settings={appSettings} onKeepAlive={onKeepAlive} onSettings={onSettings} />
            )}
            <button disabled={!active} onClick={() => active && onUnload(active.id)}>
              Unload active service
            </button>
            {import.meta.env.DEV && (
              <button disabled={!active} onClick={onDevtools}>
                Open service DevTools
              </button>
            )}
            <small>
              Switching keeps services loaded in the background until they hibernate. Unload destroys the webview; its profile stays on disk so you stay logged in.
            </small>
          </div>
        )}
        <button className="service" onClick={() => setSettings((v) => !v)}>
          Settings
        </button>
      </div>
    </aside>
  );
}

function AddService({ recipes, onAdd }: { recipes: Recipe[]; onAdd: Props["onAdd"] }) {
  const [recipeId, setRecipeId] = useState("");
  const [name, setName] = useState("");
  const recipe = recipes.find((r) => r.id === recipeId) ?? recipes[0];
  if (!recipe) return null;

  return (
    <form
      className="field"
      onSubmit={(e) => {
        e.preventDefault();
        onAdd(recipe.id, name.trim() || recipe.name);
        setName("");
      }}
    >
      <label>Add service</label>
      <select value={recipe.id} onChange={(e) => setRecipeId(e.target.value)}>
        {recipes.map((r) => (
          <option key={r.id} value={r.id}>
            {r.name}
          </option>
        ))}
      </select>
      <input value={name} placeholder={recipe.name} maxLength={64} onChange={(e) => setName(e.target.value)} />
      <button type="submit">Add</button>
    </form>
  );
}

/** Hibernation, kept-alive services, and close to tray. */
function Background({
  services,
  settings,
  onKeepAlive,
  onSettings,
}: {
  services: Service[];
  settings: Settings;
  onKeepAlive: Props["onKeepAlive"];
  onSettings: Props["onSettings"];
}) {
  return (
    <div className="field">
      <label>Background</label>
      <NumberSetting
        label="Unload idle services after"
        unit="min (0: never)"
        value={settings.hibernate_after_minutes}
        max={7 * 24 * 60}
        onChange={(n) => onSettings({ ...settings, hibernate_after_minutes: n ?? 0 })}
      />
      <NumberSetting
        label="Keep at most"
        unit="in background (blank: any)"
        value={settings.max_suspended}
        max={100}
        onChange={(n) => onSettings({ ...settings, max_suspended: n })}
      />
      <label>Always keep loaded (notifications)</label>
      {services.map((s) => (
        <label key={s.id} className="check">
          <input type="checkbox" checked={s.keepAlive} onChange={(e) => onKeepAlive(s.id, e.target.checked)} />
          {s.name}
        </label>
      ))}
      <label className="check">
        <input
          type="checkbox"
          checked={settings.close_to_tray}
          onChange={(e) => onSettings({ ...settings, close_to_tray: e.target.checked })}
        />
        Closing the window hides it to the tray
      </label>
    </div>
  );
}

/** A whole number committed on blur or Enter, not per keystroke; blank is null. */
function NumberSetting({
  label,
  unit,
  value,
  max,
  onChange,
}: {
  label: string;
  unit: string;
  value: number | null;
  max: number;
  onChange: (n: number | null) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft === null) return;
    const n = draft.trim() === "" ? null : Number(draft);
    setDraft(null);
    if (n === value || (n !== null && !(Number.isInteger(n) && n >= 0 && n <= max))) return;
    onChange(n);
  };

  return (
    <label className="inline">
      {label}
      <input
        type="number"
        min={0}
        max={max}
        value={draft ?? value ?? ""}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
      {unit}
    </label>
  );
}

/** Pick a service, then confirm; deleting its profile (logins, storage) is opt-in. */
function RemoveService({ services, onRemove }: { services: Service[]; onRemove: Props["onRemove"] }) {
  const [id, setId] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [deleteProfile, setDeleteProfile] = useState(false);
  const svc = services.find((s) => s.id === id) ?? services[0];
  if (!svc) return null;

  const reset = () => {
    setConfirming(false);
    setDeleteProfile(false);
  };

  return (
    <div className="field">
      <label>Remove service</label>
      <select
        value={svc.id}
        disabled={confirming}
        onChange={(e) => setId(e.target.value)}
      >
        {services.map((s) => (
          <option key={s.id} value={s.id}>
            {s.name}
          </option>
        ))}
      </select>
      {confirming ? (
        <>
          <label className="check">
            <input type="checkbox" checked={deleteProfile} onChange={(e) => setDeleteProfile(e.target.checked)} />
            Also delete its login data (profile "{svc.profile}"; kept if another service shares it)
          </label>
          <div className="row">
            <button
              className="danger"
              onClick={() => {
                onRemove(svc.id, deleteProfile);
                reset();
              }}
            >
              Remove {svc.name}
            </button>
            <button onClick={reset}>Cancel</button>
          </div>
        </>
      ) : (
        <button onClick={() => setConfirming(true)}>Remove…</button>
      )}
    </div>
  );
}
