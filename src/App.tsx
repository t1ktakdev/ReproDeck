import { useCallback, useEffect, useMemo, useState, type CSSProperties } from "react";
import "./App.css";
import "./styles/verification.css";
import { CapsulesView } from "./components/CapsulesView";
import { ChangesView } from "./components/ChangesView";
import { EnvironmentView } from "./components/EnvironmentView";
import { EvidenceView } from "./components/EvidenceView";
import { HomeView } from "./components/HomeView";
import { Inspector } from "./components/Inspector";
import { InspectorIcon, PanelIcon, SearchIcon } from "./components/Icons";
import { NewSessionWizard, type VerificationSessionDraft } from "./components/NewSessionWizard";
import { OverviewView } from "./components/OverviewView";
import { ProjectsView } from "./components/ProjectsView";
import { ProjectWorkspace } from "./components/ProjectWorkspace";
import { SessionsView } from "./components/SessionsView";
import { SettingsView } from "./components/SettingsView";
import { Sidebar } from "./components/Sidebar";
import { TimelineView } from "./components/TimelineView";
import { ResizeHandle } from "./components/ui";
import { VerificationView } from "./components/VerificationView";
import { I18nProvider, translatedValue, useI18n } from "./i18n";
import { repoName, sessionMeta } from "./lib/format";
import { formatArguments } from "./lib/args";
import { regressionRecommendations } from "./lib/project";
import { bridgeMessage, chooseRepositoryDirectory, hasTauriRuntime, invokeTauri } from "./lib/tauri";
import { commandPaletteCommandIds, globalShortcut, migrateAppSettings, rememberedWorkbenchState } from "./lib/uiBehavior";
import { usePresence } from "./lib/usePresence";
import type {
  AppSettings,
  EnvironmentSnapshot,
  EvidenceItem,
  InvestigationCase,
  ProjectProfile,
  ProjectTab,
  RepositoryInfo,
  ReproductionRun,
  ReproductionStep,
  RootView,
  Session,
  ShadowWorkspace,
  TimelineEntry,
  WorkspaceSettings,
  WorkspaceTab,
} from "./types";
import { DEFAULT_SETTINGS } from "./types";

type RuntimeState = "checking" | "tauri" | "browser";
type ToastNotice = { id: number; tone: "success" | "danger" | "neutral"; title: string; message?: string; closing?: boolean };
type PaletteItem = { id: string; label: string; detail?: string; shortcut?: string; action: () => void | Promise<void> };

function applyPreferences(settings: AppSettings) {
  const root = document.documentElement;
  root.dataset.theme = settings.theme;
  root.dataset.density = settings.ui.density;
  root.dataset.fontSize = settings.ui.font_size;
  root.dataset.motion = !settings.ui.animations || settings.ui.reduced_motion ? "off" : "on";
  root.style.colorScheme = settings.theme === "system" ? "dark light" : settings.theme;
  root.style.setProperty("--mono-font-size", `${settings.ui.mono_font_size}px`);
  root.style.setProperty("--interface-zoom", String(settings.ui.zoom / 100));
}

