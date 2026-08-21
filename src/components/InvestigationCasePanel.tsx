import { useEffect, useMemo, useState } from "react";
import { translatedValue, useI18n } from "../i18n";
import { bridgeMessage, confirmAction, invokeTauri, revealLocalPath } from "../lib/tauri";
import { formatProjectCommand, regressionRecommendations } from "../lib/project";
import { classifyEvidence, type EvidenceRelationship } from "../lib/uiBehavior";
import type {
  AiSettings,
  ContextPacket,
  FixWorkspaceDiff,
  FixWorkspaceRecord,
  HypothesisDraft,
  InvestigationCase,
  InvestigationHypothesis,
  ProjectProfile,
} from "../types";
import { SegmentedControl, Spinner } from "./ui";

type Props = {
  value: InvestigationCase;
  onChange: (value: InvestigationCase) => void;
  onClose: () => void;
  ai: AiSettings;
  onPrepareVerification: (value: InvestigationCase) => void;
  profile: ProjectProfile;
};

type EvidenceDetail = {
  id: string;
  kind: "observed" | "source" | "experiment" | "context";
  source: string;
  location: string;
  excerpt: string;
  checksum: string;
  timestamp: number | null;
  reasons: string[];
  truncated: boolean;
};

function evidenceDetail(value: InvestigationCase, id: string): EvidenceDetail {
  if (id === value.criterion.baseline_evidence_id) {
    return {
      id,
      kind: "observed",
      source: "Project Health",
      location: [value.criterion.executable, ...value.criterion.args].join(" "),
      excerpt: value.criterion.baseline_stderr_preview || value.criterion.baseline_stdout_preview || value.criterion.baseline_summary,
      checksum: value.base_commit,
      timestamp: value.criterion.baseline_finished_at || value.created_at,
      reasons: [value.criterion.label],
      truncated: false,
    };
  }
  const source = value.source_evidence.find(item => item.id === id);
  if (source) {
    return {
      id,
      kind: "source",
      source: source.path,
      location: `${source.path}:${source.line_start}-${source.line_end}`,
      excerpt: source.excerpt,
      checksum: source.checksum,
      timestamp: value.updated_at,
      reasons: source.reasons,
      truncated: source.truncated,
    };
  }
  const experiment = value.experiments.find(item => item.evidence_id === id);
  if (experiment) {
    return {
      id,
      kind: "experiment",
      source: "Causal experiment",
      location: experiment.command_id,
      excerpt: experiment.stderr_preview || experiment.stdout_preview,
      checksum: experiment.intervention_sha256,
      timestamp: experiment.finished_at,
      reasons: [String(experiment.conclusion)],
      truncated: false,
    };
  }
  return {
    id,
    kind: "context",
    source: "Failure cluster",
    location: value.cluster.signature,
    excerpt: value.cluster.summary,
    checksum: value.base_commit,
    timestamp: value.created_at,
    reasons: [],
    truncated: false,
  };
}

function hypothesisRank(value: InvestigationHypothesis, investigation: InvestigationCase) {
  const supportedExperiments = investigation.experiments.filter(item => item.hypothesis_id === value.id && item.conclusion === "SupportsHypothesis").length;
  const status = value.status === "Supported" ? 0 : value.status === "Proposed" ? 1 : value.status === "Inconclusive" ? 2 : 3;
  return status * 100_000 + value.contradicting_evidence_ids.length * 10_000 - supportedExperiments * 1_000 - value.supporting_evidence_ids.length * 100 - value.accepted_confidence_percent;
}

