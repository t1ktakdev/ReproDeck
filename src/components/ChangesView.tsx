import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { bridgeCode, bridgeMessage, invokeTauri } from "../lib/tauri";
import { usePresence } from "../lib/usePresence";
import type { ShadowDiff, ShadowWorkspace, VerificationStatus } from "../types";
import { CheckIcon, WarningIcon } from "./Icons";

type Props = { sessionId: string; shadow: ShadowWorkspace | null; onReload: () => Promise<void> };
function Diff({ patch }: { patch: string }) {
  return <pre className="diff-viewer" aria-label="Git diff">{patch.split("\n").map((line, index) => {
    const kind = line.startsWith("+") && !line.startsWith("+++") ? "add" : line.startsWith("-") && !line.startsWith("---") ? "del" : line.startsWith("@@") ? "hunk" : line.startsWith("diff --git") ? "file" : "";
    return <span key={index} className={kind}>{line || " "}</span>;
  })}</pre>;
}

export function ChangesView({ sessionId, shadow, onReload }: Props) {
  const { t } = useI18n();
  const [diff, setDiff] = useState<ShadowDiff | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<"apply" | "discard" | null>(null);
  const lastConfirm = useRef<"apply" | "discard">("apply");
  const [verificationStatus, setVerificationStatus] = useState<VerificationStatus | null>(null);
  if (confirm) lastConfirm.current = confirm;
  const confirmPresence = usePresence(confirm !== null, 180);
  const visibleConfirm = confirm ?? lastConfirm.current;

  async function refreshDiff(checkpoint: boolean) {
    if (!shadow) { setDiff(null); return; }
    setBusy(checkpoint ? "checkpoint" : "refresh"); setError(null);
    try {
      if (checkpoint) {
        try { await invokeTauri("finalize_shadow_workspace", { sessionId }); }
        catch (nextError) { if (bridgeCode(nextError) !== "no_changes") throw nextError; }
      }
      setDiff(await invokeTauri<ShadowDiff>("shadow_diff", { sessionId }));
      setVerificationStatus(await invokeTauri<VerificationStatus>("session_verification_status", { sessionId }));
      await onReload();
    } catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }
  useEffect(() => { void refreshDiff(false); }, [sessionId, shadow?.branch]); // eslint-disable-line react-hooks/exhaustive-deps

  async function apply() {
    setBusy("apply"); setError(null);
    try { await invokeTauri("apply_shadow_workspace", { sessionId, confirmed: true }); setConfirm(null); setDiff(null); await onReload(); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }
  async function discard() {
    setBusy("discard"); setError(null);
    try { await invokeTauri("discard_shadow_workspace", { sessionId, confirmed: true }); setConfirm(null); setDiff(null); await onReload(); }
    catch (nextError) { setError(bridgeMessage(nextError)); }
    finally { setBusy(null); }
  }

  return <section className="view-page changes-page">
    <header className="view-heading"><div><h2>{t("changes.title")}</h2><p>{t("changes.description")}</p></div>{shadow && <button className="button" onClick={() => void refreshDiff(true)} disabled={busy !== null}>{busy === "checkpoint" ? t("changes.checkpointing") : t("changes.checkpoint")}</button>}</header>
    {!shadow ? <div className="quiet-empty"><h3>{t("changes.noWorkspace")}</h3><p>{t("changes.noWorkspaceHelp")}</p></div> : <>
      <div className="safety-strip"><CheckIcon/><div><strong>{t("changes.originalSeparate")}</strong><span>Base {shadow.base_commit.slice(0, 10)} · {shadow.branch}</span></div>{shadow.dirty && <span className="working-badge">{t("changes.uncheckpointed")}</span>}</div>
      {diff && diff.files.length > 0 ? <div className="changes-layout"><aside className="changed-files"><header>{diff.files.length}</header>{diff.files.map(path => <div key={path}>{path}</div>)}</aside><Diff patch={diff.patch}/></div> : <div className="quiet-empty compact"><h3>{t("changes.noDiff")}</h3><p>{t("changes.noDiffHelp")}</p></div>}
      {verificationStatus?.proof&&<section className={`patch-proof-strip ${verificationStatus.ready_to_apply?"ready":"blocked"}`}><div><small>{t("verification.patchIdentity")}</small><code title={verificationStatus.proof.identity.patch_sha256}>{verificationStatus.proof.identity.patch_sha256.slice(0,16)}</code></div><div><small>{t("verification.sourceCommit")}</small><code>{verificationStatus.proof.identity.source_commit.slice(0,12)}</code></div><div><small>{t("verification.requiredChecks")}</small><strong>{verificationStatus.required_passed}/{verificationStatus.required_total}</strong></div></section>}
      <footer className="apply-bar"><div><strong>{verificationStatus?.ready_to_apply ? t("changes.ready") : t("changes.verifyFirst")}</strong><span>{verificationStatus?.ready_to_apply ? t("changes.readyHelp") : verificationStatus ? t(`verificationReason.${verificationStatus.reason_code}`) : t("changes.verifyHelp")}</span></div><button className="button danger-quiet" onClick={() => setConfirm("discard")} disabled={busy !== null}>{t("changes.discard")}</button><button className="button primary" onClick={() => setConfirm("apply")} disabled={busy !== null || !diff || diff.files.length === 0 || !verificationStatus?.ready_to_apply}>{t("changes.apply")}</button></footer>
    </>}
    {error && <div className="inline-error page-message">{error}</div>}
    {confirmPresence.mounted && <div className={`modal-layer presence-${confirmPresence.phase}`}><section className="confirm-dialog" role="dialog" aria-modal="true"><WarningIcon/><div><h3>{visibleConfirm === "apply" ? t("changes.applyTitle") : t("changes.discardTitle")}</h3><p>{visibleConfirm === "apply" ? t("changes.applyConfirm") : t("changes.discardConfirm")}</p></div><footer><button className="button ghost" onClick={() => setConfirm(null)} disabled={busy !== null}>{t("common.cancel")}</button><button className={visibleConfirm === "apply" ? "button primary" : "button danger"} onClick={() => void (visibleConfirm === "apply" ? apply() : discard())} disabled={busy !== null}>{busy ? t("common.working") : visibleConfirm === "apply" ? t("changes.apply") : t("changes.discard")}</button></footer></section></div>}
  </section>;
}
