import { useEffect, useId, useRef, useState, type SyntheticEvent } from "react";
import { getAutostart, setAutostart } from "../services/api";
import type { BarPosition, IconSize, Settings } from "../services/types";
import { Section, Switch } from "./AppModal";

interface Props {
  /** null while still loading. */
  settings: Settings | null;
  /** Shows a change without saving it, while a slider is dragged. */
  onPreview: (settings: Settings) => void;
  /** Saves a change right away; rejects (with a `CommandError` or anything) to show it. */
  onChange: (settings: Settings) => Promise<void>;
  onClose: () => void;
}

const positions: { value: BarPosition; label: string }[] = [
  { value: "left", label: "Left" },
  { value: "top", label: "Top" },
  { value: "bottom", label: "Bottom" },
  { value: "right", label: "Right" },
];

const iconSizes: { value: IconSize; label: string }[] = [
  { value: "extra-small", label: "Extra small" },
  { value: "small", label: "Small" },
  { value: "medium", label: "Medium" },
  { value: "large", label: "Large" },
  { value: "extra-large", label: "Extra large" },
];

/** Theme colours offered as swatches; the colour input picks any other. Lowercase, like its value. */
const swatches: { value: string; label: string }[] = [
  { value: "#5b8cff", label: "Blue" },
  { value: "#8b5cf6", label: "Purple" },
  { value: "#ec4899", label: "Pink" },
  { value: "#ef4444", label: "Red" },
  { value: "#f97316", label: "Orange" },
  { value: "#eab308", label: "Yellow" },
  { value: "#22c55e", label: "Green" },
  { value: "#14b8a6", label: "Teal" },
  { value: "#64748b", label: "Slate" },
];

/** Mirrors the table in src-tauri/src/shortcuts.rs; each entry's alternatives do the same. */
const SHORTCUTS: { label: string; keys: string[] }[] = [
  { label: "Switch to app 1–8", keys: ["Ctrl+1…8"] },
  { label: "Switch to the last app", keys: ["Ctrl+9"] },
  { label: "Next app", keys: ["Ctrl+Tab", "Ctrl+PgDn"] },
  { label: "Previous app", keys: ["Ctrl+Shift+Tab", "Ctrl+PgUp"] },
  { label: "Reload", keys: ["Ctrl+R", "F5"] },
  { label: "Back / forward", keys: ["Alt+←", "Alt+→"] },
  { label: "Go to home", keys: ["Alt+Home"] },
  { label: "Settings", keys: ["Ctrl+,"] },
  { label: "Quit", keys: ["Ctrl+Q"] },
];

/** How long the colour input must rest, in ms, before its colour is saved. */
const COLOR_SAVE_DELAY = 250;

