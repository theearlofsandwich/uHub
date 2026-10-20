import { useEffect, useId, useState } from "react";
import { getPerformance } from "../services/api";
import type { Performance, Service, Usage } from "../services/types";
import { Section } from "./AppModal";
import { isHibernated } from "./Hibernated";

interface Props {
  /** For names, icons and states; usage is measured per profile. */
  services: Service[];
  onClose: () => void;
}

/** How long after one measurement the next is taken, in ms. */
const REFRESH = 2000;

/** Services sharing a profile share its processes, so they're measured as one row. */
interface Row {
  profile: string;
  services: Service[];
}

function rows(services: Service[]): Row[] {
  const byProfile = new Map<string, Row>();
  for (const s of services) {
    const row = byProfile.get(s.profile);
    if (row) row.services.push(s);
    else byProfile.set(s.profile, { profile: s.profile, services: [s] });
  }
  return [...byProfile.values()];
}

function formatMemory(bytes: number): string {
  const mb = bytes / 1024 ** 2;
  if (mb < 1000) return `${Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(mb < 10 * 1024 ? 2 : 1)} GB`;
}

function formatCpu(cpu: number | null): string {
  if (cpu === null) return "–";
  if (cpu > 0 && cpu < 0.1) return "<0.1%";
  return `${cpu < 10 ? cpu.toFixed(1) : Math.round(cpu)}%`;
}

/** Memory and CPU use of uHub and each app, measured while open. Rendered while the shell covers the window, like `SettingsModal`. */
export function PerformanceModal({ services, onClose }: Props) {
  const titleId = useId();
  const [perf, setPerf] = useState<Performance | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // The next measurement is taken once the last one is in, so slow ones never pile up.
  useEffect(() => {
    let timer: number | undefined;
    let stopped = false;
    const measure = () =>
      getPerformance()
        .then(
          (p) => {
            if (stopped) return;
            setPerf(p);
            setError(null);
          },
          (e) => !stopped && setError(String((e as { message?: unknown })?.message ?? e)),
        )
        .finally(() => {
          if (!stopped) timer = window.setTimeout(measure, REFRESH);
        });
    void measure();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, []);

  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <header className="modal-header">
          <h2 id={titleId}>Performance</h2>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}>
            ×
          </button>
        </header>

        <div className="modal-body">
          {!perf ? (
            <p className="modal-empty">{error ? "Couldn't measure." : "Measuring…"}</p>
          ) : !perf.supported ? (
            <p className="modal-empty">Performance metrics are only available on Linux so far.</p>
          ) : (
            <>
              <div className="perf-summary">
                <div className="perf-stat">
                  <span>Memory</span>
                  <strong>{formatMemory(perf.total.memory)}</strong>
                  <small>{perf.systemMemory ? `of ${formatMemory(perf.systemMemory)} on this computer` : "in total"}</small>
                </div>
                <div className="perf-stat">
                  <span>CPU</span>
                  <strong>{formatCpu(perf.total.cpu)}</strong>
                  <small>100% is one core</small>
                </div>
              </div>
              <Section title="Apps">
                {rows(services).map((row) => (
                  <PerfRow key={row.profile} row={row} usage={perf.profiles[row.profile]} total={perf.total.memory} />
                ))}
                <PerfRow usage={perf.uhub} total={perf.total.memory} />
              </Section>
              <p className="perf-note">
                Memory counts shared memory once, split between the processes sharing it; graphics memory isn't
                included.
              </p>
            </>
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

/** One app's (or, without `row`, uHub's own) usage, with its share of the total memory as a bar. */
function PerfRow({ row, usage, total }: { row?: Row; usage: Usage | undefined; total: number }) {
  const first = row?.services[0];
  const name = row ? row.services.map((s) => s.name).join(", ") : "uHub";
  const enabled = row?.services.filter((s) => s.enabled);
  // From the apps' states, not the usage: a hibernated app may still have processes
  // (auto-wake loads it behind the "App hibernated" page), and one without may be loading.
  const status = !enabled
    ? undefined
    : enabled.length === 0
      ? "Disabled"
      : enabled.every(isHibernated)
        ? "Hibernated"
        : undefined;
  const hint = row
    ? [row.services.length > 1 && "Sharing a profile", status].filter(Boolean).join(" · ") || undefined
    : "The app bar, this panel and helpers";
  const share = usage && total > 0 ? usage.memory / total : 0;
  return (
    <div className={"row-field perf-row" + (usage && !status ? "" : " idle") + (status === "Disabled" ? " off" : "")}>
      <span className={first?.icon ? "service-icon has-image" : "service-icon"}>
        {first?.icon ? <img src={first.icon} alt="" /> : <span className="icon-letter">{(first?.name ?? "u")[0]?.toUpperCase()}</span>}
      </span>
      <span className="perf-name">
        <span className="perf-label">{name}</span>
        {hint && <small>{hint}</small>}
      </span>
      <span className="perf-bar" aria-hidden="true">
        <span style={{ width: `${share * 100}%` }} />
      </span>
      <span className="perf-value">{usage ? formatMemory(usage.memory) : "–"}</span>
      <span className="perf-value">{usage ? formatCpu(usage.cpu) : "–"}</span>
    </div>
  );
}
