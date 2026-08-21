import { useEffect, useState } from "react";
import { translatedValue, useI18n } from "../i18n";
import type { ProjectProfile, RecoveryEntry, Session } from "../types";
import { relativeTime, sessionMeta } from "../lib/format";
import { bridgeMessage, chooseRepositoryDirectory, confirmAction, hasTauriRuntime, invokeTauri, revealLocalPath } from "../lib/tauri";
import { RepoIcon, SessionIcon } from "./Icons";

type Props = {
  sessions: Session[];
  onOpenProject: (profile: ProjectProfile) => void;
  onOpenSession: (id: string) => void;
  onImportCapsule: () => void;
};

export function HomeView({ sessions, onOpenProject, onOpenSession, onImportCapsule }: Props) {
  const { t, language } = useI18n();
  const [projects, setProjects] = useState<ProjectProfile[]>([]);
  const [recovery, setRecovery] = useState<RecoveryEntry[]>([]);
  const [opening, setOpening] = useState(false);
  const [recoveryBusy, setRecoveryBusy] = useState<string | null>(null);
  const [demoBusy, setDemoBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hasTauriRuntime()) return;
    let cancelled = false;
    void invokeTauri<ProjectProfile[]>("list_project_profiles")
      .then(items => { if (!cancelled) setProjects(items.slice(0, 8)); })
      .catch(nextError => { if (!cancelled) setError(bridgeMessage(nextError)); });
    void invokeTauri<RecoveryEntry[]>("list_pending_recovery")
      .then(pending => { if (!cancelled) setRecovery(pending); })
      .catch(nextError => { if (!cancelled) setError(bridgeMessage(nextError)); });
    return () => { cancelled = true; };
  }, []);

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

  async function retryRecovery(entry: RecoveryEntry) {
    if (!await confirmAction(t("recovery.confirm"))) return;
    setRecoveryBusy(entry.id); setError(null);
    try {
      await invokeTauri<void>("retry_pending_recovery", { id: entry.id, confirmed: true });
      setRecovery(current => current.filter(item => item.id !== entry.id));
    } catch (nextError) {
      setError(bridgeMessage(nextError));
      try { setRecovery(await invokeTauri<RecoveryEntry[]>("list_pending_recovery")); } catch { /* keep the reviewed entry visible */ }
    } finally { setRecoveryBusy(null); }
  }

  async function tryDemo() {
    setDemoBusy(true); setError(null);
    try { onOpenProject(await invokeTauri<ProjectProfile>("create_demo_project")); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setDemoBusy(false); }
  }

  return <section className="view-page root-page home-view">
    <header className="view-heading app-page-heading"><div><h1>{t("home.title")}</h1><p>{t("home.description")}</p></div></header>

    {recovery.length > 0 && <section className="recovery-review" aria-labelledby="recovery-title">
      <header><div><h2 id="recovery-title">{t("recovery.title")}</h2><p>{t("recovery.description")}</p></div><span>{recovery.length}</span></header>
      {recovery.map(entry => <article key={entry.id}>
        <div><strong>{entry.branch}</strong><small>{entry.repo_path}</small><code>{entry.worktree_path}</code>{entry.last_error && <p>{entry.last_error}</p>}</div>
        <div className="recovery-actions"><button className="button small" onClick={() => void revealLocalPath(entry.worktree_path).catch(nextError => setError(bridgeMessage(nextError)))}>{t("recovery.review")}</button><button className="button danger small" disabled={recoveryBusy !== null} onClick={() => void retryRecovery(entry)}>{recoveryBusy === entry.id ? t("common.working") : t("recovery.retryCleanup")}</button></div>
      </article>)}
      <footer>{t("recovery.safety")}</footer>
    </section>}

    <div className="start-layout">
      <section className="start-section">
        <header className="section-header"><h2>{t("home.start")}</h2></header>
        <div className="start-actions">
          <button disabled={opening} onClick={() => void openProject()}><RepoIcon/><span><strong>{opening ? t("projects.analyzing") : t("home.openProject")}</strong><small>{t("home.openProjectHelp")}</small></span></button>
          <button onClick={onImportCapsule}><SessionIcon/><span><strong>{t("home.importCapsule")}</strong><small>{t("home.importCapsuleHelp")}</small></span></button>
        </div>
        {projects.length === 0 && <div className="first-run-path"><strong>{t("home.firstRunTitle")}</strong><ol><li>{t("home.firstRunChecks")}</li><li>{t("home.firstRunFailure")}</li><li>{t("home.firstRunEvidence")}</li><li>{t("home.firstRunVerify")}</li></ol><button className="button" disabled={demoBusy} onClick={() => void tryDemo()}>{demoBusy?t("home.creatingDemo"):t("home.tryDemo")}</button><small>{t("home.tryDemoHelp")}</small></div>}
      </section>

      <section className="start-section recent-section"><header className="section-header"><h2>{t("home.recentProjects")}</h2><span>{projects.length}</span></header>
        {projects.length === 0 ? <div className="plain-empty"><strong>{t("home.noProjects")}</strong><span>{t("home.noProjectsHelp")}</span></div> : <div className="recent-list">{projects.map(project => <button key={project.root_path} onClick={() => onOpenProject(project)}>
          <span className="recent-session-main"><strong>{project.name}</strong><small>{project.root_path}</small></span>
          <span className="recent-session-state">{project.signals.length} {t("projects.signals")}</span>
        </button>)}</div>}
      </section>
    </div>

    <section className="home-repositories"><header className="section-header"><h2>{t("home.recentSessions")}</h2><span>{sessions.length}</span></header>
      {sessions.length === 0 ? <div className="plain-empty"><strong>{t("home.noSessions")}</strong><span>{t("home.noSessionsHelp")}</span></div> : <div className="repository-compact-list">{sessions.slice(0, 8).map(session => {
        const meta = sessionMeta(session);
        return <button key={session.id} onClick={() => onOpenSession(session.id)}><span><strong>{meta.title || session.id}</strong><small>{relativeTime(session.updated_at ?? session.created_at, language)}</small></span><span>{translatedValue(t, "state", session.state)}</span></button>;
      })}</div>}
    </section>
    {error && <div className="inline-error page-message">{error}</div>}
  </section>;
}
