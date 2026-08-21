use crate::evidence;
use crate::permissions::{self, Permission, PermissionDecision};
use crate::runner::{self, CommandSpec};
use crate::shadow_session::{self, PatchIdentity, ShadowSessionError};
use crate::state_machine::{self, SessionState};
use crate::timeline;
use crate::workflow::{self, ReproductionStep};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum VerificationError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Shadow(#[from] ShadowSessionError),
    #[error(transparent)]
    Timeline(#[from] timeline::TimelineError),
    #[error(transparent)]
    Evidence(#[from] evidence::EvidenceError),
    #[error(transparent)]
    State(#[from] state_machine::StateError),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("the transferred patch does not match its recorded SHA-256 identity")]
    HandoffHashMismatch,
    #[error("the investigation patch was produced from a different source commit")]
    HandoffSourceMismatch,
    #[error("verification handoff not found for session: {0}")]
    HandoffNotFound(String),
    #[error("the exact Before baseline must fail before the investigation patch can be activated")]
    BaselineNotFailed,
    #[error("the current patch is not the patch transferred from the investigation")]
    TransferredPatchMismatch,
    #[error("a successful After run can only be recorded for a clean checkpoint")]
    UncheckpointedAfter,
    #[error("verified patch proof is missing or no longer matches the current workspace")]
    ProofMismatch,
    #[error("regression check not found: {0}")]
    RegressionNotFound(String),
    #[error("regression checks can be promoted, but cannot be demoted after creation")]
    RegressionDemotion,
    #[error("command denied: {0}")]
    PermissionDenied(String),
    #[error("command requires approval: {0}")]
    ApprovalRequired(String),
}

pub type Result<T> = std::result::Result<T, VerificationError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegressionLevel {
    Required,
    Recommended,
    Optional,
}

impl std::fmt::Display for RegressionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::str::FromStr for RegressionLevel {
    type Err = VerificationError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "Required" => Ok(Self::Required),
            "Recommended" => Ok(Self::Recommended),
            "Optional" => Ok(Self::Optional),
            _ => Err(VerificationError::RegressionNotFound(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionDraft {
    pub stable_id: String,
    pub title: String,
    pub executable: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    pub level: RegressionLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionCheck {
    pub id: String,
    pub session_id: String,
    pub stable_id: String,
    pub title: String,
    pub executable: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    pub level: RegressionLevel,
    pub status: String,
    pub receipt_id: Option<String>,
    pub verified_patch_sha256: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationHandoff {
    pub session_id: String,
    pub investigation_case_id: String,
    pub hypothesis_id: String,
    pub experiment_id: String,
    pub source_commit: String,
    pub patch_sha256: String,
    pub patch_size: u64,
    pub files: Vec<String>,
    pub activated_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationProof {
    pub session_id: String,
    pub step_id: String,
    pub cycle: i64,
    pub identity: PatchIdentity,
    pub criterion_sha256: String,
    pub command_sha256: String,
    pub after_run_id: String,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationStatus {
    pub outcome: String,
    pub ready_to_apply: bool,
    pub reason_code: String,
    pub message: String,
    pub current_identity: Option<PatchIdentity>,
    pub proof: Option<VerificationProof>,
    pub handoff: Option<VerificationHandoff>,
    pub regressions: Vec<RegressionCheck>,
    pub required_passed: usize,
    pub required_total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionRunOutcome {
    pub check: RegressionCheck,
    pub permission: PermissionDecision,
}

#[derive(Debug, Clone)]
pub struct HandoffCandidate {
    pub investigation_case_id: String,
    pub hypothesis_id: String,
    pub experiment_id: String,
    pub source_commit: String,
    pub patch: Vec<u8>,
    pub files: Vec<String>,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn sha256(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}

fn command_sha256(step: &ReproductionStep) -> Result<String> {
    Ok(sha256(serde_json::to_vec(&serde_json::json!({
        "executable": step.executable,
        "args": step.args,
    }))?))
}

fn criterion_sha256(step: &ReproductionStep) -> Result<String> {
    Ok(sha256(serde_json::to_vec(&serde_json::json!({
        "command_sha256": command_sha256(step)?,
        "expected_exit_code": step.expected_exit_code,
        "cycle": step.active_cycle,
    }))?))
}

fn proof_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationProof> {
    let files_json: String = row.get(9)?;
    Ok(VerificationProof {
        session_id: row.get(0)?,
        step_id: row.get(1)?,
        cycle: row.get(2)?,
        identity: PatchIdentity {
            source_commit: row.get(3)?,
            source_state_sha256: row.get(4)?,
            shadow_commit: row.get(5)?,
            patch_sha256: row.get(6)?,
            patch_size: row.get::<_, i64>(7)? as u64,
            files: serde_json::from_str(&files_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        },
        criterion_sha256: row.get(8)?,
        command_sha256: row.get(10)?,
        after_run_id: row.get(11)?,
        verified_at: row.get(12)?,
    })
}

pub fn proof(conn: &Connection, session_id: &str) -> Result<Option<VerificationProof>> {
    Ok(conn
        .query_row(
            "SELECT session_id,step_id,cycle,source_commit,source_state_sha256,shadow_commit,patch_sha256,patch_size,criterion_sha256,files_json,command_sha256,after_run_id,verified_at FROM verification_proofs WHERE session_id=?1",
            rusqlite::params![session_id],
            proof_from_row,
        )
        .optional()?)
}

fn handoff_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationHandoff> {
    let files_json: String = row.get(7)?;
    Ok(VerificationHandoff {
        session_id: row.get(0)?,
        investigation_case_id: row.get(1)?,
        hypothesis_id: row.get(2)?,
        experiment_id: row.get(3)?,
        source_commit: row.get(4)?,
        patch_sha256: row.get(5)?,
        patch_size: row.get::<_, i64>(6)? as u64,
        files: serde_json::from_str(&files_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        activated_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

pub fn handoff(conn: &Connection, session_id: &str) -> Result<Option<VerificationHandoff>> {
    Ok(conn
        .query_row(
            "SELECT session_id,investigation_case_id,hypothesis_id,experiment_id,source_commit,patch_sha256,patch_size,files_json,activated_at,created_at FROM verification_handoffs WHERE session_id=?1",
            rusqlite::params![session_id],
            handoff_row,
        )
        .optional()?)
}

fn regression_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RegressionCheck> {
    let args_json: String = row.get(5)?;
    let level: String = row.get(7)?;
    Ok(RegressionCheck {
        id: row.get(0)?,
        session_id: row.get(1)?,
        stable_id: row.get(2)?,
        title: row.get(3)?,
        executable: row.get(4)?,
        args: serde_json::from_str(&args_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        expected_exit_code: row.get(6)?,
        level: level.parse().map_err(|error: VerificationError| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        status: row.get(8)?,
        receipt_id: row.get(9)?,
        verified_patch_sha256: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub fn list_regressions(conn: &Connection, session_id: &str) -> Result<Vec<RegressionCheck>> {
    let mut statement = conn.prepare(
        "SELECT id,session_id,stable_id,title,executable,args_json,expected_exit_code,level,status,receipt_id,verified_patch_sha256,created_at,updated_at FROM regression_checks WHERE session_id=?1 ORDER BY CASE level WHEN 'Required' THEN 0 WHEN 'Recommended' THEN 1 ELSE 2 END, created_at, id",
    )?;
    let values = statement
        .query_map(rusqlite::params![session_id], regression_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(values)
}

pub fn stage_handoff(
    conn: &Connection,
    session_id: &str,
    candidate: HandoffCandidate,
    regressions: &[RegressionDraft],
) -> Result<VerificationHandoff> {
    let shadow = shadow_session::get_session_shadow(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    if shadow.base_commit != candidate.source_commit {
        return Err(VerificationError::HandoffSourceMismatch);
    }
    let patch_sha256 = sha256(&candidate.patch);
    if candidate.patch.is_empty() {
        return Err(VerificationError::HandoffHashMismatch);
    }
    shadow_session::check_patch_against_session(conn, session_id, &candidate.patch)?;
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO verification_handoffs(session_id,investigation_case_id,hypothesis_id,experiment_id,source_commit,patch_sha256,patch_size,patch_bytes,files_json,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![session_id,candidate.investigation_case_id,candidate.hypothesis_id,candidate.experiment_id,candidate.source_commit,patch_sha256,candidate.patch.len() as i64,candidate.patch,serde_json::to_string(&candidate.files)?,now],
    )?;
    for draft in regressions {
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO regression_checks(id,session_id,stable_id,title,executable,args_json,expected_exit_code,level,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
            rusqlite::params![id,session_id,draft.stable_id,draft.title,draft.executable,serde_json::to_string(&draft.args)?,draft.expected_exit_code,draft.level.to_string(),now],
        )?;
    }
    handoff(conn, session_id)?.ok_or_else(|| VerificationError::HandoffNotFound(session_id.into()))
}

pub fn activate_handoff_after_baseline(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<PatchIdentity>> {
    let Some(value) = handoff(conn, session_id)? else {
        return Ok(None);
    };
    if value.activated_at.is_some() {
        return Ok(Some(shadow_session::current_patch_identity(
            conn, session_id,
        )?));
    }
    let baseline_failed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM reproduction_runs rr JOIN reproduction_steps rs ON rs.id=rr.step_id WHERE rs.session_id=?1 AND rr.phase='Before' AND rr.cycle=rs.active_cycle AND rr.status='Failed')",
        rusqlite::params![session_id],
        |row| row.get(0),
    )?;
    if !baseline_failed {
        return Err(VerificationError::BaselineNotFailed);
    }
    let patch: Vec<u8> = conn.query_row(
        "SELECT patch_bytes FROM verification_handoffs WHERE session_id=?1",
        rusqlite::params![session_id],
        |row| row.get(0),
    )?;
    if sha256(&patch) != value.patch_sha256 {
        return Err(VerificationError::HandoffHashMismatch);
    }
    let identity = shadow_session::apply_patch_and_checkpoint(conn, session_id, &patch)?;
    if identity.patch_sha256 != value.patch_sha256 || identity.source_commit != value.source_commit
    {
        return Err(VerificationError::TransferredPatchMismatch);
    }
    let now = unix_time_secs()?;
    conn.execute(
        "UPDATE verification_handoffs SET activated_at=?1 WHERE session_id=?2",
        rusqlite::params![now, session_id],
    )?;
    let action = timeline::new_action(
        session_id,
        "verification:patch-transferred",
        "Succeeded",
        Some(
            serde_json::json!({
                "investigation_case_id": value.investigation_case_id,
                "hypothesis_id": value.hypothesis_id,
                "experiment_id": value.experiment_id,
                "patch_sha256": value.patch_sha256,
                "files": value.files,
            })
            .to_string(),
        ),
    );
    if let Ok(action) = action {
        let _ = timeline::create_action(conn, &action);
    }
    Ok(Some(identity))
}

pub fn invalidate_proof(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM verification_proofs WHERE session_id=?1",
        rusqlite::params![session_id],
    )?;
    Ok(())
}

pub fn record_after_success(
    conn: &Connection,
    step: &ReproductionStep,
    after_run_id: &str,
) -> Result<VerificationProof> {
    let identity = shadow_session::current_patch_identity(conn, &step.session_id).map_err(
        |error| match error {
            ShadowSessionError::UncommittedChanges => VerificationError::UncheckpointedAfter,
            other => VerificationError::Shadow(other),
        },
    )?;
    if let Some(value) = handoff(conn, &step.session_id)? {
        if value.activated_at.is_none() || value.patch_sha256 != identity.patch_sha256 {
            return Err(VerificationError::TransferredPatchMismatch);
        }
    }
    let proof = VerificationProof {
        session_id: step.session_id.clone(),
        step_id: step.id.clone(),
        cycle: step.active_cycle,
        identity,
        criterion_sha256: criterion_sha256(step)?,
        command_sha256: command_sha256(step)?,
        after_run_id: after_run_id.to_owned(),
        verified_at: unix_time_secs()?,
    };
    conn.execute(
        "INSERT INTO verification_proofs(session_id,step_id,cycle,source_commit,source_state_sha256,shadow_commit,patch_sha256,patch_size,files_json,criterion_sha256,command_sha256,after_run_id,verified_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(session_id) DO UPDATE SET step_id=excluded.step_id,cycle=excluded.cycle,source_commit=excluded.source_commit,source_state_sha256=excluded.source_state_sha256,shadow_commit=excluded.shadow_commit,patch_sha256=excluded.patch_sha256,patch_size=excluded.patch_size,files_json=excluded.files_json,criterion_sha256=excluded.criterion_sha256,command_sha256=excluded.command_sha256,after_run_id=excluded.after_run_id,verified_at=excluded.verified_at",
        rusqlite::params![proof.session_id,proof.step_id,proof.cycle,proof.identity.source_commit,proof.identity.source_state_sha256,proof.identity.shadow_commit,proof.identity.patch_sha256,proof.identity.patch_size as i64,serde_json::to_string(&proof.identity.files)?,proof.criterion_sha256,proof.command_sha256,proof.after_run_id,proof.verified_at],
    )?;
    let action = timeline::new_action(
        &proof.session_id,
        "verification:proof-recorded",
        "Succeeded",
        Some(
            serde_json::json!({
                "step_id": proof.step_id,
                "cycle": proof.cycle,
                "after_run_id": proof.after_run_id,
                "source_commit": proof.identity.source_commit,
                "source_state_sha256": proof.identity.source_state_sha256,
                "shadow_commit": proof.identity.shadow_commit,
                "patch_sha256": proof.identity.patch_sha256,
                "patch_size": proof.identity.patch_size,
                "files": proof.identity.files,
                "criterion_sha256": proof.criterion_sha256,
                "command_sha256": proof.command_sha256,
                "original_repository_unchanged": true,
            })
            .to_string(),
        ),
    )?;
    timeline::create_action(conn, &action)?;
    Ok(proof)
}

fn primary_proof_matches(
    conn: &Connection,
    session_id: &str,
) -> Result<(VerificationProof, PatchIdentity)> {
    let proof = proof(conn, session_id)?.ok_or(VerificationError::ProofMismatch)?;
    let step = workflow::get_reproduction_step(conn, &proof.step_id)
        .map_err(|_| VerificationError::ProofMismatch)?;
    let identity = shadow_session::current_patch_identity(conn, session_id)?;
    if proof.cycle != step.active_cycle
        || proof.criterion_sha256 != criterion_sha256(&step)?
        || proof.command_sha256 != command_sha256(&step)?
        || proof.identity != identity
    {
        return Err(VerificationError::ProofMismatch);
    }
    Ok((proof, identity))
}

fn status_message(reason: &str) -> (&'static str, &'static str) {
    match reason {
        "ready" => (
            "VerifiedFix",
            "The exact patch and every required check are verified.",
        ),
        "baseline_missing" => ("Inconclusive", "Run Before to prove the baseline failure."),
        "baseline_passed" => (
            "ReproductionNotProven",
            "Before passed, so the failure was not reproduced.",
        ),
        "after_missing" => ("Inconclusive", "Checkpoint the fix and run After."),
        "after_failed" => ("NotFixed", "After did not satisfy the primary criterion."),
        "uncommitted_changes" => (
            "PatchChanged",
            "Checkpoint or discard uncommitted workspace changes, then rerun After.",
        ),
        "source_changed" => (
            "SourceChanged",
            "The source repository changed after verification; create a fresh proof.",
        ),
        "patch_changed" => (
            "PatchChanged",
            "The current patch bytes differ from the verified patch; rerun After.",
        ),
        "criterion_changed" => (
            "CriterionChanged",
            "The verification command or criterion changed; rerun Before and After.",
        ),
        "regressions_pending" => (
            "RegressionsPending",
            "Every required regression must pass for this exact patch.",
        ),
        _ => (
            "Inconclusive",
            "A complete verified proof is not available.",
        ),
    }
}

pub fn status(conn: &Connection, session_id: &str) -> Result<VerificationStatus> {
    let steps = workflow::list_reproduction_steps(conn, session_id)
        .map_err(|_| VerificationError::ProofMismatch)?;
    let proof = proof(conn, session_id)?;
    let handoff = handoff(conn, session_id)?;
    let regressions = list_regressions(conn, session_id)?;
    let mut reason = "baseline_missing";
    let mut current_identity = None;
    if let Some(step) = steps.first() {
        let before: Option<String> = conn
            .query_row(
                "SELECT status FROM reproduction_runs WHERE step_id=?1 AND phase='Before' AND cycle=?2 ORDER BY created_at DESC,rowid DESC LIMIT 1",
                rusqlite::params![step.id,step.active_cycle],
                |row| row.get(0),
            )
            .optional()?;
        let after: Option<String> = conn
            .query_row(
                "SELECT status FROM reproduction_runs WHERE step_id=?1 AND phase='After' AND cycle=?2 ORDER BY created_at DESC,rowid DESC LIMIT 1",
                rusqlite::params![step.id,step.active_cycle],
                |row| row.get(0),
            )
            .optional()?;
        reason = match before.as_deref() {
            None => "baseline_missing",
            Some("Passed") => "baseline_passed",
            Some("Failed") => match after.as_deref() {
                None => "after_missing",
                Some("Failed" | "Error" | "Interrupted") => "after_failed",
                Some("Passed") => {
                    let Some(stored) = proof.as_ref() else {
                        return build_status("patch_changed", None, proof, handoff, regressions);
                    };
                    match shadow_session::current_patch_identity(conn, session_id) {
                        Err(ShadowSessionError::UncommittedChanges) => "uncommitted_changes",
                        Err(ShadowSessionError::SourceCommitChanged) => "source_changed",
                        Err(_) => "patch_changed",
                        Ok(identity) => {
                            current_identity = Some(identity.clone());
                            if stored.cycle != step.active_cycle
                                || stored.criterion_sha256 != criterion_sha256(step)?
                                || stored.command_sha256 != command_sha256(step)?
                            {
                                "criterion_changed"
                            } else if stored.identity != identity {
                                if stored.identity.source_commit != identity.source_commit
                                    || stored.identity.source_state_sha256
                                        != identity.source_state_sha256
                                {
                                    "source_changed"
                                } else {
                                    "patch_changed"
                                }
                            } else {
                                let required_ok = regressions
                                    .iter()
                                    .filter(|item| item.level == RegressionLevel::Required)
                                    .all(|item| {
                                        item.status == "Passed"
                                            && item.verified_patch_sha256.as_deref()
                                                == Some(&identity.patch_sha256)
                                    });
                                if required_ok {
                                    "ready"
                                } else {
                                    "regressions_pending"
                                }
                            }
                        }
                    }
                }
                _ => "after_missing",
            },
            _ => "baseline_missing",
        };
    }
    build_status(reason, current_identity, proof, handoff, regressions)
}

fn build_status(
    reason: &str,
    current_identity: Option<PatchIdentity>,
    proof: Option<VerificationProof>,
    handoff: Option<VerificationHandoff>,
    regressions: Vec<RegressionCheck>,
) -> Result<VerificationStatus> {
    let (outcome, message) = status_message(reason);
    let required = regressions
        .iter()
        .filter(|item| item.level == RegressionLevel::Required)
        .collect::<Vec<_>>();
    let required_total = required.len();
    let required_passed = required
        .iter()
        .filter(|item| {
            item.status == "Passed"
                && current_identity.as_ref().is_some_and(|identity| {
                    item.verified_patch_sha256.as_deref() == Some(&identity.patch_sha256)
                })
        })
        .count();
    Ok(VerificationStatus {
        outcome: outcome.to_owned(),
        ready_to_apply: reason == "ready",
        reason_code: reason.to_owned(),
        message: message.to_owned(),
        current_identity,
        proof,
        handoff,
        regressions,
        required_passed,
        required_total,
    })
}

pub fn promote_regression(
    conn: &Connection,
    check_id: &str,
    level: RegressionLevel,
) -> Result<RegressionCheck> {
    let existing = conn
        .query_row(
            "SELECT id,session_id,stable_id,title,executable,args_json,expected_exit_code,level,status,receipt_id,verified_patch_sha256,created_at,updated_at FROM regression_checks WHERE id=?1",
            rusqlite::params![check_id],
            regression_from_row,
        )
        .optional()?
        .ok_or_else(|| VerificationError::RegressionNotFound(check_id.to_owned()))?;
    let rank = |value: RegressionLevel| match value {
        RegressionLevel::Required => 0,
        RegressionLevel::Recommended => 1,
        RegressionLevel::Optional => 2,
    };
    if rank(level) > rank(existing.level) {
        return Err(VerificationError::RegressionDemotion);
    }
    conn.execute(
        "UPDATE regression_checks SET level=?1,updated_at=?2 WHERE id=?3",
        rusqlite::params![level.to_string(), unix_time_secs()?, check_id],
    )?;
    Ok(conn.query_row(
        "SELECT id,session_id,stable_id,title,executable,args_json,expected_exit_code,level,status,receipt_id,verified_patch_sha256,created_at,updated_at FROM regression_checks WHERE id=?1",
        rusqlite::params![check_id],
        regression_from_row,
    )?)
}

pub fn run_regression(
    conn: &mut Connection,
    artifact_store: &Path,
    check_id: &str,
    explicitly_approved_once: bool,
) -> Result<RegressionRunOutcome> {
    let check = conn
        .query_row(
            "SELECT id,session_id,stable_id,title,executable,args_json,expected_exit_code,level,status,receipt_id,verified_patch_sha256,created_at,updated_at FROM regression_checks WHERE id=?1",
            rusqlite::params![check_id],
            regression_from_row,
        )
        .optional()?
        .ok_or_else(|| VerificationError::RegressionNotFound(check_id.to_owned()))?;
    let (_proof, identity) = primary_proof_matches(conn, &check.session_id)?;
    let shadow = shadow_session::get_session_shadow(conn, &check.session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(check.session_id.clone()))?;
    let decision = permissions::command_permission(
        &check.executable,
        &check.args,
        Permission::Ask,
        explicitly_approved_once,
        false,
    );
    match decision.permission {
        Permission::Deny => return Err(VerificationError::PermissionDenied(decision.explanation)),
        Permission::Ask => return Err(VerificationError::ApprovalRequired(decision.explanation)),
        Permission::Allow => {}
    }
    let meta = serde_json::json!({
        "check_id": check.id,
        "level": check.level,
        "patch_sha256": identity.patch_sha256,
        "command": { "executable": check.executable, "args": check.args },
        "expected_exit_code": check.expected_exit_code,
    });
    let action = timeline::new_action(
        &check.session_id,
        "verification:regression",
        "Running",
        Some(meta.to_string()),
    )?;
    timeline::create_action(conn, &action)?;
    let execution_id = timeline::start_execution(conn, &action.id)?;
    let (executable, args) = workflow::normalized_command(&check.executable, &check.args);
    let result = runner::run_command(
        CommandSpec {
            executable,
            args,
            cwd: Some(PathBuf::from(shadow.worktree_path)),
            env: None,
            clear_env: false,
            timeout: Some(Duration::from_secs(10 * 60)),
            output_limit: Some(10 * 1024 * 1024),
        },
        Permission::Allow,
        None,
    );
    let now = unix_time_secs()?;
    let (run_status, stdout, stderr) = match result {
        Ok(result) => (
            if result.exit_code == Some(check.expected_exit_code) {
                "Passed"
            } else {
                "Failed"
            },
            String::from_utf8_lossy(&result.stdout).into_owned(),
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ),
        Err(error) => ("Error", String::new(), error.to_string()),
    };
    let receipt_id = timeline::finish_execution(
        conn,
        &execution_id,
        run_status,
        Some(&stdout),
        Some(&stderr),
    )?;
    conn.execute(
        "UPDATE actions SET state=?1 WHERE id=?2",
        rusqlite::params![run_status, action.id],
    )?;
    if !stdout.is_empty() {
        evidence::persist_text_artifact(
            conn,
            artifact_store,
            &receipt_id,
            &stdout,
            Some("text/plain; stream=stdout"),
        )?;
    }
    if !stderr.is_empty() {
        evidence::persist_text_artifact(
            conn,
            artifact_store,
            &receipt_id,
            &stderr,
            Some("text/plain; stream=stderr"),
        )?;
    }
    conn.execute(
        "UPDATE regression_checks SET status=?1,receipt_id=?2,verified_patch_sha256=?3,updated_at=?4 WHERE id=?5",
        rusqlite::params![run_status,receipt_id,identity.patch_sha256,now,check_id],
    )?;
    let updated = conn.query_row(
        "SELECT id,session_id,stable_id,title,executable,args_json,expected_exit_code,level,status,receipt_id,verified_patch_sha256,created_at,updated_at FROM regression_checks WHERE id=?1",
        rusqlite::params![check_id],
        regression_from_row,
    )?;
    if status(conn, &check.session_id)?.ready_to_apply {
        let current: SessionState = timeline::get_session_record(conn, &check.session_id)?
            .ok_or_else(|| VerificationError::ProofMismatch)?
            .state
            .parse()?;
        if current == SessionState::Verified {
            state_machine::transition_session(conn, &check.session_id, SessionState::ReadyToApply)?;
        }
    }
    Ok(RegressionRunOutcome {
        check: updated,
        permission: decision,
    })
}

pub fn mark_verified_state(conn: &Connection, session_id: &str) -> Result<()> {
    state_machine::transition_session(conn, session_id, SessionState::Verified)?;
    if status(conn, session_id)?.ready_to_apply {
        state_machine::transition_session(conn, session_id, SessionState::ReadyToApply)?;
    }
    Ok(())
}

pub fn apply_verified(conn: &Connection, session_id: &str) -> Result<()> {
    let status = status(conn, session_id)?;
    if !status.ready_to_apply {
        return Err(VerificationError::ProofMismatch);
    }
    let identity = status
        .current_identity
        .ok_or(VerificationError::ProofMismatch)?;
    shadow_session::apply_verified_session_shadow(
        conn,
        session_id,
        &identity.patch_sha256,
        &identity.source_state_sha256,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::init_db, repository, shadow_session, workflow};
    use std::fs;
    use tempfile::TempDir;

    fn git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct Fixture {
        conn: Connection,
        repo: TempDir,
        artifacts: TempDir,
        _db: TempDir,
        step: ReproductionStep,
        worktree: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = shadow_session::discard_session_shadow(&self.conn, "session");
        }
    }

    fn verified_fixture() -> Fixture {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            repo.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(repo.path(), &["config", "core.autocrlf", "false"]);
        fs::write(repo.path().join("tracked.txt"), "base\n").unwrap();
        git(repo.path(), &["add", "tracked.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        let base = String::from_utf8(
            std::process::Command::new("git")
                .current_dir(repo.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();

        let db = tempfile::tempdir().unwrap();
        let mut conn = init_db(&db.path().join("proof.db")).unwrap();
        workflow::create_bug_session(&conn, "session", &workflow::SessionMeta::default()).unwrap();
        repository::attach_repository_to_session(&mut conn, "session", repo.path()).unwrap();
        state_machine::transition_session(&conn, "session", SessionState::Preparing).unwrap();
        state_machine::transition_session(&conn, "session", SessionState::CreatingWorkspace)
            .unwrap();
        let shadow = shadow_session::create_session_shadow(&conn, "session").unwrap();
        state_machine::transition_session(&conn, "session", SessionState::Ready).unwrap();
        let step = workflow::add_reproduction_step(
            &conn,
            "session",
            "git",
            &["diff".into(), "--exit-code".into(), base, "HEAD".into()],
            1,
        )
        .unwrap();
        let artifacts = tempfile::tempdir().unwrap();
        let before = workflow::execute_reproduction_step(
            &mut conn,
            artifacts.path(),
            &step.id,
            workflow::ReproductionPhase::Before,
            true,
        )
        .unwrap();
        assert_eq!(before.run.status, "Failed");
        fs::write(
            Path::new(&shadow.worktree_path).join("tracked.txt"),
            "fixed\n",
        )
        .unwrap();
        shadow_session::finalize_session_shadow(&conn, "session").unwrap();
        state_machine::transition_session(&conn, "session", SessionState::Fixing).unwrap();
        let after = workflow::execute_reproduction_step(
            &mut conn,
            artifacts.path(),
            &step.id,
            workflow::ReproductionPhase::After,
            true,
        )
        .unwrap();
        assert_eq!(after.run.status, "Passed");
        assert!(status(&conn, "session").unwrap().ready_to_apply);
        Fixture {
            conn,
            repo,
            artifacts,
            _db: db,
            step,
            worktree: PathBuf::from(shadow.worktree_path),
        }
    }

    #[test]
    fn after_pass_then_new_checkpoint_blocks_apply() {
        let fixture = verified_fixture();
        fs::write(fixture.worktree.join("second.txt"), "second\n").unwrap();
        shadow_session::finalize_session_shadow(&fixture.conn, "session").unwrap();
        let result = status(&fixture.conn, "session").unwrap();
        assert_eq!(result.reason_code, "patch_changed");
        assert!(apply_verified(&fixture.conn, "session").is_err());
    }

    #[test]
    fn after_pass_then_uncommitted_change_blocks_apply() {
        let fixture = verified_fixture();
        fs::write(fixture.worktree.join("tracked.txt"), "uncommitted\n").unwrap();
        let result = status(&fixture.conn, "session").unwrap();
        assert_eq!(result.reason_code, "uncommitted_changes");
        assert!(apply_verified(&fixture.conn, "session").is_err());
    }

    #[test]
    fn after_pass_then_added_file_blocks_apply() {
        let fixture = verified_fixture();
        fs::write(fixture.worktree.join("added.txt"), "new\n").unwrap();
        assert_eq!(
            status(&fixture.conn, "session").unwrap().reason_code,
            "uncommitted_changes"
        );
    }

    #[test]
    fn after_pass_then_deleted_file_blocks_apply() {
        let fixture = verified_fixture();
        fs::remove_file(fixture.worktree.join("tracked.txt")).unwrap();
        assert_eq!(
            status(&fixture.conn, "session").unwrap().reason_code,
            "uncommitted_changes"
        );
    }

    #[test]
    fn verified_patch_hash_must_match_current_apply_patch() {
        let fixture = verified_fixture();
        fixture
            .conn
            .execute(
                "UPDATE verification_proofs SET patch_sha256='00' WHERE session_id='session'",
                [],
            )
            .unwrap();
        assert_eq!(
            status(&fixture.conn, "session").unwrap().reason_code,
            "patch_changed"
        );
        assert!(apply_verified(&fixture.conn, "session").is_err());
        assert_eq!(
            fs::read_to_string(fixture.repo.path().join("tracked.txt")).unwrap(),
            "base\n"
        );
    }

    #[test]
    fn rerun_after_changed_patch_can_restore_ready_to_apply() {
        let mut fixture = verified_fixture();
        fs::write(fixture.worktree.join("second.txt"), "second\n").unwrap();
        shadow_session::finalize_session_shadow(&fixture.conn, "session").unwrap();
        assert!(!status(&fixture.conn, "session").unwrap().ready_to_apply);
        let after = workflow::execute_reproduction_step(
            &mut fixture.conn,
            fixture.artifacts.path(),
            &fixture.step.id,
            workflow::ReproductionPhase::After,
            true,
        )
        .unwrap();
        assert_eq!(after.run.status, "Passed");
        assert!(status(&fixture.conn, "session").unwrap().ready_to_apply);
    }

    #[test]
    fn source_commit_change_blocks_apply() {
        let fixture = verified_fixture();
        fs::write(fixture.repo.path().join("source-moved.txt"), "move\n").unwrap();
        git(fixture.repo.path(), &["add", "source-moved.txt"]);
        git(fixture.repo.path(), &["commit", "-m", "move source"]);
        let result = status(&fixture.conn, "session").unwrap();
        assert_eq!(result.reason_code, "source_changed");
        assert!(apply_verified(&fixture.conn, "session").is_err());
    }

    #[test]
    fn investigation_patch_is_transferred_after_before_and_required_regression_gates_apply() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            repo.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(repo.path(), &["config", "core.autocrlf", "false"]);
        fs::write(repo.path().join("tracked.txt"), "base\n").unwrap();
        git(repo.path(), &["add", "tracked.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        let investigation = crate::git_shadow::Shadow::create(repo.path(), None).unwrap();
        fs::write(investigation.worktree.join("tracked.txt"), "fixed\n").unwrap();
        investigation.commit_all("investigation fix").unwrap();
        let patch = investigation.prepare_patch_bytes().unwrap();
        let base = investigation.base_commit.clone();

        let db = tempfile::tempdir().unwrap();
        let mut conn = init_db(&db.path().join("handoff.db")).unwrap();
        workflow::create_bug_session(&conn, "session", &workflow::SessionMeta::default()).unwrap();
        repository::attach_repository_to_session(&mut conn, "session", repo.path()).unwrap();
        state_machine::transition_session(&conn, "session", SessionState::Preparing).unwrap();
        state_machine::transition_session(&conn, "session", SessionState::CreatingWorkspace)
            .unwrap();
        let verification_shadow = shadow_session::create_session_shadow(&conn, "session").unwrap();
        state_machine::transition_session(&conn, "session", SessionState::Ready).unwrap();
        let step = workflow::add_reproduction_step(
            &conn,
            "session",
            "git",
            &[
                "diff".into(),
                "--exit-code".into(),
                base.clone(),
                "HEAD".into(),
            ],
            1,
        )
        .unwrap();
        let staged = stage_handoff(
            &conn,
            "session",
            HandoffCandidate {
                investigation_case_id: "case".into(),
                hypothesis_id: "hypothesis".into(),
                experiment_id: "experiment".into(),
                source_commit: base.clone(),
                patch,
                files: vec!["tracked.txt".into()],
            },
            &[RegressionDraft {
                stable_id: "regression".into(),
                title: "Exact diff regression".into(),
                executable: "git".into(),
                args: vec!["diff".into(), "--exit-code".into(), base, "HEAD".into()],
                expected_exit_code: 1,
                level: RegressionLevel::Required,
            }],
        )
        .unwrap();
        assert!(staged.activated_at.is_none());
        assert_eq!(
            fs::read_to_string(Path::new(&verification_shadow.worktree_path).join("tracked.txt"))
                .unwrap(),
            "base\n"
        );

        let artifacts = tempfile::tempdir().unwrap();
        let before = workflow::execute_reproduction_step(
            &mut conn,
            artifacts.path(),
            &step.id,
            workflow::ReproductionPhase::Before,
            true,
        )
        .unwrap();
        assert_eq!(before.run.status, "Failed");
        assert!(handoff(&conn, "session")
            .unwrap()
            .unwrap()
            .activated_at
            .is_some());
        assert_eq!(
            fs::read_to_string(Path::new(&verification_shadow.worktree_path).join("tracked.txt"))
                .unwrap(),
            "fixed\n"
        );
        assert_eq!(
            fs::read_to_string(repo.path().join("tracked.txt")).unwrap(),
            "base\n"
        );

        let after = workflow::execute_reproduction_step(
            &mut conn,
            artifacts.path(),
            &step.id,
            workflow::ReproductionPhase::After,
            true,
        )
        .unwrap();
        assert_eq!(after.run.status, "Passed");
        assert_eq!(
            status(&conn, "session").unwrap().reason_code,
            "regressions_pending"
        );
        let check = list_regressions(&conn, "session").unwrap().remove(0);
        run_regression(&mut conn, artifacts.path(), &check.id, true).unwrap();
        assert!(status(&conn, "session").unwrap().ready_to_apply);

        shadow_session::discard_session_shadow(&conn, "session").unwrap();
        investigation.discard().unwrap();
    }

    #[test]
    fn invalid_handoff_is_rejected_without_touching_original() {
        let fixture = verified_fixture();
        let original = fs::read_to_string(fixture.repo.path().join("tracked.txt")).unwrap();
        let result = shadow_session::check_patch_against_session(
            &fixture.conn,
            "session",
            b"not a git patch",
        );
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(fixture.repo.path().join("tracked.txt")).unwrap(),
            original
        );
    }
}
