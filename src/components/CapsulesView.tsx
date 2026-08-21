import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { bytes, relativeTime } from "../lib/format";
import { bridgeMessage, chooseCapsuleFile, invokeTauri, revealLocalPath } from "../lib/tauri";
import type { CapsuleSummary, ImportedCapsule } from "../types";

export function CapsulesView() {
  const { t, language } = useI18n();
  const [items, setItems] = useState<ImportedCapsule[]>([]);
  const [preview, setPreview] = useState<CapsuleSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function load() {
    try { setItems(await invokeTauri<ImportedCapsule[]>("list_imported_capsules")); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
  }
  useEffect(() => { void load(); }, []);

  async function inspectOne(path: string) {
    setError(null);
    try { setPreview(await invokeTauri<CapsuleSummary>("inspect_capsule", { path })); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
  }

  async function importOne() {
    setError(null); setPreview(null);
    try {
      const path = await chooseCapsuleFile(t("dialog.importCapsule"), t("dialog.capsuleFilter"));
      if (!path) return;
      setBusy(true);
      const summary = await invokeTauri<CapsuleSummary>("inspect_capsule", { path });
      setPreview(summary);
      await invokeTauri<ImportedCapsule>("import_capsule", { path });
      await load();
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(false); }
  }

  return <section className="view-page root-page">
    <header className="view-heading app-page-heading"><div><h1>{t("capsules.title")}</h1><p>{t("capsules.description")}</p></div><button className="button primary" disabled={busy} onClick={() => void importOne()}>{t("capsules.import")}</button></header>
    {preview && <div className="info-strip"><strong>{t("capsules.previewImported")}: {preview.title}</strong><span>v{preview.version} · {preview.file_count} {t("capsules.files")} · {bytes(preview.total_uncompressed_bytes)}</span>{preview.redactions.length > 0 && <span>{preview.redactions.length} {t("capsules.redactions")}</span>}</div>}
    {items.length === 0 ? <div className="quiet-empty"><h3>{t("capsules.empty")}</h3></div> : <div className="data-list">
      {items.map(item => <article className="data-row capsule-row" key={item.id}><div className="data-row-main"><strong>{item.title || item.session_id || item.id}</strong><span>{item.session_id || "—"}</span></div><div className="data-row-meta"><span>v{item.format_version}</span><time>{relativeTime(item.imported_at, language)}</time><code>{item.sha256.slice(0, 12)}…</code></div><div className="row-actions"><button className="button small" onClick={() => void inspectOne(item.stored_path)}>{t("capsules.inspect")}</button><button className="button small" onClick={() => void revealLocalPath(item.stored_path).catch(nextError => setError(bridgeMessage(nextError)))}>{t("common.explorer")}</button></div></article>)}
    </div>}
    {error && <div className="inline-error page-message">{error}</div>}
  </section>;
}
