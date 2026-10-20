import { useEffect, useId, useState } from "react";
import { CUSTOM, type Recipe } from "../services/types";

interface Props {
  /** In display order; the custom website is shown last whatever its place. */
  recipes: Recipe[];
  onPick: (recipe: Recipe) => void;
  onClose: () => void;
}

/** The first step of "Add app": a searchable grid of the apps uHub knows, and "Custom website". */
export function RecipePicker({ recipes, onPick, onClose }: Props) {
  const [query, setQuery] = useState("");
  const titleId = useId();
  const custom = recipes.find((r) => r.id === CUSTOM);
  const q = query.trim().toLowerCase();
  const shown = recipes.filter((r) => r.id !== CUSTOM && (!q || r.name.toLowerCase().includes(q) || r.id.includes(q)));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop">
      <form
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={(e) => {
          e.preventDefault();
          // Enter picks the only (or first) match.
          const first = shown[0] ?? custom;
          if (first) onPick(first);
        }}
      >
        <header className="modal-header">
          <h2 id={titleId}>Add app</h2>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}>
            ×
          </button>
        </header>

        <div className="modal-body">
          <label className="text-field">
            <span>Search</span>
            <input autoFocus type="search" value={query} placeholder="e.g. Slack" onChange={(e) => setQuery(e.target.value)} />
          </label>
          {shown.length === 0 && <p className="modal-empty">No app matches “{query.trim()}”. Add it as a custom website instead.</p>}
          <ul className="recipe-grid">
            {shown.map((r) => (
              <li key={r.id}>
                <button type="button" className="recipe" onClick={() => onPick(r)}>
                  <span className="recipe-icon">{r.icon ? <img src={r.icon} alt="" /> : r.name[0]}</span>
                  <span className="recipe-name">{r.name}</span>
                </button>
              </li>
            ))}
            {custom && (
              <li>
                <button type="button" className="recipe custom" onClick={() => onPick(custom)}>
                  <span className="recipe-icon">+</span>
                  <span className="recipe-name">Custom website</span>
                </button>
              </li>
            )}
          </ul>
        </div>
      </form>
    </div>
  );
}
