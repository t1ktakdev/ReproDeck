import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { bridgeMessage, invokeTauri, revealLocalPath } from "../lib/tauri";
import type { StoredRepository } from "../types";

export function RepositoriesView() {
  const { t } = useI18n();
  const [items, setItems] = useState<StoredRepository[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  async function load() {
    setLoading(true); setError(null);
    try { setItems(await invokeTauri<StoredRepository[]>("list_repositories")); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setLoading(false); }
  }

  useEffect(() => { void load(); }, []);

  return <section className="view-page root-page">
    <header className="view-heading app-page-heading"><div><h1>{t("repos.title")}</h1><p>{t("repos.description")}</p></div><button className="button" onClick={() => void load()}>{t("common.refresh")}</button></header>
    {loading ? <div className="loading-state">{t("common.loading")}</div> : items.length === 0 ? <div className="quiet-empty"><h3>{t("repos.empty")}</h3></div> : <div className="data-list repository-list">
      {items.map(item => <article key={item.id} className="data-row repository-row">
        <div className="data-row-main"><strong>{item.current ? item.current.path.split(/[\\/]/).filter(Boolean).slice(-1)[0] : item.path.split(/[\\/]/).filter(Boolean).slice(-1)[0]}</strong><span className="mono-wrap">{item.path}</span></div>
        <div className="data-row-meta">{item.current ? <><span>{item.current.branch}</span><code>{item.current.head_commit.slice(0, 10)}</code>{item.current.is_dirty && <span className="warning-text">{t("repos.dirty")}</span>}</> : <span className="warning-text">{t("repos.unavailable")}</span>}</div>
        <button className="button small" disabled={!item.accessible} onClick={() => void revealLocalPath(item.path).catch(nextError => setError(bridgeMessage(nextError)))}>{t("common.explorer")}</button>
      </article>)}
    </div>}
    {error && <div className="inline-error page-message">{error}</div>}
  </section>;
}