function ReproDeckApp({ settings, onSettings }: { settings: AppSettings; onSettings: (value: AppSettings) => Promise<void> }) {
  const { t } = useI18n();
  const [runtime, setRuntime] = useState<RuntimeState>(() => hasTauriRuntime() ? "checking" : "browser");
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<ProjectProfile | null>(null);
  const [repository, setRepository] = useState<RepositoryInfo | null>(null);
  const [shadow, setShadow] = useState<ShadowWorkspace | null>(null);
  const [steps, setSteps] = useState<ReproductionStep[]>([]);
  const [runs, setRuns] = useState<ReproductionRun[]>([]);
  const [timeline, setTimeline] = useState<TimelineEntry[]>([]);
  const [evidence, setEvidence] = useState<EvidenceItem[]>([]);
  const [environment, setEnvironment] = useState<EnvironmentSnapshot | null>(null);
  const [tab, setTabState] = useState<WorkspaceTab>("overview");
  const [projectTab, setProjectTabState] = useState<ProjectTab>("project-overview");
  const [investigationSeed, setInvestigationSeed] = useState("");
  const [rootView, setRootView] = useState<RootView>("home");
  const [selectedActionId, setSelectedActionId] = useState<string | null>(null);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [newSessionDraft, setNewSessionDraft] = useState<VerificationSessionDraft | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState("");
  const [paletteIndex, setPaletteIndex] = useState(0);
  const [sidebarCompact, setSidebarCompact] = useState(settings.ui.sidebar_mode === "compact");
  const [sidebarWidth, setSidebarWidth] = useState(settings.ui.remember_sidebar_width ? settings.ui.sidebar_width : DEFAULT_SETTINGS.ui.sidebar_width);
  const [inspectorOpen, setInspectorOpen] = useState(settings.ui.remember_inspector_state ? settings.ui.inspector_open : false);
  const [inspectorWidth, setInspectorWidth] = useState(settings.ui.remember_inspector_width ? settings.ui.inspector_width : DEFAULT_SETTINGS.ui.inspector_width);
  const [toasts, setToasts] = useState<ToastNotice[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const selectedSession = sessions.find(session => session.id === selectedSessionId) ?? null;
  const selectedEntry = timeline.find(entry => entry.action.id === selectedActionId) ?? null;
  const meta = sessionMeta(selectedSession);
  const palettePresence = usePresence(paletteOpen, 180);
  const newSessionPresence = usePresence(newSessionOpen, 200);

  function dismissToast(id: number) {
    setToasts(current => current.map(item => item.id === id ? { ...item, closing: true } : item));
    window.setTimeout(() => setToasts(current => current.filter(item => item.id !== id)), 180);
  }

  useEffect(() => {
    setSidebarCompact(settings.ui.sidebar_mode === "compact");
    const remembered = rememberedWorkbenchState(settings, { sidebarWidth, inspectorWidth, inspectorOpen });
    setSidebarWidth(remembered.sidebarWidth);
    setInspectorWidth(remembered.inspectorWidth);
    setInspectorOpen(remembered.inspectorOpen);
  }, [settings.ui]);

  useEffect(() => {
    const notify = (event: Event) => {
      if (!settings.behavior.notifications) return;
      const detail = (event as CustomEvent<Omit<ToastNotice, "id">>).detail;
      const notice = { ...detail, id: Date.now() + Math.random() };
      setToasts(current => [...current.slice(-2), notice]);
      window.setTimeout(() => dismissToast(notice.id), 4200);
    };
    window.addEventListener("reprodeck:notify", notify);
    return () => window.removeEventListener("reprodeck:notify", notify);
  }, [settings.behavior.notifications]);

  const updateWorkspaceSetting = useCallback((workspace: WorkspaceSettings) => {
    void onSettings({ ...settings, workspace }).catch(nextError => setError(bridgeMessage(nextError)));
  }, [onSettings, settings]);

  const loadSessionData = useCallback(async (sessionId: string) => {
    const [nextRepository, nextShadow, nextSteps, nextRuns, nextTimeline, nextEvidence, nextEnvironment] = await Promise.all([
      invokeTauri<RepositoryInfo | null>("get_session_repository", { sessionId }),
      invokeTauri<ShadowWorkspace | null>("get_shadow_workspace", { sessionId }),
      invokeTauri<ReproductionStep[]>("list_reproduction_steps", { sessionId }),
      invokeTauri<ReproductionRun[]>("list_reproduction_runs", { sessionId }),
      invokeTauri<TimelineEntry[]>("list_timeline_entries", { sessionId }),
      invokeTauri<EvidenceItem[]>("list_evidence_items", { sessionId }),
      invokeTauri<EnvironmentSnapshot | null>("latest_environment", { sessionId }),
    ]);
    setRepository(nextRepository);
    setShadow(nextShadow);
    setSteps(nextSteps);
    setRuns(nextRuns);
    setTimeline(nextTimeline);
    setEvidence(nextEvidence);
    setEnvironment(nextEnvironment);
  }, []);

  const loadSessions = useCallback(async (preferred?: string | null) => {
    if (!hasTauriRuntime()) {
      setRuntime("browser");
      setLoading(false);
      return [] as Session[];
    }
    try {
      await invokeTauri("runtime_health");
      setRuntime("tauri");
      const next = await invokeTauri<Session[]>("list_sessions");
      setSessions(next ?? []);
      const target = preferred ?? selectedSessionId;
      if (target && next.some(session => session.id === target)) await loadSessionData(target);
      return next;
    } catch (nextError) {
      setError(bridgeMessage(nextError));
      return [] as Session[];
    } finally {
      setLoading(false);
    }
  }, [loadSessionData, selectedSessionId]);

  useEffect(() => {
    let cancelled = false;
    async function initialize() {
      if (!hasTauriRuntime()) { setRuntime("browser"); setLoading(false); return; }
      try {
        await invokeTauri("runtime_health");
        if (cancelled) return;
        setRuntime("tauri");
        const nextSessions = await invokeTauri<Session[]>("list_sessions");
        if (cancelled) return;
        setSessions(nextSessions ?? []);
        const saved = settings.workspace;
        if (settings.behavior.restore_last_workspace && saved.kind === "session" && saved.session_id && nextSessions.some(item => item.id === saved.session_id)) {
          setSelectedSessionId(saved.session_id);
          setTabState(saved.session_tab);
          await loadSessionData(saved.session_id);
        } else if ((settings.behavior.restore_last_workspace && saved.kind === "project" || settings.behavior.restore_last_project) && saved.project_path) {
          const profile = await invokeTauri<ProjectProfile>("analyze_project", { path: saved.project_path });
          if (!cancelled) {
            setSelectedProject(profile);
            setProjectTabState(settings.behavior.restore_last_workspace ? saved.project_tab : "project-overview");
          }
        } else if (settings.behavior.restore_last_workspace) {
          setRootView(saved.root_view);
        }
      } catch (nextError) {
        if (!cancelled) setError(bridgeMessage(nextError));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void initialize();
    return () => { cancelled = true; };
  }, []); // Rehydrate the persisted desktop workspace only once.

  const clearWorkspace = useCallback((view: RootView) => {
    setSelectedSessionId(null);
    setSelectedProject(null);
    setSelectedActionId(null);
    setRepository(null);
    setShadow(null);
    setSteps([]);
    setRuns([]);
    setTimeline([]);
    setEvidence([]);
    setEnvironment(null);
    setInvestigationSeed("");
    setRootView(view);
    updateWorkspaceSetting({ ...settings.workspace, kind: "root", root_view: view, investigation_case_id: null });
  }, [settings.workspace, updateWorkspaceSetting]);

  const openSession = useCallback(async (id: string) => {
    setSelectedProject(null);
    setSelectedSessionId(id);
    setRootView("home");
    setTabState("overview");
    setSelectedActionId(null);
    setError(null);
    updateWorkspaceSetting({ ...settings.workspace, kind: "session", session_id: id, session_tab: "overview", investigation_case_id: null });
    try { await loadSessionData(id); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
  }, [loadSessionData, settings.workspace, updateWorkspaceSetting]);

  const openProject = useCallback((profile: ProjectProfile) => {
    setSelectedSessionId(null);
    setSelectedProject(profile);
    setSelectedActionId(null);
    setRepository(null);
    setShadow(null);
    setSteps([]);
    setRuns([]);
    setTimeline([]);
    setEvidence([]);
    setEnvironment(null);
    setRootView("home");
    setProjectTabState("project-overview");
    setInvestigationSeed("");
    setError(null);
    updateWorkspaceSetting({ ...settings.workspace, kind: "project", project_path: profile.root_path, project_tab: "project-overview", investigation_case_id: null });
  }, [settings.workspace, updateWorkspaceSetting]);

  const openProjectDialog = useCallback(async () => {
    const path = await chooseRepositoryDirectory(t("dialog.chooseProject"));
    if (!path) return;
    const profile = await invokeTauri<ProjectProfile>("analyze_project", { path });
    openProject(profile);
  }, [openProject, t]);

  const refreshProject = useCallback(async () => {
    if (!selectedProject) return;
    const profile = await invokeTauri<ProjectProfile>("analyze_project", { path: selectedProject.root_path });
    setSelectedProject(profile);
  }, [selectedProject]);

  const investigateProject = useCallback((query: string) => {
    setInvestigationSeed(query);
    setProjectTabState("agent");
    if (selectedProject) updateWorkspaceSetting({ ...settings.workspace, kind: "project", project_path: selectedProject.root_path, project_tab: "agent" });
  }, [selectedProject, settings.workspace, updateWorkspaceSetting]);

  const reloadCurrent = useCallback(async () => {
    if (!selectedSessionId) return;
    try {
      const nextSessions = await invokeTauri<Session[]>("list_sessions");
      setSessions(nextSessions ?? []);
      await loadSessionData(selectedSessionId);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
  }, [loadSessionData, selectedSessionId]);

  function setSessionTab(next: WorkspaceTab) {
    setTabState(next);
    if (selectedSessionId) updateWorkspaceSetting({ ...settings.workspace, kind: "session", session_id: selectedSessionId, session_tab: next });
  }

  function setProjectTab(next: ProjectTab) {
    setProjectTabState(next);
    if (selectedProject) updateWorkspaceSetting({ ...settings.workspace, kind: "project", project_path: selectedProject.root_path, project_tab: next });
  }

  function prepareVerificationSession(value: InvestigationCase) {
    const supported = value.hypotheses.find(item => item.status === "Supported");
    const experiment = [...value.experiments].reverse().find(item => item.hypothesis_id === supported?.id && item.conclusion === "SupportsHypothesis");
    if (!supported || !experiment || !selectedProject) {
      setError(t("investigation.prepareVerificationBlocked"));
      return;
    }
    const regressions = regressionRecommendations(selectedProject, experiment.changed_files, value.criterion.command_id)
      .filter(item => item.command.id !== value.criterion.command_id)
      .map(item => ({
        stable_id: item.command.id,
        title: item.command.label,
        executable: item.command.executable,
        args: item.command.args,
        expected_exit_code: 0,
        level: item.tier === "required" ? "Required" as const : item.tier === "recommended" ? "Recommended" as const : "Optional" as const,
      }));
    setNewSessionDraft({
      path: value.repo_root,
      title: `${t("investigation.verifyPrefix")}: ${value.cluster.title}`,
      expected: t("investigation.verificationExpected").replace("{exit}", String(value.criterion.expected_exit_code)),
      actual: value.criterion.baseline_summary || value.cluster.summary,
      notes: [
        `${t("investigation.sourceCase")}: ${value.id}`,
        `${t("investigation.failureSignature")}: ${value.cluster.signature}`,
        `${t("investigation.supportedRootCause")}: ${supported?.statement ?? "—"}`,
        `${t("investigation.evidence")}: ${value.evidence_ids.join(", ")}`,
      ].join("\n"),
      executable: value.criterion.executable,
      args: formatArguments(value.criterion.args),
      expectedExitCode: value.criterion.expected_exit_code,
      handoff: {
        caseId: value.id,
        hypothesisId: supported.id,
        experimentId: experiment.id,
        regressions,
      },
    });
    setNewSessionOpen(true);
  }

  function toggleSidebar() {
    const compact = !sidebarCompact;
    setSidebarCompact(compact);
    void onSettings({ ...settings, ui: { ...settings.ui, sidebar_mode: compact ? "compact" : "expanded" } }).catch(nextError => setError(bridgeMessage(nextError)));
  }

  function toggleInspector(next = !inspectorOpen) {
    setInspectorOpen(next);
    if (settings.ui.remember_inspector_state) void onSettings({ ...settings, ui: { ...settings.ui, inspector_open: next } }).catch(nextError => setError(bridgeMessage(nextError)));
  }

  function commitSidebarWidth(value: number) {
    if (settings.ui.remember_sidebar_width) void onSettings({ ...settings, ui: { ...settings.ui, sidebar_width: value } }).catch(nextError => setError(bridgeMessage(nextError)));
  }

  function commitInspectorWidth(value: number) {
    if (settings.ui.remember_inspector_width) void onSettings({ ...settings, ui: { ...settings.ui, inspector_width: value } }).catch(nextError => setError(bridgeMessage(nextError)));
  }

  const inspectorAvailable = Boolean(
    (selectedSession && tab === "timeline" && selectedEntry)
    || (selectedProject && projectTab === "checks" && settings.workspace.investigation_case_id),
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const shortcut = globalShortcut(event);
      if (!shortcut) return;
      event.preventDefault();
      if (shortcut === "new-session" && runtime === "tauri") { setNewSessionDraft(null); setNewSessionOpen(true); }
      else if (shortcut === "open-project" && runtime === "tauri") void openProjectDialog().catch(nextError => setError(bridgeMessage(nextError)));
      else if (shortcut === "command-palette") setPaletteOpen(true);
      else if (shortcut === "toggle-sidebar" && (selectedProject || selectedSession)) toggleSidebar();
      else if (shortcut === "toggle-inspector" && inspectorAvailable) toggleInspector();
      else if (shortcut === "settings") clearWorkspace("settings");
      else if (shortcut === "escape") setPaletteOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [runtime, sidebarCompact, inspectorOpen, inspectorAvailable, settings, openProjectDialog, clearWorkspace, selectedProject, selectedSession]);

  function renderView() {
    if (selectedProject) {
      return <ProjectWorkspace profile={selectedProject} tab={projectTab} settings={settings} investigationSeed={investigationSeed} onRefresh={refreshProject} onInvestigate={investigateProject} inspectorOpen={inspectorOpen} inspectorWidth={inspectorWidth} preferredInvestigationCaseId={settings.workspace.investigation_case_id} onInvestigationCaseChange={caseId => updateWorkspaceSetting({ ...settings.workspace, investigation_case_id: caseId })} onInspectorOpen={toggleInspector} onInspectorWidth={setInspectorWidth} onInspectorWidthCommit={commitInspectorWidth} onPrepareVerification={prepareVerificationSession}/>;
    }
    if (!selectedSession) {
      switch (rootView) {
        case "sessions": return <SessionsView sessions={sessions} onNew={() => { if (runtime === "tauri") { setNewSessionDraft(null); setNewSessionOpen(true); } }} onOpen={id => void openSession(id)} />;
        case "projects": return <ProjectsView onOpenProject={openProject}/>;
        case "capsules": return <CapsulesView />;
        case "settings": return <SettingsView settings={settings} onSave={onSettings} />;
        default: return <HomeView sessions={sessions} onOpenProject={openProject} onOpenSession={id => void openSession(id)} onImportCapsule={() => clearWorkspace("capsules")} />;
      }
    }
    switch (tab) {
      case "overview": return <OverviewView session={selectedSession} repository={repository} shadow={shadow} steps={steps} runs={runs} environment={environment} onReload={reloadCurrent} onGoChanges={() => setSessionTab("changes")} />;
      case "timeline": return <TimelineView entries={timeline} selectedId={selectedActionId} onSelect={id => { setSelectedActionId(id); toggleInspector(true); }} />;
      case "evidence": return <EvidenceView items={evidence} entries={timeline} />;
      case "changes": return <ChangesView sessionId={selectedSession.id} shadow={shadow} onReload={reloadCurrent} />;
      case "environment": return <EnvironmentView environment={environment} />;
      case "verification": return selectedSessionId ? <VerificationView sessionId={selectedSessionId} steps={steps} runs={runs} onReload={reloadCurrent} /> : null;
    }
  }

  const paletteItems = useMemo<PaletteItem[]>(() => {
    const items: PaletteItem[] = [
      { id: "open-project", label: t("palette.openProject"), detail: t("nav.projects"), shortcut: "Ctrl O", action: openProjectDialog },
      { id: "new-session", label: t("nav.newSession"), detail: t("nav.sessions"), shortcut: "Ctrl N", action: () => { if (runtime === "tauri") { setNewSessionDraft(null); setNewSessionOpen(true); } } },
      { id: "home", label: t("nav.home"), action: () => clearWorkspace("home") },
      { id: "projects", label: t("nav.projects"), action: () => clearWorkspace("projects") },
      { id: "sessions", label: t("nav.sessions"), action: () => clearWorkspace("sessions") },
      { id: "capsules", label: t("nav.capsules"), action: () => clearWorkspace("capsules") },
      { id: "settings", label: t("nav.settings"), shortcut: "Ctrl ,", action: () => clearWorkspace("settings") },
      { id: "theme", label: t("palette.cycleTheme"), detail: translatedValue(t, "settings", settings.theme), action: () => { const themes = ["system", "dark", "light"] as const; const theme = themes[(themes.indexOf(settings.theme) + 1) % themes.length]; void onSettings({ ...settings, theme }); } },
    ];
    if (selectedProject || selectedSession) items.splice(7, 0, { id: "sidebar", label: sidebarCompact ? t("nav.expandSidebar") : t("nav.collapseSidebar"), shortcut: "Ctrl B", action: toggleSidebar });
    if (inspectorAvailable) items.splice(8, 0, { id: "inspector", label: inspectorOpen ? t("palette.closeInspector") : t("palette.openInspector"), shortcut: "Ctrl Shift I", action: () => toggleInspector() });
    if (selectedProject) items.splice(2, 0,
      { id: "run-checks", label: t("palette.runChecks"), detail: selectedProject.name, action: () => { setProjectTab("checks"); window.setTimeout(() => window.dispatchEvent(new CustomEvent("reprodeck:run-checks")), 40); } },
      { id: "start-investigation", label: t("palette.startInvestigation"), detail: selectedProject.name, action: () => { setProjectTab("checks"); toggleInspector(true); window.setTimeout(() => window.dispatchEvent(new CustomEvent("reprodeck:start-investigation")), 40); } },
      { id: "open-agent", label: t("palette.openAgent"), detail: selectedProject.name, action: () => setProjectTab("agent") },
    );
    for (const session of sessions.slice(0, 10)) items.push({ id: `session-${session.id}`, label: sessionMeta(session).title || session.id, detail: t("nav.sessions"), action: () => openSession(session.id) });
    const allowed = new Set(commandPaletteCommandIds({ project: Boolean(selectedProject), session: Boolean(selectedSession), inspector: inspectorAvailable }));
    return items.filter(item => item.id.startsWith("session-") || allowed.has(item.id));
  }, [clearWorkspace, inspectorOpen, onSettings, openProjectDialog, openSession, projectTab, runtime, selectedEntry, selectedProject, selectedSession, sessions, settings, sidebarCompact, t, tab]);

  const filteredPaletteItems = useMemo(() => {
    const query = paletteQuery.trim().toLowerCase();
    return query ? paletteItems.filter(item => `${item.label} ${item.detail ?? ""}`.toLowerCase().includes(query)) : paletteItems;
  }, [paletteItems, paletteQuery]);

  useEffect(() => setPaletteIndex(0), [paletteQuery, paletteOpen]);

  function runPalette(item: PaletteItem | undefined) {
    if (!item) return;
    setPaletteOpen(false);
    setPaletteQuery("");
    void item.action();
  }

  const rootTitle = t(`nav.${rootView}`);
  const timelineInspectorVisible = tab === "timeline" && !!selectedSession && inspectorOpen && !!selectedEntry;
  const timelineInspectorPresence = usePresence(timelineInspectorVisible, 220);

  return <div className={`app-shell ${sidebarCompact ? "sidebar-compact" : ""}`}>
    <Sidebar
      sessions={sessions}
      selectedSessionId={selectedSessionId}
      selectedProject={selectedProject}
      tab={tab}
      projectTab={projectTab}
      rootView={rootView}
      compact={sidebarCompact}
      width={sidebarWidth}
      onSelectSession={id => void openSession(id)}
      onTab={setSessionTab}
      onProjectTab={setProjectTab}
      onNewSession={() => { if (runtime === "tauri") { setNewSessionDraft(null); setNewSessionOpen(true); } }}
      onRoot={clearWorkspace}
      onToggleCompact={toggleSidebar}
      onWidthChange={setSidebarWidth}
      onWidthCommit={commitSidebarWidth}
    />

    <div className="app-main">
      <header className="topbar">
        {(selectedProject || selectedSession) && <button className="topbar-icon" onClick={toggleSidebar} aria-label={sidebarCompact ? t("nav.expandSidebar") : t("nav.collapseSidebar")}><PanelIcon/></button>}
        <div className="breadcrumbs">
          {selectedProject ? <><button className="crumb-root" onClick={() => clearWorkspace("projects")}>{t("nav.projects")}</button><span>/</span><strong>{selectedProject.name}</strong>{selectedProject.git && <><span className="dot-separator">·</span><span className="repo-context">{selectedProject.git.branch}</span>{selectedProject.git.is_dirty && <span className="dirty-label">{t("top.localChanges")}</span>}</>}</>
          : selectedSession ? <><button className="crumb-root" onClick={() => clearWorkspace("home")}>{t("nav.home")}</button><span>/</span><strong>{meta.title || selectedSession.id}</strong>{repository && <><span className="dot-separator">·</span><span className="repo-context">{repoName(repository.path)} / {repository.branch}</span>{repository.is_dirty && <span className="dirty-label">{t("top.localChanges")}</span>}</>}</>
          : <strong>{rootTitle}</strong>}
        </div>
        {inspectorAvailable && <button className={`topbar-icon inspector-toggle ${inspectorOpen ? "active" : ""}`} onClick={() => toggleInspector()} aria-label={inspectorOpen ? t("palette.closeInspector") : t("palette.openInspector")}><InspectorIcon/></button>}
        <button className="search-button" onClick={() => setPaletteOpen(true)}><SearchIcon/><span>{t("top.search")}</span><kbd>Ctrl K</kbd></button>
      </header>

      {runtime === "browser" && <div className="runtime-banner"><strong>{t("runtime.previewTitle")}</strong><span>{t("runtime.previewText")}</span></div>}
      {error && <div className="error-banner" role="alert"><span>{error}</span><button onClick={() => setError(null)}>{t("common.dismiss")}</button></div>}

      <div className={`content-layout ${timelineInspectorPresence.mounted ? "with-inspector" : ""}`} style={{ "--inspector-width": `${inspectorWidth}px` } as CSSProperties}>
        <main className="content-area">{loading ? <div className="loading-state"><span className="loading-spinner"/>{t("common.loading")}</div> : renderView()}</main>
        {timelineInspectorPresence.mounted && <div className={`inspector-pane presence-${timelineInspectorPresence.phase}`}><ResizeHandle side="left" value={inspectorWidth} min={360} max={760} label={t("settings.inspectorWidth")} onChange={setInspectorWidth} onCommit={commitInspectorWidth}/><Inspector entry={selectedEntry} onClose={() => toggleInspector(false)} /></div>}
      </div>

      <footer className="statusbar">
        <div><span className={`connection-dot ${runtime}`}/>{runtime === "tauri" ? t("top.bridge") : runtime === "browser" ? t("top.preview") : t("common.loading")}</div>
        <div>{selectedProject ? `${selectedProject.name}${selectedProject.git ? ` · ${selectedProject.git.branch} @ ${selectedProject.git.head_commit?.slice(0, 8) ?? "—"}` : ""}` : selectedSession ? selectedSession.id : `${sessions.length} ${t("nav.sessions").toLowerCase()}`}{repository ? ` · ${repository.branch} @ ${repository.head_commit.slice(0, 8)}` : ""}</div>
        <div>{t("top.localOnly")}</div>
      </footer>
    </div>

    {newSessionPresence.mounted && runtime === "tauri" && <NewSessionWizard initialPath={selectedProject?.root_path} initialDraft={newSessionDraft} presenceClass={newSessionPresence.phase} onClose={() => setNewSessionOpen(false)} onComplete={async id => { setNewSessionOpen(false); await loadSessions(id); await openSession(id); setNewSessionDraft(null); }} />}

    {palettePresence.mounted && <div className={`palette-layer presence-${palettePresence.phase}`} role="presentation" onMouseDown={() => setPaletteOpen(false)}><section className="command-palette" role="dialog" aria-modal="true" aria-label={t("top.search")} onMouseDown={event => event.stopPropagation()}>
      <header><SearchIcon/><input autoFocus value={paletteQuery} onChange={event => setPaletteQuery(event.target.value)} onKeyDown={event => {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); setPaletteIndex(current => Math.max(0, Math.min(filteredPaletteItems.length - 1, current + (event.key === "ArrowDown" ? 1 : -1)))); }
        else if (event.key === "Enter") { event.preventDefault(); runPalette(filteredPaletteItems[paletteIndex]); }
        else if (event.key === "Escape") { event.preventDefault(); setPaletteOpen(false); }
      }} placeholder={t("top.search")} aria-controls="command-results" aria-activedescendant={filteredPaletteItems[paletteIndex]?.id ? `palette-${filteredPaletteItems[paletteIndex].id}` : undefined}/></header>
      <div className="palette-results" id="command-results" role="listbox">{filteredPaletteItems.map((item, index) => <button id={`palette-${item.id}`} role="option" aria-selected={index === paletteIndex} className={index === paletteIndex ? "active" : ""} key={item.id} onMouseEnter={() => setPaletteIndex(index)} onClick={() => runPalette(item)}><span><strong>{item.label}</strong>{item.detail && <small>{item.detail}</small>}</span>{item.shortcut && <kbd>{item.shortcut}</kbd>}</button>)}{filteredPaletteItems.length === 0 && <p className="palette-empty">{t("palette.noResults")}</p>}</div>
      <footer><span>↑↓ {t("palette.navigate")}</span><span>Enter {t("palette.open")}</span><span>Esc {t("common.close")}</span></footer>
    </section></div>}

    <div className="toast-region" aria-live="polite" aria-atomic="false">{toasts.map(item => <div key={item.id} className={`toast toast-${item.tone} ${item.closing ? "presence-closing" : "presence-open"}`}><i/><div><strong>{item.title}</strong>{item.message && <p>{item.message}</p>}</div><button onClick={() => dismissToast(item.id)} aria-label={t("common.close")}>×</button></div>)}</div>
  </div>;
}

export default function App() {
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [settingsReady, setSettingsReady] = useState(!hasTauriRuntime());

  useEffect(() => {
    applyPreferences(settings);
    document.documentElement.lang = settings.language;
  }, [settings]);

  useEffect(() => {
    if (!hasTauriRuntime()) return;
    let cancelled = false;
    void invokeTauri<AppSettings>("load_settings")
      .then(value => { if (!cancelled) setSettings(migrateAppSettings(value)); })
      .catch(() => { /* defaults remain usable when settings storage is unavailable */ })
      .finally(() => { if (!cancelled) setSettingsReady(true); });
    return () => { cancelled = true; };
  }, []);

  async function saveSettings(value: AppSettings) {
    const saved = hasTauriRuntime() ? await invokeTauri<AppSettings>("save_settings", { value }) : value;
    setSettings(saved);
  }

  if (!settingsReady) return <div className="boot-screen"><span className="loading-spinner"/>ReproDeck</div>;
  return <I18nProvider language={settings.language}><ReproDeckApp settings={settings} onSettings={saveSettings}/></I18nProvider>;
}
