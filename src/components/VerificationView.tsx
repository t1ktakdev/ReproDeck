import { useCallback, useEffect, useState } from "react";
import { translatedValue, useI18n } from "../i18n";
import { commandText } from "../lib/format";
import { activeCycleRuns, latestRun } from "../lib/proof";
import { bridgeMessage, confirmAction, invokeTauri } from "../lib/tauri";
import type { RegressionCheck, ReproductionRun, ReproductionStep, VerificationStatus } from "../types";

type Props = { sessionId: string; steps: ReproductionStep[]; runs: ReproductionRun[]; onReload: () => Promise<void> };

export function VerificationView({ sessionId, steps, runs, onReload }: Props) {
  const { t } = useI18n();
  const [status, setStatus] = useState<VerificationStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const refresh = useCallback(async () => setStatus(await invokeTauri<VerificationStatus>("session_verification_status", { sessionId })), [sessionId]);

  useEffect(() => {
    let cancelled = false;
    setStatus(null);
    setError(null);
    void refresh().catch(nextError => { if (!cancelled) setError(bridgeMessage(nextError)); });
    return () => { cancelled = true; };
  }, [refresh, runs]);

  async function copy(label: string, value: string) {
    await navigator.clipboard.writeText(value); setCopied(label);
    window.setTimeout(() => setCopied(current => current === label ? null : current), 1400);
  }
  async function runRegression(check: RegressionCheck) {
    if (!await confirmAction(t("confirm.runRegression").replace("{command}", commandText(check.executable, check.args)))) return;
    setBusy(check.id); setError(null);
    try { await invokeTauri("run_regression_check", { checkId: check.id, approvedOnce: true }); await onReload(); await refresh(); }
    catch (nextError) { setError(bridgeMessage(nextError)); } finally { setBusy(null); }
  }
  async function promote(check: RegressionCheck) {
    setBusy(check.id); setError(null);
    try { await invokeTauri("promote_regression_check", { checkId: check.id, level: "Required" }); await refresh(); }
    catch (nextError) { setError(bridgeMessage(nextError)); } finally { setBusy(null); }
  }

  const step = steps[0] ?? null;
  const stepRuns = step ? activeCycleRuns(step, runs) : [];
  const before = latestRun(stepRuns, "Before");
  const after = latestRun(stepRuns, "After");
  const proof = status?.proof ?? null;
  const hash = proof?.identity.patch_sha256 ?? status?.handoff?.patch_sha256 ?? null;

  return <section className="view-page verification-page">
    <header className="view-heading"><div><h2>{t("verification.title")}</h2><p>{t("verification.description")}</p></div>{status&&<span className={`verdict ${status.ready_to_apply?"verifiedfix":"inconclusive"}`}>{status.ready_to_apply?t("verification.verified"):t(`verificationReason.${status.reason_code}`)}</span>}</header>
    {!step ? <div className="quiet-empty"><h3>{t("verification.inconclusive")}</h3></div> : !status ? <div className="loading-state" role="status"><span className="loading-spinner"/>{t("common.loading")}</div> : <>
      <section className="proof-chain" aria-label={t("verification.proofChain")}>
        <header><div><h3>{t("verification.proofChain")}</h3><p>{t("verification.proofChainHelp")}</p></div><span>{status?.required_passed ?? 0}/{status?.required_total ?? 0} {t("verification.requiredChecks")}</span></header>
        <div className="proof-chain-flow">
          <article className={before?.status==="Failed"?"complete":"pending"}><small>01 · {t("overview.before")}</small><strong>{before?translatedValue(t,"run",before.status):t("overview.notRun")}</strong><span>{before?.receipt_id??t("verification.noReceipt")}</span></article>
          <article className={status?.handoff?.activated_at?"complete":"pending"}><small>02 · {t("verification.transferredPatch")}</small><strong>{status?.handoff?t("verification.identityChecked"):t("verification.notLinked")}</strong><span>{status?.handoff?.patch_sha256.slice(0,16)??"—"}</span></article>
          <article className={after?.status==="Passed"?"complete":"pending"}><small>03 · {t("overview.after")}</small><strong>{after?translatedValue(t,"run",after.status):t("overview.notRun")}</strong><span>{after?.receipt_id??t("verification.noReceipt")}</span></article>
          <article className={status?.required_total===status?.required_passed?"complete":"pending"}><small>04 · {t("verification.regressions")}</small><strong>{status?.required_passed??0}/{status?.required_total??0}</strong><span>{t("verification.requiredPassed")}</span></article>
          <article className={status?.ready_to_apply?"complete":"pending"}><small>05 · Apply</small><strong>{status?.ready_to_apply?t("changes.ready"):t("changes.verifyFirst")}</strong><span>{status?t(`verificationReason.${status.reason_code}`):"—"}</span></article>
        </div>
      </section>

      <section className="verification-facts">
        <div><small>{t("verification.command")}</small><code>{commandText(step.executable, step.args)}</code><span>exit {step.expected_exit_code} · cycle {step.active_cycle}</span></div>
        <div><small>{t("verification.sourceCommit")}</small><code>{proof?.identity.source_commit.slice(0,16)??status?.handoff?.source_commit.slice(0,16)??"—"}</code><span>{t("verification.sourceStateBound")}</span></div>
        <div><small>{t("verification.patchIdentity")}</small><code>{hash?.slice(0,20)??"—"}</code>{hash&&<button className="button ghost small" onClick={()=>void copy("patch",hash)}>{copied==="patch"?t("common.copied"):t("common.copy")}</button>}</div>
        <div><small>{t("verification.afterReceipt")}</small><code>{proof?.after_run_id??"—"}</code><span>{proof?new Date(proof.verified_at*1000).toLocaleString():t("verification.pending")}</span></div>
      </section>

      {status?.handoff&&<details className="proof-links"><summary>{t("verification.investigationLinks")}</summary><dl><div><dt>Case</dt><dd><code>{status.handoff.investigation_case_id}</code></dd></div><div><dt>Hypothesis</dt><dd><code>{status.handoff.hypothesis_id}</code></dd></div><div><dt>Experiment</dt><dd><code>{status.handoff.experiment_id}</code></dd></div><div><dt>{t("verification.files")}</dt><dd>{status.handoff.files.join(", ")}</dd></div></dl></details>}

      <section className="regression-contract">
        <header><div><h3>{t("verification.regressionContract")}</h3><p>{t("verification.regressionContractHelp")}</p></div></header>
        {status?.regressions.length ? status.regressions.map(check=><div className="regression-row" key={check.id}>
          <span className={`criterion-mark ${check.status.toLowerCase()}`}>{check.status==="Passed"?"✓":check.status==="Failed"?"×":"·"}</span>
          <div><strong>{check.title}</strong><code>{commandText(check.executable,check.args)}</code><small>{check.receipt_id??t("verification.noReceipt")}</small></div>
          <span className={`regression-level level-${check.level.toLowerCase()}`}>{t(`regressionLevel.${check.level}`)}</span>
          {check.level!=="Required"&&<button className="button ghost small" disabled={busy!==null} onClick={()=>void promote(check)}>{t("verification.makeRequired")}</button>}
          <button className="button small" disabled={busy!==null||!proof} onClick={()=>void runRegression(check)}>{busy===check.id?t("common.working"):t("verification.runCheck")}</button>
        </div>) : <div className="quiet-empty compact"><h3>{t("verification.noRegressions")}</h3><p>{t("verification.noRegressionsHelp")}</p></div>}
      </section>
    </>}
    {error&&<div className="inline-error page-message" role="alert">{error}</div>}
  </section>;
}
