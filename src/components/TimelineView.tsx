import { translatedValue, useI18n } from "../i18n";
import { clockTime, commandText, duration } from "../lib/format";
import type { TimelineEntry } from "../types";

function metaOf(entry: TimelineEntry): Record<string, unknown> {
  if (!entry.action.meta) return {};
  try { const value = JSON.parse(entry.action.meta); return value && typeof value === "object" ? value as Record<string, unknown> : {}; }
  catch { return {}; }
}
function commandOf(entry: TimelineEntry): string | null {
  const command = metaOf(entry).command;
  if (!command || typeof command !== "object") return null;
  const value = command as Record<string, unknown>;
  const executable = typeof value.executable === "string" ? value.executable : "";
  const args = Array.isArray(value.args) ? value.args.filter((item): item is string => typeof item === "string") : [];
  return executable ? commandText(executable, args) : null;
}
function statusClass(status: string): string {
  const value = status.toLowerCase();
  if (value === "passed" || value === "succeeded") return "passed";
  if (value === "failed" || value === "error") return "failed";
  if (value === "running") return "running";
  return "neutral";
}

type Props = { entries: TimelineEntry[]; selectedId: string | null; onSelect: (id: string) => void };
export function TimelineView({ entries, selectedId, onSelect }: Props) {
  const { t } = useI18n();
  const chronological = [...entries].reverse();
  const titleOf = (entry: TimelineEntry) => {
    const meta = metaOf(entry);
    if (entry.action.kind === "environment:capture") return t("timeline.environmentCaptured");
    if (entry.action.kind === "reproduction:command") return `${typeof meta.phase === "string" ? meta.phase : t("overview.reproduction")} · ${t("timeline.command")}`;
    if (entry.action.kind === "verification:baseline-reset") return t("timeline.baselineReset");
    return entry.action.kind.replace(/[:_-]+/g, " ").replace(/^./, c => c.toUpperCase());
  };
  return <section className="view-page timeline-page"><header className="view-heading"><div><h2>{t("nav.timeline")}</h2><p>{t("timeline.description")}</p></div><span>{entries.length}</span></header>
    {chronological.length === 0 ? <div className="quiet-empty"><h3>{t("timeline.empty")}</h3><p>{t("timeline.emptyHelp")}</p></div> : <div className="timeline-list">{chronological.map(entry => {
      const status = entry.execution?.status ?? entry.action.state;
      const command = commandOf(entry);
      return <button key={entry.action.id} className={selectedId === entry.action.id ? "timeline-item selected" : "timeline-item"} onClick={() => onSelect(entry.action.id)}>
        <div className="timeline-time">{clockTime(entry.action.created_at)}</div><div className="timeline-rail"><span className={`timeline-dot ${statusClass(status)}`}/></div>
        <div className="timeline-main"><div className="timeline-title"><strong>{titleOf(entry)}</strong><span className={`status-label ${statusClass(status)}`}>{translatedValue(t, "run", status)}</span></div>{command && <code>{command}</code>}<small>{entry.execution ? duration(entry.execution.duration_ms) : t("timeline.recorded")}{entry.artifacts.length ? ` · ${entry.artifacts.length} ${t("nav.evidence").toLowerCase()}` : ""}</small></div>
      </button>;
    })}</div>}
  </section>;
}
