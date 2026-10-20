import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { inspectSite } from "../services/api";
import type { NewService, SiteInfo } from "../services/types";

interface Props {
  /** Rejects with a `CommandError` (or anything) to keep the modal open and show it. */
  onSubmit: (service: NewService) => Promise<void>;
  onClose: () => void;
}

const HIBERNATE_OPTIONS = [
  { minutes: 5, label: "5 minutes" },
  { minutes: 15, label: "15 minutes" },
  { minutes: 30, label: "30 minutes" },
  { minutes: 60, label: "1 hour" },
];

/** Must match MAX_ICON_LEN in src-tauri/src/services.rs. */
const MAX_ICON_LEN = 256 * 1024;
/** Dropped raster images are scaled down to fit this square. */
const ICON_SIZE = 128;
/** Wait for typing to pause before fetching the site. */
const INSPECT_DELAY_MS = 600;

type Options = Omit<NewService, "name" | "url" | "icon">;

const DEFAULTS: Options = {
  notifications: true,
  audio: true,
  mediaIndicator: true,
  unreadBadge: true,
  enabled: true,
  // A hibernated app sends no notifications, so it's opt-in.
  hibernation: false,
  hibernateAfterMinutes: 15,
  openLinksInside: false,
};

/** "slack.com" -> "https://slack.com/"; null unless it is a plausible http(s) address. */
function normalizeUrl(input: string): string | null {
  const text = input.trim();
  if (!text) return null;
  try {
    const url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `https://${text}`);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    // A bare word ("slack") is far more likely unfinished typing than a host.
    if (!url.hostname.includes(".") && url.hostname !== "localhost" && !url.hostname.startsWith("[")) return null;
    return url.href;
  } catch {
    return null;
  }
}

function message(e: unknown): string {
  return typeof e === "object" && e !== null && "message" in e ? String(e.message) : String(e);
}