export function InvestigationCasePanel({ value, onChange, onClose, ai, onPrepareVerification, profile }: Props) {
  const { t } = useI18n();
  const [workspace, setWorkspace] = useState<FixWorkspaceRecord | null>(null);
  const [context, setContext] = useState<ContextPacket | null>(null);
  const [diff, setDiff] = useState<FixWorkspaceDiff | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [statement, setStatement] = useState("");
  const [rationale, setRationale] = useState("");
  const [falsifier, setFalsifier] = useState("");
  const [nextExperiment, setNextExperiment] = useState("");
  const [confidence, setConfidence] = useState(60);
  const [relationships, setRelationships] = useState<Record<string, EvidenceRelationship>>({});
  const [apiKey, setApiKey] = useState("");

  useEffect(() => {
    let active = true;
    setError(null);
    setRelationships({});
    invokeTauri<FixWorkspaceRecord | null>("get_fix_workspace", { caseId: value.id })
      .then(next => { if (active) setWorkspace(next); })
      .catch(nextError => { if (active) setError(bridgeMessage(nextError)); });
    return () => { active = false; };
  }, [value.id]);

  const latestExperiment = value.experiments[value.experiments.length - 1] ?? null;
  const rankedHypotheses = useMemo(() => [...value.hypotheses].sort((a, b) => hypothesisRank(a, value) - hypothesisRank(b, value)), [value]);
  const regressionChecks = useMemo(() => regressionRecommendations(profile, workspace?.changed_files ?? [], value.criterion.command_id), [profile, value.criterion.command_id, workspace?.changed_files]);

  async function action<T>(name: string, fn: () => Promise<T>, after: (result: T) => void) {
    setBusy(name);
    setError(null);
    try { after(await fn()); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function buildContext() {
    await action("context", () => invokeTauri<ContextPacket>("compile_investigation_context", { caseId: value.id }), packet => {
      setContext(packet);
      void invokeTauri<InvestigationCase[]>("list_investigation_cases", { path: value.root_path })
        .then(items => items.find(item => item.id === value.id))
        .then(item => { if (item) onChange(item); })
        .catch(nextError => setError(bridgeMessage(nextError)));
    });
  }

  async function createWorkspace() {
    await action("workspace", () => invokeTauri<FixWorkspaceRecord>("create_fix_workspace", { caseId: value.id }), setWorkspace);
  }

  async function refreshWorkspace() {
    await action("refresh", () => invokeTauri<FixWorkspaceRecord | null>("get_fix_workspace", { caseId: value.id }), setWorkspace);
  }

  async function checkpoint() {
    await action("checkpoint", () => invokeTauri<FixWorkspaceRecord>("checkpoint_fix_workspace", { caseId: value.id }), next => {
      setWorkspace(next);
      setDiff(null);
    });
  }

  async function reviewDiff() {
    await action("diff", () => invokeTauri<FixWorkspaceDiff>("fix_workspace_diff", { caseId: value.id }), setDiff);
  }

  async function generateHypothesesWithAi() {
    if (!ai.enabled || !ai.model.trim()) {
      setError(t("investigation.aiUnavailable"));
      return;
    }
    if (!await confirmAction(t("confirm.aiHypotheses"))) return;
    await action("ai-hypotheses", () => invokeTauri<InvestigationCase>("generate_investigation_hypotheses", {
      caseId: value.id,
      apiKey: apiKey.trim() || null,
      confirmedNetwork: true,
    }), next => {
      onChange(next);
      setContext(null);
    });
  }

  async function saveHypothesis() {
    if (!statement.trim() || !falsifier.trim() || !nextExperiment.trim()) {
      setError(t("investigation.hypothesisRequiredFields"));
      return;
    }
    const classified = classifyEvidence(value.evidence_ids, relationships);
    const draft: HypothesisDraft = {
      statement: statement.trim(),
      rationale: rationale.trim(),
      ...classified,
      falsifier: falsifier.trim(),
      next_experiment: nextExperiment.trim(),
      confidence_percent: confidence,
      source: "Manual",
    };
    await action("hypothesis", () => invokeTauri<InvestigationCase>("record_investigation_hypotheses", { caseId: value.id, hypotheses: [draft] }), next => {
      onChange(next);
      setStatement("");
      setRationale("");
      setFalsifier("");
      setNextExperiment("");
      setRelationships({});
    });
  }

  async function runExperiment(hypothesisId: string) {
    if (!await confirmAction(t("confirm.causalExperiment"))) return;
    await action("experiment", () => invokeTauri<InvestigationCase>("run_causal_experiment", {
      caseId: value.id,
      hypothesisId,
      timeoutSecs: 180,
      confirmedExecution: true,
    }), next => {
      onChange(next);
      setDiff(null);
      void refreshWorkspace();
    });
  }

  async function discardWorkspace() {
    if (!await confirmAction(t("confirm.discardFixWorkspace"))) return;
    await action("discard", () => invokeTauri<void>("discard_fix_workspace", { caseId: value.id, confirmed: true }), () => {
      setWorkspace(null);
      setDiff(null);
    });
  }

  function renderEvidence(id: string, relationship?: EvidenceRelationship) {
    const detail = evidenceDetail(value, id);
    return <details className={`evidence-disclosure evidence-${relationship?.toLowerCase() ?? detail.kind}`} key={`${relationship ?? "detail"}:${id}`}>
      <summary><span><code>{id}</code><small>{detail.source}</small></span>{relationship && <b>{t(`evidenceRelationship.${relationship}`)}</b>}</summary>
      <div className="evidence-disclosure-body">
        <dl><div><dt>{t("investigation.evidenceType")}</dt><dd>{t(`evidenceKind.${detail.kind}`)}</dd></div><div><dt>{t("investigation.location")}</dt><dd>{detail.location}</dd></div><div><dt>{t("investigation.checksum")}</dt><dd><code>{detail.checksum.slice(0, 16)}</code></dd></div>{detail.timestamp && <div><dt>{t("investigation.captured")}</dt><dd>{new Date(detail.timestamp * 1000).toLocaleString()}</dd></div>}</dl>
        {detail.reasons.length > 0 && <p className="evidence-relevance"><strong>{t("investigation.relevance")}</strong> {detail.reasons.join(" · ")}</p>}
        {detail.excerpt ? <pre>{detail.excerpt}</pre> : <p className="muted-copy">{t("investigation.excerptUnavailable")}</p>}
        {detail.truncated && <small>{t("inspector.truncated")}</small>}
      </div>
    </details>;
  }

  return <section className="investigation-case-panel">
    <header className="investigation-case-header">
      <div>
        <span className="project-kicker">{t("investigation.kicker")}</span>
        <h2>{value.cluster.title}</h2>
        <p><code>{value.cluster.signature}</code></p>
      </div>
      <div className="investigation-header-actions">
        <span className={`investigation-state state-${value.state.toLowerCase()}`}>{translatedValue(t, "investigationState", value.state)}</span>
        <button className="button small" onClick={onClose}>{t("common.close")}</button>
      </div>
    </header>

    <section className="investigation-case-summary" aria-label={t("investigation.summary")}>
      <div><small>{t("investigation.observedFailure")}</small><strong>{translatedValue(t, "checkStatus", value.criterion.baseline_status)}</strong><span>exit {value.criterion.baseline_exit_code ?? "—"} · {value.criterion.baseline_duration_ms} ms</span></div>
      <div><small>{t("investigation.evidenceCollected")}</small><strong>{value.evidence_ids.length}</strong><span>{value.source_evidence.length} {t("investigation.sourceFragments")}</span></div>
      <div><small>{t("investigation.candidates")}</small><strong>{value.hypotheses.length}</strong><span>{value.hypotheses.filter(item => item.status === "Supported").length} {t("investigation.supported")}</span></div>
      <div><small>{t("investigation.experiment")}</small><strong>{latestExperiment ? translatedValue(t, "experimentConclusion", latestExperiment.conclusion) : t("investigation.pending")}</strong><span>{latestExperiment ? `${latestExperiment.changed_files.length} ${t("investigation.filesChanged")}` : t("investigation.interventionHelp")}</span></div>
    </section>

    <section className="observed-failure-section">
      <header><div><h3>{t("investigation.observedFailure")}</h3><p>{value.criterion.baseline_summary || value.cluster.summary}</p></div><button className="button small" disabled={busy !== null} onClick={() => void buildContext()}>{busy === "context" ? <><Spinner label={t("common.working")}/>{t("common.working")}</> : value.source_evidence.length ? t("investigation.refreshContext") : t("investigation.compileContext")}</button></header>
      {renderEvidence(value.criterion.baseline_evidence_id, "Neutral")}
    </section>

    {context && <details className="investigation-context"><summary>{t("investigation.sourceContext")} · {context.snippets.length}</summary><div>{context.snippets.map(snippet => <article key={snippet.id}><header><code>{snippet.path}:{snippet.line_start}-{snippet.line_end}</code><span>{snippet.score}</span></header><pre>{snippet.content}</pre><small>{snippet.id} · {snippet.reasons.join(" · ")}</small></article>)}</div><section className="context-exclusions"><h4>{t("investigation.excludedContext")}</h4>{context.stats.sensitive_files_excluded>0&&<p><code>.env / secret paths</code><span>{context.stats.sensitive_files_excluded} · {t("investigation.secretPolicyReason")}</span></p>}<p><code>node_modules / build outputs</code><span>{t("investigation.ignorePolicyReason")}</span></p>{context.stats.skipped_large_or_binary>0&&<p><code>{context.stats.skipped_large_or_binary} {t("investigation.largeOrBinary")}</code><span>{t("investigation.budgetReason")}</span></p>}</section><footer>{context.stats.selected_chars.toLocaleString()} / 36,000 {t("investigation.contextBudget")} · {context.stats.files_ranked}/{context.stats.files_considered} {t("investigation.filesRanked")} · {context.stats.packet_truncated?t("investigation.truncated"):t("investigation.withinBudget")}</footer></details>}

    {value.hypotheses.length === 0 ? <section className="hypothesis-editor">
      <header><div><h3>{t("investigation.addHypothesis")}</h3><p>{t("investigation.addHypothesisHelp")}</p></div><div className="hypothesis-ai-actions"><button className="button small" disabled={busy !== null || !ai.enabled || !ai.model.trim()} onClick={() => void generateHypothesesWithAi()}>{busy === "ai-hypotheses" ? <><Spinner label={t("investigation.generatingAi")}/>{t("investigation.generatingAi")}</> : t("investigation.generateAi")}</button><small>{ai.enabled && ai.model ? ai.model : t("investigation.aiUnavailable")}</small></div></header>
      {ai.enabled && !ai.base_url.includes("127.0.0.1") && !ai.base_url.includes("localhost") && <label className="hypothesis-ai-key"><span>{t("agent.apiKey")}</span><input type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} autoComplete="off"/></label>}
      <label><span>{t("investigation.statement")}</span><input value={statement} onChange={event => setStatement(event.target.value)} placeholder={t("investigation.statementPlaceholder")}/></label>
      <label><span>{t("investigation.rationale")}</span><textarea value={rationale} onChange={event => setRationale(event.target.value)} rows={2}/></label>
      <div className="hypothesis-grid"><label><span>{t("investigation.falsifier")}</span><input value={falsifier} onChange={event => setFalsifier(event.target.value)} placeholder={t("investigation.falsifierPlaceholder")}/></label><label><span>{t("investigation.nextExperiment")}</span><input value={nextExperiment} onChange={event => setNextExperiment(event.target.value)} placeholder={t("investigation.nextExperimentPlaceholder")}/></label></div>
      <section className="evidence-selector"><header><h4>{t("investigation.classifyEvidence")}</h4><p>{t("investigation.classifyEvidenceHelp")}</p></header>{value.evidence_ids.map(id => { const detail = evidenceDetail(value, id); const relationship = relationships[id] ?? "Neutral"; return <div className="evidence-selector-row" key={id}><details><summary><code>{id}</code><span>{detail.source}</span></summary>{detail.excerpt ? <pre>{detail.excerpt}</pre> : <p>{t("investigation.excerptUnavailable")}</p>}</details><SegmentedControl ariaLabel={`${id} ${t("investigation.relationship")}`} value={relationship} options={(["Supports", "Neutral", "Contradicts"] as const).map(item => ({ value: item, label: t(`evidenceRelationship.${item}`) }))} onChange={next => setRelationships(current => ({ ...current, [id]: next }))}/></div>; })}</section>
      <div className="hypothesis-footer"><label><span>{t("investigation.confidence")}</span><input type="number" min={0} max={100} value={confidence} onChange={event => setConfidence(Math.max(0, Math.min(100, Number(event.target.value) || 0)))}/></label><small>{t("investigation.confidenceHelp")}</small><button className="button primary small" disabled={busy !== null} onClick={() => void saveHypothesis()}>{busy === "hypothesis" ? t("common.working") : t("investigation.saveHypothesis")}</button></div>
    </section> : <section className="hypothesis-list">
      {rankedHypotheses.map((hypothesis, index) => { const supportingExperiments=value.experiments.filter(item=>item.hypothesis_id===hypothesis.id&&item.conclusion==="SupportsHypothesis").length; return <article key={hypothesis.id} className={`hypothesis-row hypothesis-${hypothesis.status.toLowerCase()}`}>
        <header><span>{t("investigation.hypothesisLabel").replace("{count}", String(index + 1))}</span><strong>{hypothesis.accepted_confidence_percent}%</strong><small>{t("investigation.confidenceNotProbability")}</small><em>{translatedValue(t, "hypothesisStatus", hypothesis.status)}</em></header>
        <div className="hypothesis-copy"><h3>{hypothesis.statement}</h3>{hypothesis.rationale && <p>{hypothesis.rationale}</p>}<div className="hypothesis-questions"><p><strong>{t("investigation.falsifier")}</strong>{hypothesis.falsifier}</p><p><strong>{t("investigation.nextExperiment")}</strong>{hypothesis.next_experiment}</p></div>
          <div className="hypothesis-ranking"><strong>{t("investigation.rankingWhy")}</strong><span>{supportingExperiments} {t("investigation.supportingExperiments")}</span><span>{hypothesis.contradicting_evidence_ids.length} {t("investigation.contradictions")}</span><span>{hypothesis.supporting_evidence_ids.length} {t("investigation.supportingSources")}</span>{workspace?.changed_files.length===1&&hypothesis.status==="Supported"&&<span>{t("investigation.minimalIntervention")}</span>}</div>
          <details className="hypothesis-evidence"><summary>{t("investigation.evidence")} · {hypothesis.supporting_evidence_ids.length + hypothesis.neutral_evidence_ids.length + hypothesis.contradicting_evidence_ids.length}</summary>
            {hypothesis.supporting_evidence_ids.length > 0 && <section><h4>{t("evidenceRelationship.Supports")}</h4>{hypothesis.supporting_evidence_ids.map(id => renderEvidence(id, "Supports"))}</section>}
            {hypothesis.contradicting_evidence_ids.length > 0 && <section><h4>{t("evidenceRelationship.Contradicts")}</h4>{hypothesis.contradicting_evidence_ids.map(id => renderEvidence(id, "Contradicts"))}</section>}
            {hypothesis.neutral_evidence_ids.length > 0 && <section><h4>{t("evidenceRelationship.Neutral")}</h4>{hypothesis.neutral_evidence_ids.map(id => renderEvidence(id, "Neutral"))}</section>}
          </details>
          {hypothesis.rejected_evidence_ids.length > 0 && <details className="citation-rejection"><summary>{t("investigation.citationRejected")} · {hypothesis.rejected_evidence_ids.length}</summary>{hypothesis.rejected_evidence_ids.map(id => <code key={id}>{id}</code>)}</details>}
        </div>
        <section className="experiment-plan"><h4>{t("investigation.experimentPlan")}</h4><dl><div><dt>{t("investigation.whatChanges")}</dt><dd>{workspace?.changed_files.length ? workspace.changed_files.join(", ") : t("investigation.checkpointNeeded")}</dd></div><div><dt>{t("investigation.whyTests")}</dt><dd>{hypothesis.next_experiment}</dd></div><div><dt>{t("investigation.exactCommand")}</dt><dd><code>{[value.criterion.executable, ...value.criterion.args].join(" ")}</code></dd></div><div><dt>{t("investigation.expectedResult")}</dt><dd>exit {value.criterion.expected_exit_code}</dd></div></dl><p><strong>{t("investigation.originalProtected")}</strong> {t("investigation.originalProtectedHelp")}</p><button className="button small" disabled={busy !== null || !workspace || workspace.dirty || workspace.changed_files.length === 0} onClick={() => void runExperiment(hypothesis.id)}>{busy === "experiment" ? t("common.working") : t("investigation.testHypothesis")}</button></section>
      </article>;})}
    </section>}

    <section className="fix-workspace-card">
      <header><div><h3>{t("investigation.fixWorkspace")}</h3><p>{t("investigation.fixWorkspaceHelp")}</p></div><span className="apply-locked">{t("investigation.applyLocked")}</span></header>
      {!workspace ? <div className="fix-workspace-empty"><p>{t("investigation.noWorkspace")}</p><button className="button primary small" disabled={busy !== null} onClick={() => void createWorkspace()}>{busy === "workspace" ? t("common.working") : t("investigation.createWorkspace")}</button></div> : <>
        <dl className="workspace-facts"><div><dt>{t("overview.branch")}</dt><dd><code>{workspace.branch}</code></dd></div><div><dt>{t("overview.path")}</dt><dd title={workspace.project_path}>{workspace.project_path}</dd></div><div><dt>{t("investigation.state")}</dt><dd>{workspace.dirty ? t("investigation.uncheckpointed") : `${workspace.changed_files.length} ${t("investigation.filesChanged")}`}</dd></div></dl>
        <div className="workspace-actions"><button className="button small" onClick={() => void revealLocalPath(workspace.project_path).catch(nextError => setError(bridgeMessage(nextError)))}>{t("investigation.open")}</button><button className="button small" disabled={busy !== null} onClick={() => void refreshWorkspace()}>{t("common.refresh")}</button><button className="button small" disabled={busy !== null || !workspace.dirty} onClick={() => void checkpoint()}>{busy === "checkpoint" ? t("common.working") : t("investigation.checkpoint")}</button><button className="button small" disabled={busy !== null || workspace.dirty || workspace.changed_files.length === 0} onClick={() => void reviewDiff()}>{t("investigation.reviewDiff")}</button><button className="button danger small" disabled={busy !== null} onClick={() => void discardWorkspace()}>{t("investigation.discard")}</button></div>
      </>}
      <div className="apply-lock-copy"><strong>{t("investigation.originalProtected")}</strong><span>{t("investigation.originalProtectedHelp")}</span></div>
      {diff && <details className="investigation-diff" open><summary>{t("investigation.reviewedPatch")} · {diff.files.length}</summary><pre>{diff.patch}</pre></details>}
    </section>

    {value.experiments.length > 0 && <section className="experiment-results"><header><h3>{t("investigation.experimentResults")}</h3><span>{value.experiments.length}</span></header>{[...value.experiments].reverse().map(experiment => <article key={experiment.id} className={`causal-result result-${experiment.conclusion.toLowerCase()}`}><header><div><span>{t("investigation.causalExperiment")}</span><strong>{translatedValue(t, "experimentConclusion", experiment.conclusion)}</strong></div><code>{experiment.evidence_id}</code></header><div className="causal-comparison"><span><small>{t("investigation.baseline")}</small><strong>{translatedValue(t, "checkStatus", value.criterion.baseline_status)}</strong><b>exit {value.criterion.baseline_exit_code ?? "—"} · {value.criterion.baseline_duration_ms} ms</b></span><span><small>{t("investigation.experiment")}</small><strong>{translatedValue(t, "checkStatus", experiment.status)}</strong><b>exit {experiment.exit_code ?? "—"} · {Math.max(0, experiment.finished_at - experiment.started_at)} s</b></span></div><dl><div><dt>{t("investigation.changedFiles")}</dt><dd>{experiment.changed_files.join(", ") || "—"}</dd></div><div><dt>{t("investigation.originalRepo")}</dt><dd>{experiment.original_unchanged ? t("investigation.unchanged") : t("investigation.changed")}</dd></div><div><dt>{t("investigation.commandMutation")}</dt><dd>{experiment.workspace_unchanged_by_command ? t("investigation.none") : t("investigation.detected")}</dd></div></dl><p>{t("investigation.supportNotProof")}</p></article>)}</section>}

    {value.hypotheses.some(item => item.status === "Supported") && <section className="verification-handoff">
      <header><h3>{t("investigation.continueVerification")}</h3><span>{t("investigation.protectedFlow")}</span></header>
      <p>{t("investigation.continueVerificationHelp")}</p>
      <div><span>{t("investigation.supportedRootCause")}</span><span>{t("investigation.proposedFix")}</span><strong>{t("investigation.beforeAfter")}</strong><span>{t("investigation.regression")}</span><span>{t("investigation.readyApply")}</span></div>
      {regressionChecks.length > 0 && <section className="regression-recommendations">{(["required", "recommended", "optional"] as const).map(tier => { const checks = regressionChecks.filter(item => item.tier === tier); return checks.length > 0 && <div key={tier}><h4>{t(`regression.${tier}`)}</h4>{checks.map(item => <p key={item.command.id}><span><strong>{item.command.label}</strong><small>{t(`regressionReason.${item.reasons[0]}`)}</small></span><code>{formatProjectCommand(item.command)}</code></p>)}</div>; })}</section>}
      <button className="button primary" disabled={!workspace || workspace.dirty || workspace.changed_files.length === 0} onClick={() => onPrepareVerification(value)}>{t("investigation.prepareVerification")}</button>
      {(!workspace || workspace.dirty || workspace.changed_files.length === 0) && <small>{t("investigation.prepareVerificationBlocked")}</small>}
    </section>}

    {error && <div className="inline-error page-message" role="alert">{error}</div>}
  </section>;
}