/** App-wide settings. Rendered while the shell covers the window, like `AppModal`. */
export function SettingsModal({ settings, onPreview, onChange, onClose }: Props) {
  const titleId = useId();
  const [error, setError] = useState<string | null>(null);
  /** Kept by the OS rather than in `settings`; null until read (or if it can't be). */
  const [autostart, setAutostartState] = useState<boolean | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const showError = (e: unknown) => setError(String((e as { message?: unknown })?.message ?? e));

  useEffect(() => {
    getAutostart().then(setAutostartState, showError);
  }, []);

  const toggleAutostart = (enabled: boolean) => {
    setAutostartState(enabled);
    setAutostart(enabled).then(
      () => setError(null),
      (e) => {
        setAutostartState(!enabled);
        showError(e);
      },
    );
  };

  const set =
    <K extends keyof Settings>(key: K) =>
    (value: Settings[K]) => {
      if (!settings) return;
      onChange({ ...settings, [key]: value }).then(() => setError(null), showError);
    };

  // A slider previews while it moves and saves once it's let go (pointer
  // released, or a key released after stepping it with the keyboard).
  const slide =
    <K extends keyof Settings>(key: K, parse: (value: string) => Settings[K]) =>
    ({
      onChange: (e: SyntheticEvent<HTMLInputElement>) =>
        settings && onPreview({ ...settings, [key]: parse(e.currentTarget.value) }),
      onPointerUp: (e: SyntheticEvent<HTMLInputElement>) => set(key)(parse(e.currentTarget.value)),
      onKeyUp: (e: SyntheticEvent<HTMLInputElement>) => set(key)(parse(e.currentTarget.value)),
    });

  // The colour input reports every colour it passes through: preview each,
  // save the one it rests on. Not cancelled on close, so that one is saved.
  const colorTimer = useRef<number | undefined>(undefined);
  const pickColor = (color: string) => {
    if (!settings) return;
    onPreview({ ...settings, theme_color: color });
    window.clearTimeout(colorTimer.current);
    colorTimer.current = window.setTimeout(() => set("theme_color")(color), COLOR_SAVE_DELAY);
  };

  const horizontal = settings?.bar_position === "top" || settings?.bar_position === "bottom";

  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <header className="modal-header">
          <h2 id={titleId}>Settings</h2>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}>
            ×
          </button>
        </header>

        <div className="modal-body">
          {settings ? (
            <>
              <Section title="General">
                <Switch
                  label="Close button closes to tray"
                  hint={settings.close_to_tray ? "uHub keeps running in the tray" : "Closing the window quits uHub"}
                  checked={settings.close_to_tray}
                  onChange={set("close_to_tray")}
                />
                {autostart !== null && (
                  <Switch label="Enable autostart on login" checked={autostart} onChange={toggleAutostart} />
                )}
              </Section>
              <Section title="App bar">
                <label className="row-field">
                  <span>
                    Position
                    <small>{horizontal ? "Along the window's edge" : "Icons only; hover an app for its name"}</small>
                  </span>
                  <select
                    value={settings.bar_position}
                    onChange={(e) => set("bar_position")(e.target.value as BarPosition)}
                  >
                    {positions.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </label>
                {horizontal && (
                  <Switch
                    label="Show labels"
                    hint="App names next to their icons"
                    checked={settings.bar_labels}
                    onChange={set("bar_labels")}
                  />
                )}
                <label className="row-field">
                  <span>Icon size</span>
                  <select value={settings.icon_size} onChange={(e) => set("icon_size")(e.target.value as IconSize)}>
                    {iconSizes.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="row-field">
                  <span>Spacing</span>
                  <span className="range-field">
                    <input
                      type="range"
                      min={0}
                      max={16}
                      step={1}
                      value={settings.icon_spacing}
                      {...slide("icon_spacing", Number)}
                    />
                    <output>{settings.icon_spacing}px</output>
                  </span>
                </label>
              </Section>
              <Section title="Theme">
                <div className="row-field">
                  <span>
                    Colour
                    <small>The title bar and highlights</small>
                  </span>
                  <span className="color-field">
                    {swatches.map((c) => (
                      <button
                        key={c.value}
                        type="button"
                        className="swatch"
                        style={{ background: c.value }}
                        title={c.label}
                        aria-label={c.label}
                        aria-pressed={settings.theme_color === c.value}
                        onClick={() => {
                          window.clearTimeout(colorTimer.current);
                          set("theme_color")(c.value);
                        }}
                      />
                    ))}
                    <input
                      type="color"
                      title="Custom colour"
                      aria-label="Custom colour"
                      value={settings.theme_color}
                      onChange={(e) => pickColor(e.target.value)}
                    />
                  </span>
                </div>
              </Section>
              <Section title="Keyboard shortcuts">
                {SHORTCUTS.map((s) => (
                  <div key={s.label} className="row-field">
                    <span>{s.label}</span>
                    <span className="shortcut-keys">
                      {s.keys.map((k) => (
                        <kbd key={k}>{k}</kbd>
                      ))}
                    </span>
                  </div>
                ))}
              </Section>
            </>
          ) : (
            <p className="modal-empty">Loading…</p>
          )}
        </div>

        <footer className="modal-footer">
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
          <button type="button" className="button primary" autoFocus onClick={onClose}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