/** An image file as a data: URL: SVG as is, anything else scaled into ICON_SIZE as PNG. */
async function iconFromFile(file: File): Promise<string> {
  if (!file.type.startsWith("image/")) throw new Error(`${file.name} is not an image`);
  const data = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error(`couldn't read ${file.name}`));
    reader.readAsDataURL(file);
  });
  if (file.type === "image/svg+xml") {
    if (data.length > MAX_ICON_LEN) throw new Error("that SVG is too large (256 KiB at most)");
    return data;
  }
  const img = new Image();
  img.src = data;
  await img.decode();
  const scale = Math.min(1, ICON_SIZE / Math.max(img.naturalWidth, img.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("couldn't process the image");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/png");
}

type Site =
  | { status: "idle" }
  | { status: "loading"; url: string }
  | { status: "done"; url: string; info: SiteInfo }
  | { status: "error"; url: string; error: string };

/** "Add app": name, address, icon and per-app options. Rendered while the shell covers the window. */
export function AddAppModal({ onSubmit, onClose }: Props) {
  const [name, setName] = useState("");
  const [address, setAddress] = useState("");
  const [customIcon, setCustomIcon] = useState<string | null>(null);
  const [iconError, setIconError] = useState<string | null>(null);
  const [opts, setOpts] = useState<Options>(DEFAULTS);
  const [site, setSite] = useState<Site>({ status: "idle" });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();

  const url = normalizeUrl(address);
  const siteInfo = site.status === "done" && site.url === url ? site.info : null;
  const icon = customIcon ?? siteInfo?.icon ?? null;
  const fallbackName = siteInfo?.name ?? (url ? new URL(url).hostname.replace(/^www\./, "") : "");
  const set = <K extends keyof Options>(key: K) => (value: Options[K]) => setOpts((o) => ({ ...o, [key]: value }));

  // Fetch the site's name and icon once the address stops changing.
  useEffect(() => {
    if (!url) {
      setSite({ status: "idle" });
      return;
    }
    let current = true;
    setSite({ status: "loading", url });
    const timer = setTimeout(() => {
      inspectSite(url).then(
        (info) => current && setSite({ status: "done", url, info }),
        (e) => current && setSite({ status: "error", url, error: message(e) }),
      );
    }, INSPECT_DELAY_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [url]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !submitting && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, submitting]);

  const pickIcon = (file: File | undefined) => {
    if (!file) return;
    setIconError(null);
    iconFromFile(file).then(setCustomIcon, (e) => setIconError(message(e)));
  };

  const submit = async () => {
    if (!url || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit({ ...opts, name: name.trim() || fallbackName, url, icon });
    } catch (e) {
      setError(message(e));
      setSubmitting(false);
    }
  };

  let iconStatus: string;
  if (customIcon) iconStatus = "Custom icon";
  else if (!url) iconStatus = "Enter the site address to use its icon";
  else if (site.status === "loading") iconStatus = "Looking up the site's icon…";
  else if (site.status === "error") iconStatus = "Couldn't reach the site; drop an icon instead";
  else if (siteInfo?.icon) iconStatus = "The site's icon";
  else iconStatus = "The site has no icon; drop one instead";

  return (
    <div className="modal-backdrop">
      <form
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <header className="modal-header">
          <h2 id={titleId}>Add app</h2>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose} disabled={submitting}>
            ×
          </button>
        </header>

        <div className="modal-body">
          <div className="identity">
            <IconDrop icon={icon} letter={(name.trim() || fallbackName || "?")[0]} onFile={pickIcon} />
            <div className="identity-fields">
              <label className="text-field">
                <span>Name</span>
                <input
                  autoFocus
                  value={name}
                  maxLength={64}
                  placeholder={fallbackName || "e.g. Slack"}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <label className="text-field">
                <span>Site address</span>
                <input
                  type="url"
                  inputMode="url"
                  spellCheck={false}
                  value={address}
                  placeholder="https://app.slack.com"
                  aria-invalid={address.trim() !== "" && !url}
                  onChange={(e) => setAddress(e.target.value)}
                />
              </label>
            </div>
          </div>
          <p className="icon-status">
            {iconStatus}
            {customIcon && (
              <button type="button" className="link-button" onClick={() => setCustomIcon(null)}>
                Use the site's icon
              </button>
            )}
            {iconError && <span className="field-error">{iconError}</span>}
          </p>

          <Section title="Notifications">
            <Switch label="Enable notifications" checked={opts.notifications} onChange={set("notifications")} />
            <Switch
              label="Enable audio"
              hint="Off mutes everything the app plays, its notification sounds included"
              checked={opts.audio}
              onChange={set("audio")}
            />
            <Switch
              label="Enable media play indicator"
              hint="A speaker next to the app in the sidebar while it plays audio"
              checked={opts.mediaIndicator}
              onChange={set("mediaIndicator")}
            />
          </Section>

          <Section title="Unread messages">
            <Switch label="Show unread messages badge" checked={opts.unreadBadge} onChange={set("unreadBadge")} />
          </Section>

          <Section title="General">
            <Switch
              label="Enable service"
              hint="A disabled app stays in the sidebar, greyed out, and never loads"
              checked={opts.enabled}
              onChange={set("enabled")}
            />
            <Switch
              label="Enable hibernation"
              hint="Unloads the app while it's in the background, to save memory. A hibernated app sends no notifications"
              checked={opts.hibernation}
              onChange={set("hibernation")}
            />
            <label className={"row-field" + (opts.hibernation ? "" : " disabled")}>
              <span>Hibernate after</span>
              <select
                value={opts.hibernateAfterMinutes}
                disabled={!opts.hibernation}
                onChange={(e) => set("hibernateAfterMinutes")(Number(e.target.value))}
              >
                {HIBERNATE_OPTIONS.map((o) => (
                  <option key={o.minutes} value={o.minutes}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            <Switch
              label="Open URLs in uHub"
              hint="Links to other sites open in a uHub window instead of your browser"
              checked={opts.openLinksInside}
              onChange={set("openLinksInside")}
            />
          </Section>
        </div>

        <footer className="modal-footer">
          {error && <p className="field-error">{error}</p>}
          <button type="button" className="button" onClick={onClose} disabled={submitting}>
            Cancel
          </button>
          <button type="submit" className="button primary" disabled={!url || submitting}>
            {submitting ? "Adding…" : "Add app"}
          </button>
        </footer>
      </form>
    </div>
  );
}

/** The icon preview, which takes a dropped image or a click to choose one. */
function IconDrop({ icon, letter, onFile }: { icon: string | null; letter: string; onFile: (f: File | undefined) => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  return (
    <>
      <button
        type="button"
        className={"icon-drop" + (over ? " over" : "")}
        title="Drop an image here, or click to choose one"
        onClick={() => input.current?.click()}
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          onFile(e.dataTransfer.files[0]);
        }}
      >
        {icon ? <img src={icon} alt="" /> : <span className="icon-letter">{letter.toUpperCase()}</span>}
        <span className="icon-drop-hint">Drop or click</span>
      </button>
      {/* Outside the button: its click would bubble back into the button's. */}
      <input
        ref={input}
        type="file"
        accept="image/*"
        hidden
        onChange={(e) => {
          onFile(e.target.files?.[0]);
          e.target.value = "";
        }}
      />
    </>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="section">
      <legend>{title}</legend>
      {children}
    </fieldset>
  );
}

function Switch({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="row-field">
      <span>
        {label}
        {hint && <small>{hint}</small>}
      </span>
      <input type="checkbox" role="switch" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}
