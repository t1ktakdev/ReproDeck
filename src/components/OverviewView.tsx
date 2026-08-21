import { useMemo, useState } from "react";
import { translatedValue, useI18n } from "../i18n";
import { bytes, commandText, sessionMeta } from "../lib/format";
import { activeCycleRuns, canRunAfter, latestRun } from "../lib/proof";
import { bridgeCode, bridgeMessage, chooseCapsuleDestination, confirmAction, invokeTauri, openExternalUrl, revealLocalPath } from "../lib/tauri";
import type { CapsuleExportPreview, CapsuleSummary, EnvironmentSnapshot, GitHubCreatedItem, ReproductionRun, ReproductionStep, RepositoryInfo, Session, ShadowWorkspace } from "../types";
import { PlayIcon } from "./Icons";

type Props = {
  session: Session;
  repository: RepositoryInfo | null;
  shadow: ShadowWorkspace | null;
  steps: ReproductionStep[];
  runs: ReproductionRun[];
  environment: EnvironmentSnapshot | null;
  onReload: () => Promise<void>;
  onGoChanges: () => void;
};

export function OverviewView({ session, repository, shadow, steps, runs, environment, onReload, onGoChanges }: Props) {
  const { t } = useI18n();
  const meta = sessionMeta(session);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [exportPreview, setExportPreview] = useState<CapsuleExportPreview | null>(null);
  const primary = steps[0] ?? null;
  const stepRuns = useMemo(() => primary ? activeCycleRuns(primary, runs) : [], [primary, runs]);
  const latestBefore = latestRun(stepRuns, "Before");
  const latestAfter = latestRun(stepRuns, "After");

  async function run(phase: "Before" | "After") {
    if (!primary) return;
    setBusy(phase); setError(null); setNotice(null);
    try {
      await invokeTauri("execute_reproduction_step", { stepId: primary.id, phase, approvedOnce: true });
      await onReload();
    } catch (nextError) {
      if (bridgeCode(nextError) === "approval_required") setNotice(bridgeMessage(nextError));
      else setError(bridgeMessage(nextError));
    } finally { setBusy(null); }
  }

  async function resetBaseline() {
    if (!primary) return;
    const confirmed = await confirmAction(t("confirm.resetBaseline"));
    if (!confirmed) return;
    setBusy("reset"); setError(null); setNotice(null);
    try { await invokeTauri("reset_reproduction_baseline", { stepId: primary.id, confirmed: true }); await onReload(); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function prepareCapsuleExport() {
    setError(null); setNotice(null); setBusy("preview-export");
    try {
      setExportPreview(await invokeTauri<CapsuleExportPreview>("preview_session_capsule", { sessionId: session.id }));
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function exportCapsule() {
    if (!exportPreview) return;
    setError(null); setNotice(null);
    try {
      const safeName = (meta.title || session.id).replace(/[<>:"/\\|?*]+/g, "-").trim() || session.id;
      const destination = await chooseCapsuleDestination(`${safeName}.reprodeck`, t("dialog.exportCapsule"), t("dialog.capsuleFilter"));
      if (!destination) return;
      setBusy("export");
      const summary = await invokeTauri<CapsuleSummary>("export_session_capsule", { sessionId: session.id, destination });
      setExportPreview(null);
      setNotice(`${t("capsules.exportDone")} ${summary.file_count} ${t("capsules.files")}${summary.redactions.length ? ` · ${summary.redactions.length} ${t("capsules.redactions")}` : ""}`);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function createGitHubIssue() {
    if (!repository) return;
    const body = [
      `## Bug`, meta.actual || "No actual behavior recorded.", "", `## Expected`, meta.expected || "—", "",
      `## Reproduction`, primary ? `\`${commandText(primary.executable, primary.args)}\`` : "—", "",
      `## Verification`, `Before: ${latestBefore?.status ?? "Not run"}${latestBefore?.exit_code != null ? ` (exit ${latestBefore.exit_code})` : ""}`, `After: ${latestAfter?.status ?? "Not run"}${latestAfter?.exit_code != null ? ` (exit ${latestAfter.exit_code})` : ""}`,
      "", "_Created by ReproDeck after explicit confirmation._",
    ].join("\n");
    if (!await confirmAction(t("github.confirmIssue"))) return;
    setBusy("issue"); setError(null);
    try {
      const item = await invokeTauri<GitHubCreatedItem>("github_create_issue", { sessionId: session.id, title: meta.title || session.id, body, confirmed: true });
      await openExternalUrl(item.url);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function createDraftPr() {
    if (!await confirmAction(t("github.confirmPr"))) return;
    setBusy("pr"); setError(null);
    try {
      const body = `## Summary\n${meta.title || session.id}\n\n## Verification\nBefore: ${latestBefore?.status ?? "—"}\nAfter: ${latestAfter?.status ?? "—"}\n\nReproDeck verified the configured reproduction criterion before Apply.`;
      const item = await invokeTauri<GitHubCreatedItem>("github_create_draft_pr", { sessionId: session.id, title: meta.title || session.id, body, confirmed: true });
      await openExternalUrl(item.url);
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  async function copyWorkspace() {
    if (!shadow) return;
    try { await navigator.clipboard.writeText(shadow.worktree_path); setCopied(true); setTimeout(() => setCopied(false), 1500); }
    catch { setError(t("common.copyFailed")); }
  }

  return <section className="view-page overview-page">
    <header className="session-summary"><div><h1>{meta.title || session.id}</h1><p>{meta.actual || "—"}</p></div><div className="session-header-actions"><span className="session-state">{translatedValue(t, "state", session.state)}</span><button className="button" disabled={busy !== null} onClick={() => void prepareCapsuleExport()}>{busy === "preview-export" ? t("common.loading") : t("capsules.export")}</button></div></header>

    <div className="overview-grid">
      <section className="plain-panel problem-panel"><header><h2>{t("overview.problem")}</h2></header><dl><div><dt>{t("overview.expected")}</dt><dd>{meta.expected || "—"}</dd></div><div><dt>{t("overview.actual")}</dt><dd>{meta.actual || "—"}</dd></div>{meta.notes && <div><dt>{t("overview.notes")}</dt><dd>{meta.notes}</dd></div>}</dl></section>

      <section className="plain-panel repo-panel"><header><h2>{t("overview.repository")}</h2>{repository?.is_dirty && <span className="small-warning">{t("top.localChanges")}</span>}</header>{repository ? <dl><div><dt>{t("overview.path")}</dt><dd className="mono-wrap">{repository.path}</dd></div><div><dt>{t("overview.branch")}</dt><dd>{repository.branch}</dd></div><div><dt>{t("overview.head")}</dt><dd><code>{repository.head_commit.slice(0, 12)}</code></dd></div></dl> : <p className="muted-copy">—</p>}</section>

      <section className="plain-panel reproduction-panel"><header><div><h2>{t("overview.reproduction")}</h2><p>{t("overview.reproHelp")}</p></div></header>
        {primary ? <>
          <div className="command-line"><code>{commandText(primary.executable, primary.args)}</code><span>exit {primary.expected_exit_code} = success · cycle {primary.active_cycle}</span></div>
          <div className="proof-row">
            <div className={`proof-cell ${latestBefore ? latestBefore.status.toLowerCase() : "pending"}`}><span>{t("overview.before")}</span><strong>{latestBefore ? `${translatedValue(t, "run", latestBefore.status)}${latestBefore.exit_code !== null ? ` · exit ${latestBefore.exit_code}` : ""}` : t("overview.notRun")}</strong><small>{t("overview.proveBug")}</small></div>
            <div className="proof-arrow">→</div>
            <div className={`proof-cell ${latestAfter ? latestAfter.status.toLowerCase() : "pending"}`}><span>{t("overview.after")}</span><strong>{latestAfter ? `${translatedValue(t, "run", latestAfter.status)}${latestAfter.exit_code !== null ? ` · exit ${latestAfter.exit_code}` : ""}` : t("overview.notRun")}</strong><small>{t("overview.runAfterFix")}</small></div>
          </div>
          <div className="action-row">
            {!latestBefore ? <button className="button primary" disabled={busy !== null} onClick={() => void run("Before")}><PlayIcon/>{busy === "Before" ? t("overview.running") : t("overview.runBefore")}</button> : <button className="button" disabled={busy !== null} onClick={() => void resetBaseline()}>{t("overview.resetBaseline")}</button>}
            <button className="button primary-after" disabled={busy !== null || !canRunAfter(latestBefore)} onClick={() => void run("After")}><PlayIcon/>{busy === "After" ? t("overview.running") : latestAfter ? t("overview.runAfterAgain") : t("overview.runAfter")}</button>
          </div>
          {latestBefore && <div className="baseline-note"><strong>{t("overview.baselineProtected")}</strong><span>{t("overview.baselineProtectedHelp")}</span></div>}
        </> : <p className="muted-copy">—</p>}
      </section>

      <section className="plain-panel isolation-panel"><header><h2>{t("overview.isolated")}</h2><span className={shadow ? "safe-badge" : "muted-badge"}>{shadow ? t("overview.active") : t("overview.notCreated")}</span></header>
        {shadow ? <><p>{t("overview.isolatedHelp")}</p><div className="path-box"><code>{shadow.worktree_path}</code><div className="path-actions"><button className="text-button" onClick={() => void revealLocalPath(shadow.worktree_path).catch(nextError => setError(bridgeMessage(nextError)))}>{t("overview.showExplorer")}</button><button className="text-button" onClick={() => void copyWorkspace()}>{copied ? t("common.copied") : t("overview.copyPath")}</button></div></div><button className="button" onClick={onGoChanges}>{t("overview.reviewChanges")}</button></> : <p className="muted-copy">—</p>}
      </section>

      <section className="plain-panel environment-card"><header><h2>{t("environment.title")}</h2></header>{environment ? <><strong>{environment.os} · {environment.arch}</strong><p>{environment.git_version || "Git —"}</p><div className="runtime-list">{Object.entries(environment.runtimes).map(([name, version]) => <span key={name}><b>{name}</b>{version}</span>)}</div></> : <p className="muted-copy">—</p>}</section>

      <section className="plain-panel integration-panel"><header><h2>{t("github.title")}</h2></header><p>{t("github.help")}</p><div className="action-row"><button className="button" disabled={!repository || busy !== null} onClick={() => void createGitHubIssue()}>{t("github.issue")}</button><button className="button" disabled={session.state !== "Applied" || busy !== null} onClick={() => void createDraftPr()}>{t("github.pr")}</button></div></section>
    </div>
    {error && <div className="inline-error page-message">{error}</div>}
    {notice && <div className="inline-notice page-message">{notice}</div>}

    {exportPreview && <div className="modal-layer" role="presentation" onMouseDown={() => busy === null && setExportPreview(null)}>
      <section className="capsule-review-dialog" role="dialog" aria-modal="true" aria-labelledby="capsule-review-title" onMouseDown={event => event.stopPropagation()}>
        <header><div><h2 id="capsule-review-title">{t("capsules.previewTitle")}</h2><p>{exportPreview.summary.title} · {exportPreview.summary.file_count} {t("capsules.files")} · {bytes(exportPreview.summary.total_uncompressed_bytes)}</p></div><button className="icon-close" aria-label={t("common.close")} disabled={busy !== null} onClick={() => setExportPreview(null)}>×</button></header>
        <div className="capsule-review-body">
          <section><h3>{t("capsules.included")}</h3><div className="capsule-file-list">{exportPreview.files.map(file => <div key={file.path}><code>{file.path}</code><span>{bytes(file.size)}</span></div>)}</div></section>
          <section><h3>{t("capsules.redacted")}</h3>{exportPreview.summary.redactions.length ? <ul>{exportPreview.summary.redactions.map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul> : <p className="muted-copy">{t("capsules.noRedactions")}</p>}</section>
        </div>
        <footer><button className="button" disabled={busy !== null} onClick={() => setExportPreview(null)}>{t("common.cancel")}</button><button className="button primary" disabled={busy !== null} onClick={() => void exportCapsule()}>{busy === "export" ? t("common.working") : t("capsules.continueExport")}</button></footer>
      </section>
    </div>}
  </section>;
}
