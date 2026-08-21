import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { translatedValue, useI18n } from "../i18n";
import { bugHunterRunOrder, formatContextPacket, formatProjectCommand, healthCounts, runnableProjectCommands } from "../lib/project";
import { bridgeMessage, confirmAction, invokeTauri, revealLocalPath } from "../lib/tauri";
import { usePresence } from "../lib/usePresence";
import type { AppSettings, BugHunterAnalysis, BugHunterPlan, ContextPacket, InvestigationCase, ProjectHealthReport, ProjectProblemRecord, ProjectProfile, ProjectTab } from "../types";
import { CheckIcon, RepoIcon, SearchIcon, WarningIcon } from "./Icons";
import { InvestigationCasePanel } from "./InvestigationCasePanel";
import { ResizeHandle, Spinner } from "./ui";

type Investigation = { analysis: string; context: ContextPacket };
type Props = {
  profile: ProjectProfile;
  tab: ProjectTab;
  settings: AppSettings;
  investigationSeed: string;
  onRefresh: () => Promise<void>;
  onInvestigate: (query: string) => void;
  inspectorOpen: boolean;
  inspectorWidth: number;
  preferredInvestigationCaseId: string | null;
  onInvestigationCaseChange: (caseId: string) => void;
  onInspectorOpen: (open: boolean) => void;
  onInspectorWidth: (width: number) => void;
  onInspectorWidthCommit: (width: number) => void;
  onPrepareVerification: (value: InvestigationCase) => void;
};

function localizedSignal(t: (key: string) => string, profile: ProjectProfile, signal: ProjectProfile["signals"][number]) {
  const titleKey = `signalTitle.${signal.id}`;
  const detailKey = `signalDetail.${signal.id}`;
  const rawTitle = t(titleKey);
  let detail = t(detailKey);
  const count = signal.id === "git-dirty" ? profile.git?.changed_files.length ?? 0
    : signal.id === "maintenance-markers" ? profile.stats.todo_markers
    : signal.id === "sensitive-excluded" ? profile.stats.sensitive_files_excluded
    : 0;
  detail = detail.replace("{count}", String(count));
  return { title: rawTitle === titleKey ? signal.title : rawTitle, detail: detail === detailKey ? signal.detail : detail };
}

function ProjectOverview({ profile, onRefresh }: { profile: ProjectProfile; onRefresh: () => Promise<void> }) {
  const { t } = useI18n();
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function refresh() {
    setRefreshing(true); setError(null);
    try { await onRefresh(); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setRefreshing(false); }
  }
  return <section className="view-page project-view">
    <header className="project-identity"><div className="project-title-block"><span className="project-kicker">{t("project.passport")}</span><h1>{profile.name}{profile.version && <small>v{profile.version}</small>}</h1><p>{profile.description || t("project.noDescription")}</p></div><div className="project-actions"><button className="button" onClick={() => void revealLocalPath(profile.root_path).catch(nextError => setError(bridgeMessage(nextError)))}><RepoIcon/>{t("common.explorer")}</button><button className="button" disabled={refreshing} onClick={() => void refresh()}>{refreshing ? t("projects.analyzing") : t("project.rescan")}</button></div></header>

    <div className="project-factbar">
      <span><small>{t("project.files")}</small><strong>{profile.stats.files_seen.toLocaleString()}</strong></span>
      <span><small>{t("project.sourceFiles")}</small><strong>{profile.stats.source_files.toLocaleString()}</strong></span>
      <span><small>{t("project.tests")}</small><strong>{profile.stats.test_files.toLocaleString()}</strong></span>
      <span><small>{t("project.commands")}</small><strong>{profile.commands.length}</strong></span>
      <span><small>{t("project.signals")}</small><strong>{profile.signals.length}</strong></span>
    </div>

    <div className="project-columns">
      <section className="project-section"><header><h2>{t("project.stack")}</h2></header><div className="technology-table">{profile.technologies.length ? profile.technologies.map(item => <div key={`${item.category}:${item.name}`}><span>{item.category}</span><strong>{item.name}</strong><small>{item.evidence[0]}</small></div>) : <p className="muted-copy">{t("project.stackUnknown")}</p>}</div></section>
      <section className="project-section"><header><h2>{t("project.languages")}</h2></header><div className="language-list">{profile.languages.length ? profile.languages.slice(0, 10).map(item => <div key={item.language}><strong>{item.language}</strong><span>{item.files}</span></div>) : <p className="muted-copy">—</p>}</div></section>
      <section className="project-section project-git"><header><h2>{t("project.git")}</h2></header>{profile.git ? <dl><div><dt>{t("overview.branch")}</dt><dd>{profile.git.branch}</dd></div><div><dt>HEAD</dt><dd><code>{profile.git.head_commit?.slice(0, 12) ?? "—"}</code></dd></div><div><dt>{t("project.workingTree")}</dt><dd className={profile.git.is_dirty ? "warning-text" : "success-text"}>{profile.git.is_dirty ? t("project.dirty") : t("common.clean")}</dd></div></dl> : <p className="muted-copy">{t("project.notGit")}</p>}</section>
      <section className="project-section"><header><h2>{t("project.entrypoints")}</h2></header>{profile.entrypoints.length ? <div className="path-list">{profile.entrypoints.map(path => <code key={path}>{path}</code>)}</div> : <p className="muted-copy">{t("project.noEntrypoints")}</p>}</section>
    </div>
    {profile.stats.sensitive_files_excluded > 0 && <div className="privacy-note"><strong>{t("project.privacyProtected")}</strong><span>{t("project.privacyProtectedHelp").replace("{count}", String(profile.stats.sensitive_files_excluded))}</span></div>}
    {error && <div className="inline-error page-message">{error}</div>}
  </section>;
}

