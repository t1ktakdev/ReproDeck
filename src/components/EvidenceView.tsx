import { useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { bytes } from "../lib/format";
import { bridgeMessage, invokeTauri } from "../lib/tauri";
import type { ArtifactRecord, EvidenceItem, TimelineEntry } from "../types";

type Props = { items: EvidenceItem[]; entries: TimelineEntry[] };

export function EvidenceView({ items, entries }: Props) {
  const { t } = useI18n();
  const artifacts = useMemo(() => {
    const byId = new Map<string, ArtifactRecord>();
    for (const entry of entries) for (const artifact of entry.artifacts) byId.set(artifact.id, artifact);
    return byId;
  }, [entries]);
  const [selected, setSelected] = useState<EvidenceItem | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function inspect(item: EvidenceItem) {
    setSelected(item); setText(null); setError(null);
    if (!item.artifact_id) return;
    try { setText(await invokeTauri<string>("read_artifact_text", { artifactId: item.artifact_id })); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
  }

  return <section className="view-page">
    <header className="view-heading"><div><h2>{t("evidence.title")}</h2><p>{t("evidence.description")}</p></div><span>{items.length}</span></header>
    {items.length === 0 ? <div className="quiet-empty"><h3>{t("evidence.empty")}</h3><p>{t("evidence.emptyHelp")}</p></div> : <div className="evidence-layout">
      <div className="evidence-list">{items.map(item => {
        const artifact = item.artifact_id ? artifacts.get(item.artifact_id) : undefined;
        return <button key={item.id} className={selected?.id === item.id ? "selected" : ""} onClick={() => void inspect(item)}>
          <div><strong>{item.summary}</strong><span>{artifact ? bytes(artifact.size) : item.kind.replace(/_/g, " ")}</span></div>
          <small>{item.source}{item.checksum ? ` · ${item.checksum.slice(0, 12)}…` : ""}</small>
        </button>;
      })}</div>
      <div className="evidence-preview">{selected ? <>
        <header><strong>{selected.summary}</strong><code>{selected.kind}</code></header>
        <dl className="detail-list"><div><dt>{t("evidence.source")}</dt><dd>{selected.source}</dd></div><div><dt>ID</dt><dd>{selected.id}</dd></div>{selected.checksum && <div><dt>SHA-256</dt><dd>{selected.checksum}</dd></div>}</dl>
        {selected.artifact_id ? text !== null ? <pre>{text}</pre> : !error && <p className="muted-copy">{t("common.loading")}</p> : <p className="muted-copy">{selected.summary}</p>}
        {error && <div className="inline-error">{error}</div>}
      </> : <p className="muted-copy">{t("evidence.emptyHelp")}</p>}</div>
    </div>}
  </section>;
}
