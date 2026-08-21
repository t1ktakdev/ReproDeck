import { useMemo, useState } from "react";
import { translatedValue, useI18n } from "../i18n";
import { bytes, duration } from "../lib/format";
import { bridgeMessage, invokeTauri } from "../lib/tauri";
import type { TimelineEntry } from "../types";
import { CloseIcon } from "./Icons";

type Props = { entry: TimelineEntry | null; onClose: () => void };
type InspectorTab = "details" | "output" | "evidence";

function filterOutput(value: string, query: string): string {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return value;
  return value.split(/\r?\n/).filter(line => line.toLowerCase().includes(trimmed)).join("\n");
}

export function Inspector({ entry, onClose }: Props) {
  const { t } = useI18n();
  const [tab, setTab] = useState<InspectorTab>("details");
  const [artifactText, setArtifactText] = useState<Record<string, string>>({});
  const [artifactError, setArtifactError] = useState<string | null>(null);
  const [outputQuery, setOutputQuery] = useState("");
  const [wrapOutput, setWrapOutput] = useState(true);
  const [copyState, setCopyState] = useState<"stdout" | "stderr" | null>(null);

  async function openArtifact(id: string) {
    setArtifactError(null);
    try {
      const value = await invokeTauri<string>("read_artifact_text", { artifactId: id });
      setArtifactText(current => ({ ...current, [id]: value }));
    } catch (error) { setArtifactError(bridgeMessage(error)); }
  }

  async function copyOutput(kind: "stdout" | "stderr", value: string) {
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      setCopyState(kind);
      window.setTimeout(() => setCopyState(current => current === kind ? null : current), 1200);
    } catch (error) { setArtifactError(bridgeMessage(error)); }
  }

  const stdout = entry?.receipt?.stdout_preview ?? "";
  const stderr = entry?.receipt?.stderr_preview ?? "";
  const filteredStdout = useMemo(() => filterOutput(stdout, outputQuery), [stdout, outputQuery]);
  const filteredStderr = useMemo(() => filterOutput(stderr, outputQuery), [stderr, outputQuery]);

  if (!entry) return <aside className="inspector"><div className="inspector-title"><div><strong>{t("inspector.title")}</strong><span>{t("inspector.event")}</span></div><button className="icon-button" onClick={onClose} aria-label={t("common.close")}><CloseIcon/></button></div><div className="inspector-empty">{t("inspector.empty")}</div></aside>;
  const status = entry.execution?.status ?? entry.action.state;
  const labels: Record<InspectorTab,string> = { details: t("inspector.details"), output: t("inspector.output"), evidence: t("inspector.evidence") };

  return <aside className="inspector"><div className="inspector-title"><div><strong>{t("inspector.title")}</strong><span>{entry.action.kind.replace(/[:_-]+/g," ")}</span></div><button className="icon-button" onClick={onClose} aria-label={t("common.close")}><CloseIcon/></button></div><div className="inspector-tabs">{(["details","output","evidence"] as InspectorTab[]).map(value => <button key={value} className={tab === value ? "active" : ""} onClick={() => setTab(value)}>{labels[value]}{value === "evidence" && entry.artifacts.length ? ` (${entry.artifacts.length})` : ""}</button>)}</div><div className="inspector-scroll">
    {tab === "details" && <><span className={`status-label inspector-status ${status.toLowerCase()}`}>{translatedValue(t, "run", status)}</span><dl className="detail-list"><div><dt>{t("inspector.action")}</dt><dd>{entry.action.id}</dd></div><div><dt>{t("inspector.execution")}</dt><dd>{entry.execution?.id ?? "—"}</dd></div><div><dt>{t("inspector.duration")}</dt><dd>{duration(entry.execution?.duration_ms ?? null)}</dd></div><div><dt>{t("inspector.receipt")}</dt><dd>{entry.receipt?.id ?? "—"}</dd></div></dl>{entry.action.meta && <div className="raw-meta"><span>{t("inspector.metadata")}</span><pre>{entry.action.meta}</pre></div>}</>}
    {tab === "output" && <div className={`output-viewer ${wrapOutput ? "wrap" : "nowrap"}`}><div className="output-toolbar"><input value={outputQuery} onChange={event => setOutputQuery(event.target.value)} placeholder={t("inspector.searchOutput")} aria-label={t("inspector.searchOutput")}/><button className="text-button" onClick={() => setWrapOutput(value => !value)}>{wrapOutput ? t("inspector.noWrap") : t("inspector.wrap")}</button></div><section><header><strong>stdout</strong><span className="output-actions">{entry.receipt?.stdout_truncated && <span>{t("inspector.truncated")}</span>}<button className="text-button" disabled={!stdout} onClick={() => void copyOutput("stdout", stdout)}>{copyState === "stdout" ? t("common.copied") : t("inspector.copyStdout")}</button></span></header><pre>{filteredStdout || (outputQuery && stdout ? t("inspector.noMatches") : t("inspector.noStdout"))}</pre></section><section><header><strong>stderr</strong><span className="output-actions">{entry.receipt?.stderr_truncated && <span>{t("inspector.truncated")}</span>}<button className="text-button" disabled={!stderr} onClick={() => void copyOutput("stderr", stderr)}>{copyState === "stderr" ? t("common.copied") : t("inspector.copyStderr")}</button></span></header><pre>{filteredStderr || (outputQuery && stderr ? t("inspector.noMatches") : t("inspector.noStderr"))}</pre></section>{artifactError && <div className="inline-error">{artifactError}</div>}</div>}
    {tab === "evidence" && <div className="artifact-stack">{entry.artifacts.length === 0 && <p className="muted-copy">{t("inspector.noArtifact")}</p>}{entry.artifacts.map(artifact => <article key={artifact.id} className="artifact-card"><div><strong>{artifact.media_type || "Artifact"}</strong><span>{bytes(artifact.size)} · {artifact.checksum.slice(0,12)}…</span></div><button className="text-button" onClick={() => void openArtifact(artifact.id)}>{t("inspector.read")}</button>{artifactText[artifact.id] && <pre>{artifactText[artifact.id]}</pre>}</article>)}{artifactError && <div className="inline-error">{artifactError}</div>}</div>}
  </div></aside>;
}
