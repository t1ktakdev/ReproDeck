import { useI18n } from "../i18n";
import type { ProjectProfile, ProjectTab, RootView, Session, WorkspaceTab } from "../types";
import { relativeTime, sessionMeta } from "../lib/format";
import { CapsuleIcon, ChangesIcon, EnvIcon, EvidenceIcon, HomeIcon, OverviewIcon, PanelIcon, RepoIcon, SearchIcon, SessionIcon, SettingsIcon, TimelineIcon, VerifyIcon, WarningIcon } from "./Icons";
import { ResizeHandle, Tooltip } from "./ui";

type Props = {
  sessions: Session[];
  selectedSessionId: string | null;
  selectedProject: ProjectProfile | null;
  tab: WorkspaceTab;
  projectTab: ProjectTab;
  onSelectSession: (id: string) => void;
  onTab: (tab: WorkspaceTab) => void;
  onProjectTab: (tab: ProjectTab) => void;
  onNewSession: () => void;
  onRoot: (view: RootView) => void;
  rootView: RootView;
  compact: boolean;
  width: number;
  onToggleCompact: () => void;
  onWidthChange: (width: number) => void;
  onWidthCommit: (width: number) => void;
};

export function Sidebar({ sessions, selectedSessionId, selectedProject, tab, projectTab, onSelectSession, onTab, onProjectTab, onNewSession, onRoot, rootView, compact, width, onToggleCompact, onWidthChange, onWidthCommit }: Props) {
  const { t, language } = useI18n();
  const sessionWorkspace = [
    ["overview", t("nav.overview"), OverviewIcon], ["timeline", t("nav.timeline"), TimelineIcon], ["evidence", t("nav.evidence"), EvidenceIcon],
    ["changes", t("nav.changes"), ChangesIcon], ["environment", t("nav.environment"), EnvIcon], ["verification", t("nav.verification"), VerifyIcon],
  ] as const;
  const projectWorkspace = [
    ["project-overview", t("nav.projectOverview"), OverviewIcon], ["problems", t("nav.problems"), WarningIcon], ["agent", t("nav.agent"), SearchIcon], ["checks", t("nav.checks"), VerifyIcon],
  ] as const;
  const roots = [
    ["home", t("nav.home"), HomeIcon], ["projects", t("nav.projects"), RepoIcon], ["sessions", t("nav.sessions"), SessionIcon], ["capsules", t("nav.capsules"), CapsuleIcon],
  ] as const;
  const hasContext = Boolean(selectedProject || selectedSessionId);

  const contextTitle = selectedProject?.name ?? (selectedSessionId ? t("nav.thisSession") : "ReproDeck");
  const contextMeta = selectedProject
    ? (selectedProject.git?.branch ?? t("project.notGit"))
    : selectedSessionId
      ? sessions.find(item => item.id === selectedSessionId)?.id.slice(0, 12) ?? ""
      : t("app.subtitle");

  return <aside className={`sidebar workbench-sidebar ${compact ? "compact" : ""} ${hasContext ? "has-context" : "rail-only"}`} style={{ width: compact || !hasContext ? 54 : width }}>
    <nav className="activity-rail" aria-label="Main navigation">
      <Tooltip label="ReproDeck"><button className="rail-brand" onClick={() => onRoot("home")} aria-label="ReproDeck"><span>R</span></button></Tooltip>
      <div className="rail-group">
        {roots.map(([value, label, Icon]) => <Tooltip key={value} label={label}><button aria-label={label} className={!selectedSessionId && !selectedProject && rootView === value ? "active" : ""} onClick={() => onRoot(value as RootView)}><Icon/>{value === "sessions" && sessions.length > 0 && <span className="rail-badge">{Math.min(sessions.length, 9)}</span>}</button></Tooltip>)}
      </div>
      <div className="rail-spacer"/>
      {hasContext && <Tooltip label={compact ? t("nav.expandSidebar") : t("nav.collapseSidebar")} shortcut="Ctrl B"><button aria-label={compact ? t("nav.expandSidebar") : t("nav.collapseSidebar")} onClick={onToggleCompact}><PanelIcon/></button></Tooltip>}
      <Tooltip label={t("nav.settings")}><button aria-label={t("nav.settings")} className={!selectedSessionId && !selectedProject && rootView === "settings" ? "active" : ""} onClick={() => onRoot("settings")}><SettingsIcon/></button></Tooltip>
    </nav>

    {hasContext && <section className="context-sidebar">
      <header className="context-sidebar-header">
        <div className="context-title"><strong title={contextTitle}>{contextTitle}</strong><span>{contextMeta}</span></div>
        <button className="context-new" onClick={onNewSession} title={t("nav.newSession")}><span>+</span><kbd>Ctrl N</kbd></button>
      </header>

      {selectedProject ? <>
        <div className="context-section-label">{t("nav.thisProject")}</div>
        <nav className="context-nav" aria-label="Project workspace">
          {projectWorkspace.map(([value, label, Icon]) => <button key={value} className={projectTab === value ? "active" : ""} onClick={() => onProjectTab(value)}><Icon/><span>{label}</span></button>)}
        </nav>
      </> : <>
        <div className="context-section-label">{t("nav.thisSession")}</div>
        <nav className="context-nav" aria-label="Session workspace">
          {sessionWorkspace.map(([value, label, Icon]) => <button key={value} className={tab === value ? "active" : ""} onClick={() => onTab(value)}><Icon/><span>{label}</span></button>)}
        </nav>
      </>}

      <div className="context-section-label recent-title">{t("nav.recentProofs")}</div>
      <div className="session-list context-recents">
        {sessions.slice(0, 8).map(session => {
          const meta = sessionMeta(session);
          return <button key={session.id} className={session.id === selectedSessionId ? "selected" : ""} onClick={() => onSelectSession(session.id)}>
            <div><span className={`state-dot state-${session.state.toLowerCase()}`}/><strong>{meta.title || session.id}</strong></div>
            <small>{relativeTime(session.updated_at ?? session.created_at, language)}</small>
          </button>;
        })}
        {sessions.length === 0 && <p className="sidebar-empty">—</p>}
      </div>

      <footer className="context-sidebar-foot"><span className="privacy-dot"/>{t("nav.storedLocal")}</footer>
    </section>}
    {!compact && hasContext && <ResizeHandle side="right" value={width} min={220} max={340} label={t("settings.sidebarWidth")} onChange={onWidthChange} onCommit={onWidthCommit}/>}
  </aside>;
}
