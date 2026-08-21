import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { bridgeMessage, chooseRepositoryDirectory, invokeTauri } from "../lib/tauri";
import type { ProjectProfile } from "../types";
import { RepoIcon } from "./Icons";

type Props = { onOpenProject: (profile: ProjectProfile) => void };

function stackSummary(profile: ProjectProfile) {
  return profile.technologies.slice(0, 5).map(item => item.name).join(" · ") || profile.languages.slice(0, 4).map(item => item.language).join(" · ");
}

export function ProjectsView({ onOpenProject }: Props) {
  const { t } = useI18n();
  const [items, setItems] = useState<ProjectProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    setLoading(true); setError(null);
    try { setItems(await invokeTauri<ProjectProfile[]>("list_project_profiles")); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setLoading(false); }
  }

  async function openProject() {
    setOpening(true); setError(null);
    try {
      const selected = await chooseRepositoryDirectory(t("dialog.chooseProject"));
      if (!selected) return;
      const profile = await invokeTauri<ProjectProfile>("analyze_project", { path: selected });
      onOpenProject(profile);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setOpening(false); }
  }

  useEffect(() => { void load(); }, []);

  return <section className="view-page root-page projects-page">
    <header className="view-heading app-page-heading"><div><h1>{t("projects.title")}</h1><p>{t("projects.description")}</p></div><button className="button primary" disabled={opening} onClick={() => void openProject()}><RepoIcon/>{opening ? t("projects.analyzing") : t("projects.open")}</button></header>
    {loading ? <div className="loading-state">{t("common.loading")}</div> : items.length === 0 ? <div className="quiet-empty"><h3>{t("projects.empty")}</h3><p>{t("projects.emptyHelp")}</p></div> : <div className="project-library">
      {items.map(profile => <button key={profile.root_path} className="project-library-row" onClick={() => onOpenProject(profile)}>
        <span className="project-library-main"><strong>{profile.name}</strong><small>{profile.root_path}</small></span>
        <span className="project-library-stack">{stackSummary(profile) || t("projects.stackUnknown")}</span>
        <span className="project-library-health"><b>{profile.signals.length}</b><small>{t("projects.signals")}</small></span>
        <span className="project-library-git">{profile.git?.branch ?? "—"}</span>
      </button>)}
    </div>}
    {error && <div className="inline-error page-message">{error}</div>}
  </section>;
}