function Problems({ profile, onInvestigate }: { profile: ProjectProfile; onInvestigate: (query: string) => void }) {
  const { t } = useI18n();
  const [confirmed, setConfirmed] = useState<ProjectProblemRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const severityOrder = { Warning: 0, Review: 1, Info: 2 } as const;
  const ordered = useMemo(() => [...profile.signals].sort((a, b) => severityOrder[a.severity] - severityOrder[b.severity]), [profile.signals]);
  const activeProblems = useMemo(() => confirmed.filter(problem => problem.active), [confirmed]);
  const clearedProblems = useMemo(() => confirmed.filter(problem => !problem.active), [confirmed]);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    invokeTauri<ProjectProblemRecord[]>("list_project_problems", { path: profile.root_path })
      .then(items => { if (active) setConfirmed(items); })
      .catch(nextError => { if (active) setError(bridgeMessage(nextError)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [profile.root_path]);

  return <section className="view-page project-view">
    <header className="view-heading"><div><h1>{t("problems.title")}</h1><p>{t("problems.description")}</p></div><span className="evidence-rule">{t("problems.rule")}</span></header>

    <section className="problem-section">
      <header className="section-heading compact"><div><h2>{t("problems.confirmedTitle")}</h2><p>{t("problems.confirmedHelp")}</p></div><span className="count-badge">{activeProblems.length}</span></header>
      {loading ? <div className="quiet-empty compact"><p>{t("common.loading")}</p></div> : activeProblems.length === 0 ? <div className="quiet-empty compact"><h3>{t("problems.noConfirmed")}</h3><p>{t("problems.noConfirmedHelp")}</p></div> : <div className="problem-list confirmed-problems">{activeProblems.map(problem => <article key={problem.id} className="problem-row confirmed-problem">
        <div className="problem-mark"><WarningIcon/></div>
        <div className="problem-copy"><div><span className="confirmed-label">{t("problems.reproduced")}</span><code>{problem.command_id}</code>{problem.occurrences > 1 && <small>{t("problems.seenTimes").replace("{count}", String(problem.occurrences))}</small>}</div><h3>{problem.title}</h3><p>{problem.summary}</p><details><summary>{t("problems.evidence")} · {problem.evidence_ids.length}</summary>{problem.evidence_ids.map(item => <code key={item}>{item}</code>)}</details></div>
        <button className="button small" onClick={() => onInvestigate(`${problem.title}. ${problem.summary}. Evidence: ${problem.evidence_ids.join(", ")}`)}>{t("problems.investigate")}</button>
      </article>)}</div>}
    </section>

    {clearedProblems.length > 0 && <section className="problem-section cleared-problems-section">
      <header className="section-heading compact"><div><h2>{t("problems.clearedTitle")}</h2><p>{t("problems.clearedHelp")}</p></div><span className="count-badge">{clearedProblems.length}</span></header>
      <div className="problem-list cleared-problems">{clearedProblems.map(problem => <article key={problem.id} className="problem-row cleared-problem">
        <div className="problem-mark"><CheckIcon/></div>
        <div className="problem-copy"><div><span className="cleared-label">{t("problems.cleared")}</span><code>{problem.command_id}</code><small>{problem.cleared_at ? new Date(problem.cleared_at * 1000).toLocaleString() : ""}</small></div><h3>{problem.title}</h3><p>{problem.summary}</p><small>{t("problems.clearedNotVerified")}</small></div>
      </article>)}</div>
    </section>}

    <section className="problem-section">
      <header className="section-heading compact"><div><h2>{t("problems.signalsTitle")}</h2><p>{t("problems.signalsHelp")}</p></div><span className="count-badge">{ordered.length}</span></header>
      {ordered.length === 0 ? <div className="quiet-empty compact"><h3>{t("problems.none")}</h3><p>{t("problems.noneHelp")}</p></div> : <div className="problem-list">{ordered.map(signal => { const copy = localizedSignal(t, profile, signal); return <article key={signal.id} className={`problem-row severity-${signal.severity.toLowerCase()}`}>
        <div className="problem-mark">{signal.severity === "Warning" ? <WarningIcon/> : signal.severity === "Review" ? <SearchIcon/> : <CheckIcon/>}</div>
        <div className="problem-copy"><div><span>{translatedValue(t, "signal", signal.severity)}</span><code>{signal.id}</code></div><h3>{copy.title}</h3><p>{copy.detail}</p>{signal.evidence.length > 0 && <details><summary>{t("problems.evidence")} · {signal.evidence.length}</summary>{signal.evidence.map(item => <code key={item}>{item}</code>)}</details>}</div>
        <button className="button small" onClick={() => onInvestigate(`${copy.title}. ${copy.detail}`)}>{t("problems.investigate")}</button>
      </article>; })}</div>}
    </section>
    {error && <div className="inline-error page-message">{error}</div>}
  </section>;
}

function Checks({ profile, onInvestigate, settings, inspectorOpen, inspectorWidth, preferredInvestigationCaseId, onInvestigationCaseChange, onInspectorOpen, onInspectorWidth, onInspectorWidthCommit, onPrepareVerification }: { profile: ProjectProfile; onInvestigate: (query: string) => void; settings: AppSettings; inspectorOpen: boolean; inspectorWidth: number; preferredInvestigationCaseId: string | null; onInvestigationCaseChange: (caseId: string) => void; onInspectorOpen: (open: boolean) => void; onInspectorWidth: (width: number) => void; onInspectorWidthCommit: (width: number) => void; onPrepareVerification: (value: InvestigationCase) => void }) {
  const { t } = useI18n();
  const runnable = useMemo(() => runnableProjectCommands(profile.commands), [profile.commands]);
  const [selected, setSelected] = useState<string[]>([]);
  const [plan, setPlan] = useState<BugHunterPlan | null>(null);
  const [report, setReport] = useState<ProjectHealthReport | null>(null);
  const [analysis, setAnalysis] = useState<BugHunterAnalysis | null>(null);
  const [cases, setCases] = useState<InvestigationCase[]>([]);
  const [activeCase, setActiveCase] = useState<InvestigationCase | null>(null);
  const [pendingCluster, setPendingCluster] = useState<BugHunterAnalysis["clusters"][number] | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyQuery, setHistoryQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [loadingPlan, setLoadingPlan] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const resultAnchor = useRef<HTMLElement>(null);
  const checksPane = useRef<HTMLElement>(null);
  const investigationPresence = usePresence(Boolean(activeCase || pendingCluster) && inspectorOpen, 220);
  const historyPresence = usePresence(historyOpen, 200);

  useEffect(() => {
    let active = true;
    setLoadingPlan(true);
    setError(null);
    Promise.all([
      invokeTauri<BugHunterPlan>("build_bug_hunter_plan", { path: profile.root_path }),
      invokeTauri<ProjectHealthReport | null>("latest_project_health", { path: profile.root_path }),
      invokeTauri<BugHunterAnalysis | null>("analyze_bug_hunter_failures", { path: profile.root_path }),
      invokeTauri<InvestigationCase[]>("list_investigation_cases", { path: profile.root_path }),
    ])
      .then(([nextPlan, nextReport, nextAnalysis, nextCases]) => {
        if (!active) return;
        setPlan(nextPlan);
        setSelected(nextPlan.steps.map(step => step.command_id));
        setReport(nextReport);
        setAnalysis(nextAnalysis);
        setCases(nextCases);
        setActiveCase(current => {
          const restored = current
            ? nextCases.find(item => item.id === current.id)
            : nextCases.find(item => item.id === preferredInvestigationCaseId);
          return restored ?? null;
        });
      })
      .catch(nextError => { if (active) setError(bridgeMessage(nextError)); })
      .finally(() => { if (active) setLoadingPlan(false); });
    return () => { active = false; };
  }, [profile.root_path, profile.analyzed_at, preferredInvestigationCaseId]);

  function toggle(id: string) {
    setSelected(current => current.includes(id) ? current.filter(value => value !== id) : [...current, id]);
  }

  async function runHealth() {
    if (!profile.git) { setError(t("checks.gitRequired")); return; }
    if (selected.length === 0) { setError(t("checks.selectOne")); return; }
    if (!await confirmAction(t("confirm.projectHealth"))) return;
    setBusy(true); setError(null);
    try {
      const next = await invokeTauri<ProjectHealthReport>("run_project_health", { path: profile.root_path, commandIds: bugHunterRunOrder(selected, plan), timeoutSecs: 180, confirmedExecution: true });
      setReport(next);
      const nextAnalysis = await invokeTauri<BugHunterAnalysis | null>("analyze_bug_hunter_failures", { path: profile.root_path });
      setAnalysis(nextAnalysis);
      let nextCases = await invokeTauri<InvestigationCase[]>("list_investigation_cases", { path: profile.root_path });
      setCases(nextCases);
      const firstCluster = nextAnalysis?.clusters[0];
      if (firstCluster && settings.behavior.auto_open_investigation) {
        setActiveCase(null);
        setPendingCluster(firstCluster);
        onInspectorOpen(true);
      }
      if (settings.behavior.auto_scroll_logs) window.requestAnimationFrame(() => resultAnchor.current?.scrollIntoView({ block: "start", behavior: settings.ui.reduced_motion ? "auto" : "smooth" }));
      window.dispatchEvent(new CustomEvent("reprodeck:notify", { detail: nextAnalysis?.clusters.length
        ? { tone: "danger", title: t("checks.failureFound"), message: nextAnalysis.clusters[0]?.summary }
        : { tone: "success", title: t("checks.runPassed"), message: t("checks.originalUntouched") } }));
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(false); }
  }

  async function openInvestigation(clusterId: string) {
    setBusy(true); setError(null);
    try {
      const created = await invokeTauri<InvestigationCase>("create_investigation_case", { path: profile.root_path, clusterId });
      setActiveCase(created);
      setPendingCluster(null);
      onInvestigationCaseChange(created.id);
      onInspectorOpen(true);
      setCases(current => [created, ...current.filter(item => item.id !== created.id)]);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(false); }
  }

  function updateCase(next: InvestigationCase) {
    setActiveCase(next);
    onInvestigationCaseChange(next.id);
    setCases(current => [next, ...current.filter(item => item.id !== next.id)]);
  }

  const resultByCommand = useMemo(() => new Map(report?.checks.map(check => [check.command_id, check]) ?? []), [report]);
  const counts = useMemo(() => healthCounts(report), [report]);
  const planByCommand = useMemo(() => new Map(plan?.steps.map(step => [step.command_id, step]) ?? []), [plan]);
  const selectableIds = useMemo(() => new Set([...runnable.map(command => command.id), ...(plan?.steps.map(step => step.command_id) ?? [])]), [runnable, plan]);
  const latestCaseByCluster = useMemo(() => new Map(cases.filter(item => item.health_run_id === report?.id).map(item => [item.cluster.id, item])), [cases, report?.id]);
  const primaryFailure = useMemo(() => report?.checks.find(check => ["Failed", "Error", "TimedOut"].includes(check.status)) ?? null, [report]);


  const filteredCases = useMemo(() => {
    const query = historyQuery.trim().toLowerCase();
    return query ? cases.filter(item => `${item.cluster.title} ${item.cluster.signature} ${item.state}`.toLowerCase().includes(query)) : cases;
  }, [cases, historyQuery]);

  function showCase(item: InvestigationCase) {
    setActiveCase(item);
    setPendingCluster(null);
    onInvestigationCaseChange(item.id);
    onInspectorOpen(true);
    setHistoryOpen(false);
  }

  useEffect(() => {
    if (!loadingPlan) window.requestAnimationFrame(() => checksPane.current?.scrollTo({ top: 0 }));
  }, [loadingPlan, profile.root_path]);

  useEffect(() => {
    const runChecks = () => { if (!busy && selected.length > 0 && profile.git) void runHealth(); };
    const startInvestigation = () => {
      const existing = activeCase ?? cases[0];
      if (existing) { showCase(existing); return; }
      const cluster = analysis?.clusters[0];
      if (cluster) void openInvestigation(cluster.id);
    };
    window.addEventListener("reprodeck:run-checks", runChecks);
    window.addEventListener("reprodeck:start-investigation", startInvestigation);
    return () => {
      window.removeEventListener("reprodeck:run-checks", runChecks);
      window.removeEventListener("reprodeck:start-investigation", startInvestigation);
    };
  });

  return <section className="project-view checks-workbench-view">
    <header className="workbench-page-header">
      <div className="workbench-title-group">
        <span className="workbench-eyebrow">BUG HUNTER</span>
        <h1>{t("checks.title")}</h1>
        <p>{t("checks.description")}</p>
      </div>
      <div className="workbench-run-actions">
        {report && <div className={`run-state run-state-${report.status.toLowerCase()}`}><span/>{translatedValue(t, "health", report.status)}</div>}
        <button className="button primary run-main-action" disabled={busy || selected.length === 0 || !profile.git} onClick={() => void runHealth()}>{busy && <Spinner label={t("checks.running")}/>} {busy ? t("checks.running") : t("checks.runSelected")}</button>
      </div>
    </header>

    <div className="investigation-state-summary" aria-label={t("checks.title")}>
      <span><small>{t("investigation.observed")}</small><strong>{report ? translatedValue(t, "health", report.status) : t("investigation.pending")}</strong></span>
      <span><small>{t("investigation.evidence")}</small><strong>{activeCase?.evidence_ids.length ?? analysis?.clusters[0]?.evidence_ids.length ?? 0}</strong></span>
      <span><small>{t("investigation.hypotheses")}</small><strong>{activeCase?.hypotheses.length ?? 0}</strong></span>
      <span><small>{t("investigation.experiment")}</small><strong>{activeCase?.experiments.length ? translatedValue(t, "experimentConclusion", activeCase.experiments[activeCase.experiments.length - 1].conclusion) : t("investigation.pending")}</strong></span>
    </div>

    <div className={`checks-workspace ${investigationPresence.mounted ? "with-case" : ""}`} style={{ "--case-inspector-width": `${inspectorWidth}px` } as CSSProperties}>
      <main className="checks-primary-pane" ref={checksPane}>
        {!profile.git && <div className="workbench-alert warning-note"><strong>{t("checks.gitRequiredTitle")}</strong><span>{t("checks.gitRequired")}</span></div>}
        {profile.git?.is_dirty && <div className="workbench-alert warning-note"><strong>{t("checks.dirtySnapshot")}</strong><span>{t("checks.dirtySnapshotHelp")}</span></div>}

        {report && <section className="run-overview-strip" ref={resultAnchor}>
          <div className="run-overview-heading"><span>{t("checks.latestRun")}</span><code>{report.base_commit.slice(0, 12)}</code><small>{new Date(report.finished_at * 1000).toLocaleString()}</small></div>
          <div className="run-overview-counts"><span><b>{counts.passed}</b>{t("checks.passed")}</span><span className={counts.failed ? "failed" : ""}><b>{counts.failed}</b>{t("checks.failed")}</span><span><b>{counts.incomplete}</b>{t("checks.incomplete")}</span></div>
          <div className={report.original_unchanged ? "repo-safe" : "repo-warning"}>{report.original_unchanged ? t("checks.originalUntouched") : t("checks.originalChanged")}</div>
        </section>}

        {primaryFailure && <section className="failure-focus" aria-labelledby="failure-focus-title">
          <header><div><span className="failure-indicator"/><div><small>{translatedValue(t, "failureClass", analysis?.clusters[0]?.class ?? "Unknown")}</small><h2 id="failure-focus-title">{t("checks.testFailed")}</h2></div></div><span>{primaryFailure.duration_ms} ms</span></header>
          <p>{primaryFailure.summary}</p>
          <dl><div><dt>{t("checks.command")}</dt><dd><code>{[primaryFailure.executable, ...primaryFailure.args].join(" ")}</code></dd></div><div><dt>{t("checks.exitCode")}</dt><dd>{primaryFailure.exit_code ?? "—"}</dd></div><div><dt>{t("checks.evidence")}</dt><dd><code>{primaryFailure.evidence_id}</code></dd></div></dl>
          {(primaryFailure.stderr_preview || primaryFailure.stdout_preview) && <details className="failure-output" open={settings.behavior.open_logs_on_failure}><summary>{t("checks.keyOutput")}</summary><pre>{(primaryFailure.stderr_preview || primaryFailure.stdout_preview).split(/\r?\n/).slice(0, 18).join("\n")}</pre></details>}
          {analysis?.clusters[0] && <button className="button primary" disabled={busy} onClick={() => { const existing = latestCaseByCluster.get(analysis.clusters[0].id); if (existing) showCase(existing); else void openInvestigation(analysis.clusters[0].id); }}>{t("investigation.startCase")}</button>}
        </section>}

        {analysis && analysis.clusters.length > 0 ? <section className="incident-section">
          <header className="workbench-section-heading"><div><span className="section-index">ISSUES</span><h2>{t("hunter.clustersTitle")}</h2></div><strong>{analysis.clusters.length}</strong></header>
          <div className="incident-list">{analysis.clusters.map(cluster => {
            const existing = latestCaseByCluster.get(cluster.id);
            return <article key={cluster.id} className={`incident-row ${activeCase?.cluster.id === cluster.id ? "selected" : ""}`}>
              <div className="incident-severity"><span/><small>{translatedValue(t, "failureClass", cluster.class)}</small></div>
              <div className="incident-copy"><h3>{cluster.summary}</h3><div><code>{cluster.signature}</code><span>{t("hunter.clusterImpact").replace("{checks}", String(cluster.check_ids.length)).replace("{evidence}", String(cluster.evidence_ids.length))}</span></div></div>
              <div className="incident-actions"><button className="button" disabled={busy} onClick={() => { if (existing) showCase(existing); else void openInvestigation(cluster.id); }}>{existing ? t("investigation.openCase") : t("investigation.startCase")}</button><button className="icon-text-action" onClick={() => onInvestigate(cluster.investigation_query)}>{t("investigation.askAgent")}</button></div>
            </article>;
          })}</div>
        </section> : report ? <section className="workbench-empty-result"><CheckIcon/><div><h2>{t("problems.none")}</h2><p>{t("problems.noneHelp")}</p></div></section> : null}

        <section className="checks-table-section">
          <header className="workbench-section-heading"><div><span className="section-index">RUN</span><h2>{t("checks.title")}</h2></div><span className="selection-readout">{selected.length} / {selectableIds.size} {t("checks.selected")}</span></header>
          {profile.commands.length === 0 ? <div className="quiet-empty"><h3>{t("checks.none")}</h3></div> : <div className="checks-table" role="table"><div className="checks-table-header" role="row"><span/><span>{t("checks.check")}</span><span>{t("checks.kind")}</span><span>{t("checks.result")}</span><span>{t("checks.time")}</span></div>{profile.commands.map(command => {
            const canRun = selectableIds.has(command.id);
            const result = resultByCommand.get(command.id);
            const planned = planByCommand.get(command.id);
            return <div key={command.id} className={`checks-table-row ${result ? `check-${result.status.toLowerCase()}` : ""}`} role="row">
              <label className="check-select compact-select" title={canRun ? t("checks.select") : t("checks.manualOnly")}><input type="checkbox" checked={selected.includes(command.id)} disabled={!canRun || busy} onChange={() => toggle(command.id)}/><span/></label>
              <div className="check-name"><strong>{command.label}</strong><code>{formatProjectCommand(command)}</code></div>
              <div className="check-source"><span>{translatedValue(t, "commandKind", command.kind)}</span><small>{planned ? `#${planned.order} · ${translatedValue(t, "planStage", planned.stage)}` : command.source}</small></div>
              <div className="check-state">{result ? <><strong>{translatedValue(t, "checkStatus", result.status)}</strong><small>{result.exit_code === null ? result.summary : `exit ${result.exit_code} · ${result.duration_ms} ms`}</small></> : <span>—</span>}</div>
              <span className="check-time">{result ? `${result.duration_ms} ms` : "—"}</span>
              {result && (result.stdout_preview || result.stderr_preview || result.summary) ? <details className="check-log" open={settings.behavior.open_logs_on_failure && ["Failed", "Error", "TimedOut"].includes(result.status)}><summary>{t("checks.output")}</summary><div><p>{result.summary}</p>{result.stderr_preview && <pre>{result.stderr_preview}</pre>}{result.stdout_preview && <pre>{result.stdout_preview}</pre>}<code>{result.evidence_id}</code></div></details> : null}
            </div>;
          })}</div>}
        </section>

        <details className="technical-drawer" open={!report}>
          <summary><span>{t("hunter.planTitle")}</span><small>{plan?.strategy ?? "diagnostics-first-v1"}</small></summary>
          {loadingPlan ? <div className="quiet-empty compact"><p>{t("common.loading")}</p></div> : plan && plan.steps.length > 0 ? <div className="technical-plan-list">{plan.steps.map(step => <div key={step.command_id}><b>{String(step.order).padStart(2,"0")}</b><span><strong>{step.label}</strong><small>{translatedValue(t, "planStage", step.stage)} · {translatedValue(t, "planCost", step.cost)} · {t(`planReason.${step.reason_code}`)}</small></span><code>{step.command_id}</code></div>)}</div> : <div className="quiet-empty compact"><h3>{t("hunter.noPlan")}</h3></div>}
        </details>

        {analysis && analysis.blockers.length > 0 && <section className="workbench-blockers"><header className="workbench-section-heading"><div><span className="section-index">BLOCKED</span><h2>{t("hunter.blockersTitle")}</h2></div><strong>{analysis.blockers.length}</strong></header>{analysis.blockers.map(item => <div key={`${item.command_id}:${item.evidence_id}`}><strong>{item.label}</strong><span>{item.summary}</span><code>{item.command_id}</code></div>)}</section>}
        {error && <div className="inline-error page-message">{error}</div>}
      </main>

      {investigationPresence.mounted && <aside className={`investigation-workbench-pane presence-${investigationPresence.phase}`} style={{ width: inspectorWidth }}><ResizeHandle side="left" value={inspectorWidth} min={360} max={760} label={t("settings.inspectorWidth")} onChange={onInspectorWidth} onCommit={onInspectorWidthCommit}/>{activeCase ? <InvestigationCasePanel value={activeCase} onChange={updateCase} onClose={() => onInspectorOpen(false)} ai={settings.ai} onPrepareVerification={onPrepareVerification} profile={profile}/> : pendingCluster && <section className="investigation-preview"><header><div><span>{t("investigation.observed")}</span><h2>{pendingCluster.title}</h2></div><button className="button small" onClick={() => onInspectorOpen(false)}>{t("common.close")}</button></header><p>{pendingCluster.summary}</p><dl><div><dt>{t("checks.evidence")}</dt><dd>{pendingCluster.evidence_ids.length}</dd></div><div><dt>{t("investigation.state")}</dt><dd>{t("investigation.notCreated")}</dd></div></dl><div className="privacy-lock"><strong>{t("investigation.noSilentCase")}</strong><p>{t("investigation.noSilentCaseHelp")}</p></div><button className="button primary" disabled={busy} onClick={() => void openInvestigation(pendingCluster.id)}>{t("investigation.startCase")}</button></section>}</aside>}
    </div>

    {cases.length > 0 && !inspectorOpen && <div className="case-history-bar"><span>{t("investigation.recentHistory")}</span>{cases.slice(0, 4).map(item => <button key={item.id} onClick={() => showCase(item)}><strong>{item.cluster.title}</strong><small>{translatedValue(t, "investigationState", item.state)}</small></button>)}<button className="view-all-cases" onClick={() => setHistoryOpen(true)}>{t("investigation.viewAll")} · {cases.length}</button></div>}
    {historyPresence.mounted && <div className={`case-history-layer presence-${historyPresence.phase}`} role="presentation" onMouseDown={() => setHistoryOpen(false)}><section className="case-history-dialog" role="dialog" aria-modal="true" aria-labelledby="case-history-title" onMouseDown={event => event.stopPropagation()}><header><div><h2 id="case-history-title">{t("investigation.allHistory")}</h2><p>{t("investigation.historyHelp")}</p></div><button className="icon-close" aria-label={t("common.close")} onClick={() => setHistoryOpen(false)}>×</button></header><input autoFocus value={historyQuery} onChange={event => setHistoryQuery(event.target.value)} placeholder={t("investigation.searchHistory")}/><div className="case-history-list">{filteredCases.map(item => <button key={item.id} onClick={() => showCase(item)}><span><strong>{item.cluster.title}</strong><small>{item.cluster.signature}</small></span><span><b>{translatedValue(t, "investigationState", item.state)}</b><time>{new Date(item.updated_at * 1000).toLocaleString()}</time></span></button>)}{filteredCases.length === 0 && <p>{t("palette.noResults")}</p>}</div></section></div>}
  </section>;
}

function Agent({ profile, settings, seed }: { profile: ProjectProfile; settings: AppSettings; seed: string }) {
  const { t } = useI18n();
  const [query, setQuery] = useState(seed);
  const [apiKey, setApiKey] = useState("");
  const [context, setContext] = useState<ContextPacket | null>(null);
  const [analysis, setAnalysis] = useState<string | null>(null);
  const [busy, setBusy] = useState<"context" | "ai" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => { if (seed) setQuery(seed); }, [seed]);

  async function buildContext() {
    if (!query.trim()) return;
    setBusy("context"); setError(null); setAnalysis(null);
    try {
      const packet = await invokeTauri<ContextPacket>("compile_project_context", { path: profile.root_path, query, maxFiles: 12, maxChars: 36000 });
      setContext(packet);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function investigate() {
    if (!query.trim()) return;
    if (!settings.ai.enabled || !settings.ai.model.trim()) { setError(t("agent.configureModel")); return; }
    if (!await confirmAction(t("confirm.aiInvestigation"))) return;
    setBusy("ai"); setError(null); setAnalysis(null);
    try {
      const result = await invokeTauri<Investigation>("ai_investigate_project", { path: profile.root_path, question: query, apiKey: apiKey.trim() || null, confirmedNetwork: true });
      setContext(result.context); setAnalysis(result.analysis);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function copyContext() {
    if (!context) return;
    try {
      await navigator.clipboard.writeText(formatContextPacket(context));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } catch (nextError) {
      setError(bridgeMessage(nextError));
    }
  }

  return <section className="view-page project-view agent-view"><header className="view-heading"><div><h1>{t("agent.title")}</h1><p>{t("agent.description")}</p></div><div className="agent-model"><span className={settings.ai.enabled ? "model-dot enabled" : "model-dot"}/><div><strong>{settings.ai.enabled && settings.ai.model ? settings.ai.model : t("agent.noModel")}</strong><small>{settings.ai.enabled ? settings.ai.base_url : t("agent.disabled")}</small></div></div></header>
    <section className="investigation-composer"><label htmlFor="investigation-query">{t("agent.question")}</label><textarea id="investigation-query" rows={4} value={query} onChange={event => setQuery(event.target.value)} placeholder={t("agent.placeholder")}/><div className="agent-actions"><button className="button" disabled={!query.trim() || busy !== null} onClick={() => void buildContext()}>{busy === "context" ? t("agent.compiling") : t("agent.buildContext")}</button><button className="button primary" disabled={!query.trim() || busy !== null || !settings.ai.enabled} onClick={() => void investigate()}>{busy === "ai" ? t("agent.investigating") : t("agent.investigate")}</button></div>{settings.ai.enabled && !settings.ai.base_url.includes("127.0.0.1") && !settings.ai.base_url.includes("localhost") && <label className="ephemeral-key"><span>{t("agent.apiKey")}</span><input type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} autoComplete="off" placeholder={t("agent.apiKeyHelp")}/></label>}</section>

    {error && <div className="inline-error page-message">{error}</div>}
    {(context || analysis) && <div className="investigation-layout">
      <section className="investigation-result"><header><h2>{analysis ? t("agent.investigation") : t("agent.contextReady")}</h2>{analysis && <span>{t("agent.notVerification")}</span>}</header>{analysis ? <pre className="analysis-output">{analysis}</pre> : <div className="analysis-placeholder"><strong>{t("agent.contextReady")}</strong><p>{t("agent.contextReadyHelp")}</p></div>}</section>
      <aside className="context-panel"><header><div><h2>{t("agent.contextCompiler")}</h2><span>{context?.snippets.length ?? 0} {t("agent.snippets")}</span></div>{context && <button className="text-button" onClick={() => void copyContext()}>{copied ? t("common.copied") : t("agent.copyContext")}</button>}</header>{context && <><div className="context-stats"><span><strong>{context.stats.files_considered.toLocaleString()}</strong><small>{t("agent.considered")}</small></span><span><strong>{context.stats.selected_chars.toLocaleString()}</strong><small>{t("agent.characters")}</small></span><span><strong>{context.stats.sensitive_files_excluded}</strong><small>{t("agent.secretsExcluded")}</small></span></div><div className="context-list">{context.snippets.map(snippet => <details key={snippet.id}><summary><span><strong>{snippet.path}</strong><small>{snippet.id}</small></span><b>{snippet.score}</b></summary><div className="context-meta">{snippet.reasons.join(" · ")} · L{snippet.line_start}–{snippet.line_end}</div><pre>{snippet.content}</pre></details>)}</div></>}</aside>
    </div>}
  </section>;
}

export function ProjectWorkspace({ profile, tab, settings, investigationSeed, onRefresh, onInvestigate, inspectorOpen, inspectorWidth, preferredInvestigationCaseId, onInvestigationCaseChange, onInspectorOpen, onInspectorWidth, onInspectorWidthCommit, onPrepareVerification }: Props) {
  switch (tab) {
    case "problems": return <Problems profile={profile} onInvestigate={onInvestigate}/>;
    case "agent": return <Agent profile={profile} settings={settings} seed={investigationSeed}/>;
    case "checks": return <Checks profile={profile} onInvestigate={onInvestigate} settings={settings} inspectorOpen={inspectorOpen} inspectorWidth={inspectorWidth} preferredInvestigationCaseId={preferredInvestigationCaseId} onInvestigationCaseChange={onInvestigationCaseChange} onInspectorOpen={onInspectorOpen} onInspectorWidth={onInspectorWidth} onInspectorWidthCommit={onInspectorWidthCommit} onPrepareVerification={onPrepareVerification}/>;
    default: return <ProjectOverview profile={profile} onRefresh={onRefresh}/>;
  }
}
