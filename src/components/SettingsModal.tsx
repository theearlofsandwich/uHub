import { useEffect, useId, useState, type SyntheticEvent } from "react";
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

/** App-wide settings. Rendered while the shell covers the window, like `AppModal`. */
export function SettingsModal({ settings, onPreview, onChange, onClose }: Props) {
  const titleId = useId();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const set =
    <K extends keyof Settings>(key: K) =>
    (value: Settings[K]) => {
      if (!settings) return;
      onChange({ ...settings, [key]: value }).then(
        () => setError(null),
        (e) => setError(String((e as { message?: unknown })?.message ?? e)),
      );
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
