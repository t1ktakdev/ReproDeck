use crate::bug_hunter::FailureCluster;
use crate::context_compiler::{self, ContextPacket, ContextRequest};
use crate::git_shadow::{GitShadowError, Shadow};
use crate::project_health::{
    self, HealthCheckResult, HealthCheckStatus, HealthRunOptions, ProjectHealthReport,
};
use crate::project_intelligence::{ProjectCommand, ProjectCommandKind, ProjectProfile};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

const CASE_SCHEMA_VERSION: u32 = 2;
const MAX_HYPOTHESES: usize = 3;
const DEFAULT_EXPERIMENT_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Error)]
pub enum RootCauseError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error(transparent)]
    Shadow(#[from] GitShadowError),
    #[error("investigation case not found: {0}")]
    CaseNotFound(String),
    #[error("failure cluster not found: {0}")]
    ClusterNotFound(String),
    #[error("the selected failure has no deterministic reproduction criterion")]
    CriterionMissing,
    #[error("the project is outside its Git repository")]
    ProjectOutsideRepository,
    #[error("the source HEAD moved; create the fix workspace to inspect the recorded base commit safely")]
    StaleSource,
    #[error("focused source evidence must be captured before the Fix Workspace contains an intervention")]
    ContextAfterIntervention,
    #[error("fix workspace not found for investigation case: {0}")]
    WorkspaceNotFound(String),
    #[error("fix workspace is stale or cannot be restored safely")]
    StaleWorkspace,
    #[error("checkpoint requires uncommitted changes in the fix workspace")]
    NoChanges,
    #[error("review and experiment require a checkpointed intervention")]
    CheckpointRequired,
    #[error("hypothesis not found: {0}")]
    HypothesisNotFound(String),
    #[error("at most {MAX_HYPOTHESES} hypotheses are accepted for one investigation case")]
    TooManyHypotheses,
    #[error("hypothesis statement, falsifier and next experiment must all be non-empty")]
    InvalidHypothesis,
    #[error("evidence {0} has more than one relationship to the same hypothesis")]
    EvidenceClassificationConflict(String),
    #[error("hypotheses cannot be replaced after experiments have been recorded")]
    ExperimentsAlreadyRecorded,
    #[error("causal experiments require explicit confirmation")]
    ConfirmationRequired,
    #[error("the recorded baseline did not fail deterministically, so a causal experiment cannot support a root-cause hypothesis")]
    BaselineNotFailed,
    #[error("project command could not be executed: {0}")]
    Command(String),
}

pub type Result<T> = std::result::Result<T, RootCauseError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvestigationState {
    HypothesisRequired,
    ExperimentRequired,
    HypothesisSupported,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HypothesisStatus {
    Proposed,
    Supported,
    Contradicted,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HypothesisSource {
    Manual,
    Model,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExperimentConclusion {
    SupportsHypothesis,
    DoesNotSupport,
    Inconclusive,
    OriginalChanged,
    WorkspaceMutatedByCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationCriterion {
    pub command_id: String,
    pub label: String,
    pub kind: ProjectCommandKind,
    pub executable: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    pub baseline_status: HealthCheckStatus,
    pub baseline_exit_code: Option<i32>,
    pub baseline_evidence_id: String,
    #[serde(default)]
    pub baseline_summary: String,
    #[serde(default)]
    pub baseline_stdout_preview: String,
    #[serde(default)]
    pub baseline_stderr_preview: String,
    #[serde(default)]
    pub baseline_duration_ms: u64,
    #[serde(default)]
    pub baseline_finished_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceEvidenceRef {
    pub id: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub checksum: String,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub excerpt: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HypothesisDraft {
    pub statement: String,
    pub rationale: String,
    pub supporting_evidence_ids: Vec<String>,
    #[serde(default)]
    pub neutral_evidence_ids: Vec<String>,
    pub contradicting_evidence_ids: Vec<String>,
    pub falsifier: String,
    pub next_experiment: String,
    pub confidence_percent: u8,
    pub source: HypothesisSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationHypothesis {
    pub id: String,
    pub statement: String,
    pub rationale: String,
    pub supporting_evidence_ids: Vec<String>,
    #[serde(default)]
    pub neutral_evidence_ids: Vec<String>,
    pub contradicting_evidence_ids: Vec<String>,
    pub rejected_evidence_ids: Vec<String>,
    pub falsifier: String,
    pub next_experiment: String,
    pub requested_confidence_percent: u8,
    pub accepted_confidence_percent: u8,
    pub source: HypothesisSource,
    pub status: HypothesisStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalExperimentRecord {
    pub id: String,
    pub hypothesis_id: String,
    pub command_id: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub intervention_sha256: String,
    pub changed_files: Vec<String>,
    pub exit_code: Option<i32>,
    pub status: HealthCheckStatus,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub evidence_id: String,
    pub original_unchanged: bool,
    pub workspace_unchanged_by_command: bool,
    pub conclusion: ExperimentConclusion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationCase {
    pub schema_version: u32,
    pub id: String,
    pub root_path: String,
    pub repo_root: String,
    pub project_relative_path: String,
    pub project_name: String,
    pub health_run_id: String,
    pub cluster: FailureCluster,
    pub base_commit: String,
    pub state: InvestigationState,
    pub criterion: InvestigationCriterion,
    pub evidence_ids: Vec<String>,
    pub source_evidence: Vec<SourceEvidenceRef>,
    pub hypotheses: Vec<InvestigationHypothesis>,
    pub experiments: Vec<CausalExperimentRecord>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixWorkspaceRecord {
    pub case_id: String,
    pub repo_root: String,
    pub project_path: String,
    pub base_commit: String,
    pub branch: String,
    pub worktree_path: String,
    pub original_head: String,
    pub original_branch: String,
    pub dirty: bool,
    pub changed_files: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixWorkspaceDiff {
    pub patch: String,
    pub files: Vec<String>,
}

#[derive(Debug)]
struct StoredWorkspaceRow {
    repo_root: String,
    project_relative_path: String,
    base_commit: String,
    branch: String,
    worktree_path: String,
    original_head: String,
    original_branch: String,
    created_at: i64,
    updated_at: i64,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

#[cfg(test)]
fn canonical_string(path: &Path) -> Result<String> {
    Ok(path.canonicalize()?.to_string_lossy().into_owned())
}

fn current_head(repo: &Path) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()?;
    if !output.status.success() {
        return Err(RootCauseError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn project_relative_path(profile: &ProjectProfile) -> Result<(String, String)> {
    let git = profile
        .git
        .as_ref()
        .ok_or(RootCauseError::CriterionMissing)?;
    let repo = Path::new(&git.root_path).canonicalize()?;
    let project = Path::new(&profile.root_path).canonicalize()?;
    let relative = project
        .strip_prefix(&repo)
        .map_err(|_| RootCauseError::ProjectOutsideRepository)?;
    Ok((
        repo.to_string_lossy().into_owned(),
        relative.to_string_lossy().into_owned(),
    ))
}

fn is_failed_baseline(status: HealthCheckStatus) -> bool {
    status == HealthCheckStatus::Failed
}

fn criterion_for(
    report: &ProjectHealthReport,
    cluster: &FailureCluster,
) -> Option<InvestigationCriterion> {
    cluster.command_ids.iter().find_map(|command_id| {
        report
            .checks
            .iter()
            .find(|check| {
                &check.command_id == command_id
                    && matches!(
                        check.status,
                        HealthCheckStatus::Failed
                            | HealthCheckStatus::TimedOut
                            | HealthCheckStatus::Error
                    )
            })
            .map(|check| InvestigationCriterion {
                command_id: check.command_id.clone(),
                label: check.label.clone(),
                kind: check.kind,
                executable: check.executable.clone(),
                args: check.args.clone(),
                expected_exit_code: 0,
                baseline_status: check.status,
                baseline_exit_code: check.exit_code,
                baseline_evidence_id: check.evidence_id.clone(),
                baseline_summary: check.summary.clone(),
                baseline_stdout_preview: check.stdout_preview.clone(),
                baseline_stderr_preview: check.stderr_preview.clone(),
                baseline_duration_ms: check.duration_ms,
                baseline_finished_at: report.finished_at,
            })
    })
}

fn persist_case(conn: &Connection, case: &InvestigationCase) -> Result<()> {
    conn.execute(
        "INSERT INTO investigation_cases(id,root_path,repo_root,project_relative_path,project_name,health_run_id,cluster_id,base_commit,state,case_json,created_at,updated_at)\n         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)\n         ON CONFLICT(id) DO UPDATE SET state=excluded.state,case_json=excluded.case_json,updated_at=excluded.updated_at",
        rusqlite::params![
            case.id,
            case.root_path,
            case.repo_root,
            case.project_relative_path,
            case.project_name,
            case.health_run_id,
            case.cluster.id,
            case.base_commit,
            format!("{:?}", case.state),
            serde_json::to_string(case)?,
            case.created_at,
            case.updated_at,
        ],
    )?;
    Ok(())
}

pub fn create_case(
    conn: &Connection,
    profile: &ProjectProfile,
    report: &ProjectHealthReport,
    cluster: &FailureCluster,
) -> Result<InvestigationCase> {
    if cluster.id.trim().is_empty()
        || cluster.check_ids.is_empty()
        || !cluster
            .check_ids
            .iter()
            .all(|check_id| report.checks.iter().any(|check| &check.id == check_id))
    {
        return Err(RootCauseError::ClusterNotFound(cluster.id.clone()));
    }
    if let Some(existing) = conn
        .query_row(
            "SELECT case_json FROM investigation_cases WHERE health_run_id=?1 AND cluster_id=?2",
            rusqlite::params![report.id, cluster.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(serde_json::from_str(&existing)?);
    }

    let criterion = criterion_for(report, cluster).ok_or(RootCauseError::CriterionMissing)?;
    let (repo_root, project_relative_path) = project_relative_path(profile)?;
    let now = unix_time_secs()?;
    let mut evidence_ids = cluster.evidence_ids.clone();
    if !evidence_ids.contains(&criterion.baseline_evidence_id) {
        evidence_ids.push(criterion.baseline_evidence_id.clone());
    }
    evidence_ids.sort();
    evidence_ids.dedup();

    let case = InvestigationCase {
        schema_version: CASE_SCHEMA_VERSION,
        id: Uuid::new_v4().to_string(),
        root_path: profile.root_path.clone(),
        repo_root,
        project_relative_path,
        project_name: profile.name.clone(),
        health_run_id: report.id.clone(),
        cluster: cluster.clone(),
        base_commit: report.base_commit.clone(),
        state: InvestigationState::HypothesisRequired,
        criterion,
        evidence_ids,
        source_evidence: Vec::new(),
        hypotheses: Vec::new(),
        experiments: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    persist_case(conn, &case)?;
    Ok(case)
}

pub fn load_case(conn: &Connection, case_id: &str) -> Result<InvestigationCase> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT case_json FROM investigation_cases WHERE id=?1",
            rusqlite::params![case_id],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| serde_json::from_str(&value).map_err(RootCauseError::from))
        .transpose()?
        .ok_or_else(|| RootCauseError::CaseNotFound(case_id.to_owned()))
}

pub fn list_cases(
    conn: &Connection,
    root_path: &str,
    limit: usize,
) -> Result<Vec<InvestigationCase>> {
    let mut stmt = conn.prepare(
        "SELECT case_json FROM investigation_cases WHERE root_path=?1 ORDER BY updated_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![root_path, limit.clamp(1, 500) as i64],
        |row| row.get::<_, String>(0),
    )?;
    let mut cases = Vec::new();
    for row in rows {
        cases.push(serde_json::from_str(&row?)?);
    }
    Ok(cases)
}

fn stored_workspace(conn: &Connection, case_id: &str) -> Result<Option<FixWorkspaceRecord>> {
    let raw: Option<StoredWorkspaceRow> = conn
        .query_row(
            "SELECT repo_root,project_relative_path,base_commit,branch,worktree_path,original_head,original_branch,created_at,updated_at FROM investigation_workspaces WHERE case_id=?1",
            rusqlite::params![case_id],
            |row| {
                Ok(StoredWorkspaceRow {
                    repo_root: row.get(0)?,
                    project_relative_path: row.get(1)?,
                    base_commit: row.get(2)?,
                    branch: row.get(3)?,
                    worktree_path: row.get(4)?,
                    original_head: row.get(5)?,
                    original_branch: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let StoredWorkspaceRow {
        repo_root,
        project_relative_path: relative,
        base_commit,
        branch,
        worktree_path,
        original_head,
        original_branch,
        created_at,
        updated_at,
    } = raw;

    let worktree = PathBuf::from(&worktree_path);
    if !worktree.exists() {
        let repo = Path::new(&repo_root);
        let _ = Command::new("git")
            .current_dir(repo)
            .args(["worktree", "prune"])
            .output();
        let restored = Command::new("git")
            .current_dir(repo)
            .arg("worktree")
            .arg("add")
            .arg(&worktree)
            .arg(&branch)
            .output()?;
        if !restored.status.success() || !worktree.exists() {
            return Err(RootCauseError::StaleWorkspace);
        }
    }

    let shadow = Shadow {
        repo: PathBuf::from(&repo_root),
        worktree: worktree.clone(),
        branch: branch.clone(),
        base_commit: base_commit.clone(),
        original_head: original_head.clone(),
        original_branch: original_branch.clone(),
    };
    let dirty = shadow.has_uncommitted_changes()?;
    let changed_files = changed_files(&shadow)?;
    Ok(Some(FixWorkspaceRecord {
        case_id: case_id.to_owned(),
        repo_root,
        project_path: worktree.join(&relative).to_string_lossy().into_owned(),
        base_commit,
        branch,
        worktree_path,
        original_head,
        original_branch,
        dirty,
        changed_files,
        created_at,
        updated_at,
    }))
}

fn restored_shadow(record: &FixWorkspaceRecord) -> Shadow {
    Shadow {
        repo: PathBuf::from(&record.repo_root),
        worktree: PathBuf::from(&record.worktree_path),
        branch: record.branch.clone(),
        base_commit: record.base_commit.clone(),
        original_head: record.original_head.clone(),
        original_branch: record.original_branch.clone(),
    }
}

fn changed_files(shadow: &Shadow) -> Result<Vec<String>> {
    let bytes = shadow.diff_name_status_bytes()?;
    let parts = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut files = Vec::new();
    let mut index = 0usize;
    while index < parts.len() {
        let status = String::from_utf8_lossy(parts[index]);
        index += 1;
        let count = if status.starts_with('R') || status.starts_with('C') {
            2
        } else {
            1
        };
        for _ in 0..count {
            if index < parts.len() {
                files.push(String::from_utf8_lossy(parts[index]).into_owned());
                index += 1;
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

pub fn get_fix_workspace(conn: &Connection, case_id: &str) -> Result<Option<FixWorkspaceRecord>> {
    stored_workspace(conn, case_id)
}

pub fn create_fix_workspace(conn: &Connection, case_id: &str) -> Result<FixWorkspaceRecord> {
    if let Some(existing) = stored_workspace(conn, case_id)? {
        return Ok(existing);
    }
    let case = load_case(conn, case_id)?;
    let shadow = Shadow::create(Path::new(&case.repo_root), Some(&case.base_commit))?;
    let now = unix_time_secs()?;
    let insert = conn.execute(
        "INSERT INTO investigation_workspaces(case_id,repo_root,project_relative_path,base_commit,branch,worktree_path,original_head,original_branch,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![
            case.id,
            case.repo_root,
            case.project_relative_path,
            shadow.base_commit,
            shadow.branch,
            shadow.worktree.to_string_lossy().into_owned(),
            shadow.original_head,
            shadow.original_branch,
            now,
            now,
        ],
    );
    if let Err(error) = insert {
        let _ = shadow.discard();
        return Err(error.into());
    }
    stored_workspace(conn, case_id)?.ok_or(RootCauseError::StaleWorkspace)
}

pub fn checkpoint_fix_workspace(conn: &Connection, case_id: &str) -> Result<FixWorkspaceRecord> {
    let record = stored_workspace(conn, case_id)?
        .ok_or_else(|| RootCauseError::WorkspaceNotFound(case_id.to_owned()))?;
    let shadow = restored_shadow(&record);
    if !shadow.has_uncommitted_changes()? {
        return Err(RootCauseError::NoChanges);
    }
    shadow.commit_all("ReproDeck root-cause experiment checkpoint")?;
    conn.execute(
        "UPDATE investigation_workspaces SET updated_at=?1 WHERE case_id=?2",
        rusqlite::params![unix_time_secs()?, case_id],
    )?;
    stored_workspace(conn, case_id)?.ok_or(RootCauseError::StaleWorkspace)
}

pub fn fix_workspace_diff(conn: &Connection, case_id: &str) -> Result<FixWorkspaceDiff> {
    let record = stored_workspace(conn, case_id)?
        .ok_or_else(|| RootCauseError::WorkspaceNotFound(case_id.to_owned()))?;
    let shadow = restored_shadow(&record);
    if shadow.has_uncommitted_changes()? {
        return Err(RootCauseError::CheckpointRequired);
    }
    let patch = shadow.prepare_patch()?;
    if patch.is_empty() {
        return Err(RootCauseError::CheckpointRequired);
    }
    Ok(FixWorkspaceDiff {
        files: changed_files(&shadow)?,
        patch,
    })
}

/// Produce the exact checkpointed intervention for the protected verification
/// flow. Investigation workspaces intentionally have no Apply API; this value
/// can only be checked and copied into a separate session shadow workspace.
pub fn verification_handoff_candidate(
    conn: &Connection,
    case_id: &str,
    hypothesis_id: &str,
    experiment_id: &str,
) -> Result<crate::verification::HandoffCandidate> {
    let case = load_case(conn, case_id)?;
    let hypothesis = case
        .hypotheses
        .iter()
        .find(|value| value.id == hypothesis_id)
        .ok_or_else(|| RootCauseError::HypothesisNotFound(hypothesis_id.to_owned()))?;
    if hypothesis.status != HypothesisStatus::Supported {
        return Err(RootCauseError::BaselineNotFailed);
    }
    let experiment = case
        .experiments
        .iter()
        .find(|value| value.id == experiment_id && value.hypothesis_id == hypothesis_id)
        .ok_or_else(|| RootCauseError::HypothesisNotFound(experiment_id.to_owned()))?;
    if experiment.conclusion != ExperimentConclusion::SupportsHypothesis
        || !experiment.original_unchanged
        || !experiment.workspace_unchanged_by_command
    {
        return Err(RootCauseError::BaselineNotFailed);
    }
    let record = stored_workspace(conn, case_id)?
        .ok_or_else(|| RootCauseError::WorkspaceNotFound(case_id.to_owned()))?;
    let shadow = restored_shadow(&record);
    if shadow.has_uncommitted_changes()? {
        return Err(RootCauseError::CheckpointRequired);
    }
    let patch = shadow.prepare_patch_bytes()?;
    if patch.is_empty() {
        return Err(RootCauseError::CheckpointRequired);
    }
    let patch_sha256 = hex::encode(Sha256::digest(&patch));
    if patch_sha256 != experiment.intervention_sha256 {
        return Err(RootCauseError::CheckpointRequired);
    }
    Ok(crate::verification::HandoffCandidate {
        investigation_case_id: case.id,
        hypothesis_id: hypothesis_id.to_owned(),
        experiment_id: experiment_id.to_owned(),
        source_commit: case.base_commit,
        files: changed_files(&shadow)?,
        patch,
    })
}

pub fn discard_fix_workspace(conn: &Connection, case_id: &str) -> Result<()> {
    let record = stored_workspace(conn, case_id)?
        .ok_or_else(|| RootCauseError::WorkspaceNotFound(case_id.to_owned()))?;
    restored_shadow(&record).discard()?;
    conn.execute(
        "DELETE FROM investigation_workspaces WHERE case_id=?1",
        rusqlite::params![case_id],
    )?;
    Ok(())
}

pub fn compile_case_context(
    conn: &Connection,
    case_id: &str,
    max_files: usize,
    max_chars: usize,
) -> Result<ContextPacket> {
    let mut case = load_case(conn, case_id)?;
    let source_root = if let Some(workspace) = stored_workspace(conn, case_id)? {
        if workspace.dirty || !workspace.changed_files.is_empty() {
            return Err(RootCauseError::ContextAfterIntervention);
        }
        PathBuf::from(workspace.project_path)
    } else {
        if current_head(Path::new(&case.repo_root))? != case.base_commit {
            return Err(RootCauseError::StaleSource);
        }
        PathBuf::from(&case.root_path)
    };
    let request = ContextRequest::bounded(
        case.cluster.investigation_query.clone(),
        max_files.clamp(1, 24),
        max_chars.clamp(1_000, 96_000),
    );
    let packet = context_compiler::compile_context(&source_root, &request)
        .map_err(|error| RootCauseError::Command(error.to_string()))?;
    case.source_evidence = packet
        .snippets
        .iter()
        .map(|snippet| SourceEvidenceRef {
            id: snippet.id.clone(),
            path: snippet.path.clone(),
            line_start: snippet.line_start,
            line_end: snippet.line_end,
            checksum: snippet.checksum.clone(),
            reasons: snippet.reasons.clone(),
            language: snippet.language.clone(),
            score: snippet.score,
            excerpt: snippet.content.clone(),
            truncated: snippet.truncated,
        })
        .collect();
    for snippet in &case.source_evidence {
        if !case.evidence_ids.contains(&snippet.id) {
            case.evidence_ids.push(snippet.id.clone());
        }
    }
    case.evidence_ids.sort();
    case.evidence_ids.dedup();
    case.updated_at = unix_time_secs()?;
    persist_case(conn, &case)?;
    Ok(packet)
}

pub fn record_hypotheses(
    conn: &Connection,
    case_id: &str,
    drafts: Vec<HypothesisDraft>,
) -> Result<InvestigationCase> {
    if drafts.len() > MAX_HYPOTHESES {
        return Err(RootCauseError::TooManyHypotheses);
    }
    let mut case = load_case(conn, case_id)?;
    if !case.experiments.is_empty() {
        return Err(RootCauseError::ExperimentsAlreadyRecorded);
    }
    let known = case.evidence_ids.iter().cloned().collect::<HashSet<_>>();
    let mut hypotheses = Vec::with_capacity(drafts.len());

    for draft in drafts {
        if draft.statement.trim().is_empty()
            || draft.falsifier.trim().is_empty()
            || draft.next_experiment.trim().is_empty()
        {
            return Err(RootCauseError::InvalidHypothesis);
        }
        let mut rejected = Vec::new();
        let mut supporting = draft
            .supporting_evidence_ids
            .iter()
            .filter_map(|id| {
                if known.contains(id) {
                    Some(id.clone())
                } else {
                    rejected.push(id.clone());
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut neutral = draft
            .neutral_evidence_ids
            .iter()
            .filter_map(|id| {
                if known.contains(id) {
                    Some(id.clone())
                } else {
                    rejected.push(id.clone());
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut contradicting = draft
            .contradicting_evidence_ids
            .iter()
            .filter_map(|id| {
                if known.contains(id) {
                    Some(id.clone())
                } else {
                    rejected.push(id.clone());
                    None
                }
            })
            .collect::<Vec<_>>();
        supporting.sort();
        supporting.dedup();
        neutral.sort();
        neutral.dedup();
        contradicting.sort();
        contradicting.dedup();
        rejected.sort();
        rejected.dedup();

        let mut classified = HashSet::new();
        for id in supporting.iter().chain(&neutral).chain(&contradicting) {
            if !classified.insert(id.clone()) {
                return Err(RootCauseError::EvidenceClassificationConflict(id.clone()));
            }
        }

        let requested = draft.confidence_percent.min(100);
        let mut accepted = requested;
        if supporting.is_empty() {
            accepted = accepted.min(40);
        }
        if !rejected.is_empty() {
            accepted = accepted.min(25);
        }
        if !contradicting.is_empty() {
            accepted = accepted.min(60);
        }

        hypotheses.push(InvestigationHypothesis {
            id: Uuid::new_v4().to_string(),
            statement: draft.statement.trim().to_owned(),
            rationale: draft.rationale.trim().to_owned(),
            supporting_evidence_ids: supporting,
            neutral_evidence_ids: neutral,
            contradicting_evidence_ids: contradicting,
            rejected_evidence_ids: rejected,
            falsifier: draft.falsifier.trim().to_owned(),
            next_experiment: draft.next_experiment.trim().to_owned(),
            requested_confidence_percent: requested,
            accepted_confidence_percent: accepted,
            source: draft.source,
            status: HypothesisStatus::Proposed,
        });
    }

    case.hypotheses = hypotheses;
    case.state = if case.hypotheses.is_empty() {
        InvestigationState::HypothesisRequired
    } else {
        InvestigationState::ExperimentRequired
    };
    case.updated_at = unix_time_secs()?;
    persist_case(conn, &case)?;
    Ok(case)
}

fn criterion_command(criterion: &InvestigationCriterion) -> ProjectCommand {
    ProjectCommand {
        id: criterion.command_id.clone(),
        label: criterion.label.clone(),
        kind: criterion.kind,
        executable: criterion.executable.clone(),
        args: criterion.args.clone(),
        source: "investigation-case".into(),
        confidence: crate::project_intelligence::CommandConfidence::Declared,
    }
}

pub fn run_causal_experiment(
    conn: &Connection,
    case_id: &str,
    hypothesis_id: &str,
    timeout_secs: Option<u64>,
    confirmed_execution: bool,
) -> Result<InvestigationCase> {
    if !confirmed_execution {
        return Err(RootCauseError::ConfirmationRequired);
    }
    let mut case = load_case(conn, case_id)?;
    if !is_failed_baseline(case.criterion.baseline_status) {
        return Err(RootCauseError::BaselineNotFailed);
    }
    let hypothesis_index = case
        .hypotheses
        .iter()
        .position(|hypothesis| hypothesis.id == hypothesis_id)
        .ok_or_else(|| RootCauseError::HypothesisNotFound(hypothesis_id.to_owned()))?;
    let workspace = stored_workspace(conn, case_id)?
        .ok_or_else(|| RootCauseError::WorkspaceNotFound(case_id.to_owned()))?;
    let shadow = restored_shadow(&workspace);
    if shadow.has_uncommitted_changes()? {
        return Err(RootCauseError::CheckpointRequired);
    }
    let intervention = shadow.prepare_patch_bytes()?;
    if intervention.is_empty() {
        return Err(RootCauseError::CheckpointRequired);
    }

    let changed_files = changed_files(&shadow)?;
    let intervention_sha256 = hex::encode(Sha256::digest(&intervention));
    let original_before = project_health::git_snapshot(Path::new(&case.repo_root))
        .map_err(|error| RootCauseError::Command(error.to_string()))?;
    let workspace_before = project_health::git_snapshot(&shadow.worktree)
        .map_err(|error| RootCauseError::Command(error.to_string()))?;
    let started_at = unix_time_secs()?;
    let experiment_id = Uuid::new_v4().to_string();
    let command = criterion_command(&case.criterion);
    let options = HealthRunOptions {
        command_ids: vec![command.id.clone()],
        timeout_secs: timeout_secs.unwrap_or(DEFAULT_EXPERIMENT_TIMEOUT_SECS),
        confirmed_execution,
    };
    let cwd = PathBuf::from(&workspace.project_path);
    let mut check: HealthCheckResult = project_health::run_one_check(
        &format!("experiment:{experiment_id}"),
        &command,
        &cwd,
        &options,
    );
    let evidence_id = format!("experiment:{case_id}:{experiment_id}");
    check.evidence_id = evidence_id.clone();
    let finished_at = unix_time_secs()?;
    let original_after = project_health::git_snapshot(Path::new(&case.repo_root))
        .map_err(|error| RootCauseError::Command(error.to_string()))?;
    let workspace_after = project_health::git_snapshot(&shadow.worktree)
        .map_err(|error| RootCauseError::Command(error.to_string()))?;
    let original_unchanged = original_before == original_after;
    let workspace_unchanged_by_command = workspace_before == workspace_after;

    let conclusion = if !original_unchanged {
        ExperimentConclusion::OriginalChanged
    } else if !workspace_unchanged_by_command {
        ExperimentConclusion::WorkspaceMutatedByCommand
    } else if check.status == HealthCheckStatus::Passed
        && check.exit_code == Some(case.criterion.expected_exit_code)
    {
        ExperimentConclusion::SupportsHypothesis
    } else if matches!(
        check.status,
        HealthCheckStatus::Blocked | HealthCheckStatus::TimedOut | HealthCheckStatus::Error
    ) {
        ExperimentConclusion::Inconclusive
    } else {
        ExperimentConclusion::DoesNotSupport
    };

    match conclusion {
        ExperimentConclusion::SupportsHypothesis => {
            case.hypotheses[hypothesis_index].status = HypothesisStatus::Supported;
            case.state = InvestigationState::HypothesisSupported;
        }
        ExperimentConclusion::DoesNotSupport => {
            case.hypotheses[hypothesis_index].status = HypothesisStatus::Contradicted;
            case.state = InvestigationState::ExperimentRequired;
        }
        _ => {
            case.hypotheses[hypothesis_index].status = HypothesisStatus::Inconclusive;
            case.state = InvestigationState::ExperimentRequired;
        }
    }
    case.experiments.push(CausalExperimentRecord {
        id: experiment_id,
        hypothesis_id: hypothesis_id.to_owned(),
        command_id: check.command_id.clone(),
        started_at,
        finished_at,
        intervention_sha256,
        changed_files,
        exit_code: check.exit_code,
        status: check.status,
        stdout_preview: check.stdout_preview,
        stderr_preview: check.stderr_preview,
        evidence_id: evidence_id.clone(),
        original_unchanged,
        workspace_unchanged_by_command,
        conclusion,
    });
    if !case.evidence_ids.contains(&evidence_id) {
        case.evidence_ids.push(evidence_id);
    }
    case.updated_at = finished_at;
    persist_case(conn, &case)?;
    Ok(case)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bug_hunter::{FailureClass, InvestigationExperiment};
    use crate::db::init_db;
    use crate::project_intelligence::{CommandConfidence, ProjectGitState, ProjectStats};
    use tempfile::{tempdir, NamedTempFile};

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
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

    fn fixture() -> (
        tempfile::TempDir,
        NamedTempFile,
        Connection,
        ProjectProfile,
        ProjectHealthReport,
        FailureCluster,
    ) {
        let repo = tempdir().unwrap();
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            repo.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(repo.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(repo.path().join("state.txt"), "BAD\n").unwrap();
        git(repo.path(), &["add", "state.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        let root = canonical_string(repo.path()).unwrap();
        let head = current_head(repo.path()).unwrap();
        let command = ProjectCommand {
            id: "test:state".into(),
            label: "State criterion".into(),
            kind: ProjectCommandKind::Test,
            executable: "git".into(),
            args: vec![
                "grep".into(),
                "-q".into(),
                "GOOD".into(),
                "--".into(),
                "state.txt".into(),
            ],
            source: "fixture".into(),
            confidence: CommandConfidence::Declared,
        };
        let profile = ProjectProfile {
            schema_version: 1,
            fingerprint: "fixture".into(),
            root_path: root.clone(),
            name: "fixture".into(),
            version: None,
            description: None,
            analyzed_at: 0,
            git: Some(ProjectGitState {
                root_path: root.clone(),
                branch: "master".into(),
                head_commit: Some(head.clone()),
                is_dirty: false,
                changed_files: Vec::new(),
            }),
            languages: Vec::new(),
            technologies: Vec::new(),
            commands: vec![command.clone()],
            entrypoints: Vec::new(),
            test_paths: Vec::new(),
            documentation: Vec::new(),
            ci_files: Vec::new(),
            signals: Vec::new(),
            stats: ProjectStats {
                files_seen: 1,
                source_files: 0,
                test_files: 0,
                documentation_files: 0,
                sensitive_files_excluded: 0,
                skipped_large_files: 0,
                todo_markers: 0,
                scan_truncated: false,
            },
        };
        let check = HealthCheckResult {
            id: "check-1".into(),
            command_id: command.id.clone(),
            label: command.label.clone(),
            kind: command.kind,
            executable: command.executable.clone(),
            args: command.args.clone(),
            status: HealthCheckStatus::Failed,
            exit_code: Some(1),
            duration_ms: 1,
            stdout_preview: String::new(),
            stderr_preview: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            evidence_id: "health:run:check-1".into(),
            summary: "state is BAD".into(),
        };
        let report = ProjectHealthReport {
            id: "run-1".into(),
            root_path: root,
            project_name: "fixture".into(),
            base_commit: head,
            started_at: 1,
            finished_at: 2,
            status: crate::project_health::HealthRunStatus::ProblemsFound,
            original_unchanged: true,
            source_had_local_changes: false,
            checks: vec![check],
            problems: Vec::new(),
        };
        let cluster = FailureCluster {
            id: "cluster-1".into(),
            class: FailureClass::Test,
            signature: "state criterion".into(),
            title: "Test failure".into(),
            summary: "state is BAD".into(),
            check_ids: vec!["check-1".into()],
            command_ids: vec!["test:state".into()],
            evidence_ids: vec!["health:run:check-1".into()],
            related_problem_ids: Vec::new(),
            investigation_query: "why is state BAD?".into(),
            experiments: vec![InvestigationExperiment {
                order: 1,
                kind: "reproduce".into(),
                title: "Reproduce".into(),
                purpose: "same criterion".into(),
                command_id: Some("test:state".into()),
                requires_evidence: true,
            }],
        };
        let db = NamedTempFile::new().unwrap();
        let mut conn = init_db(db.path()).unwrap();
        crate::project_health::save_report(&mut conn, &report).unwrap();
        (repo, db, conn, profile, report, cluster)
    }

    #[test]
    fn case_persists_independently_of_later_health_state() {
        let (_repo, _db, conn, profile, report, cluster) = fixture();
        let created = create_case(&conn, &profile, &report, &cluster).unwrap();
        let loaded = load_case(&conn, &created.id).unwrap();
        assert_eq!(loaded.health_run_id, "run-1");
        assert_eq!(loaded.cluster.id, "cluster-1");
        assert_eq!(loaded.criterion.baseline_summary, "state is BAD");
        assert_eq!(loaded.criterion.baseline_duration_ms, 1);
        assert_eq!(loaded.criterion.baseline_finished_at, 2);
        assert_eq!(
            list_cases(&conn, &profile.root_path, 10).unwrap(),
            vec![created]
        );
    }

    #[test]
    fn unknown_model_evidence_is_rejected_and_confidence_is_capped() {
        let (_repo, _db, conn, profile, report, cluster) = fixture();
        let case = create_case(&conn, &profile, &report, &cluster).unwrap();
        let updated = record_hypotheses(
            &conn,
            &case.id,
            vec![HypothesisDraft {
                statement: "The state producer writes BAD".into(),
                rationale: "Observed failure".into(),
                supporting_evidence_ids: vec!["health:run:check-1".into(), "ctx:invented".into()],
                neutral_evidence_ids: Vec::new(),
                contradicting_evidence_ids: Vec::new(),
                falsifier: "The producer writes GOOD".into(),
                next_experiment: "Change only the producer output".into(),
                confidence_percent: 94,
                source: HypothesisSource::Model,
            }],
        )
        .unwrap();
        let hypothesis = &updated.hypotheses[0];
        assert_eq!(
            hypothesis.supporting_evidence_ids,
            vec!["health:run:check-1"]
        );
        assert_eq!(hypothesis.rejected_evidence_ids, vec!["ctx:invented"]);
        assert_eq!(hypothesis.accepted_confidence_percent, 25);
    }

    #[test]
    fn neutral_evidence_is_persisted_without_claiming_support() {
        let (_repo, _db, conn, profile, report, cluster) = fixture();
        let case = create_case(&conn, &profile, &report, &cluster).unwrap();
        let updated = record_hypotheses(
            &conn,
            &case.id,
            vec![HypothesisDraft {
                statement: "The producer may own the invalid state".into(),
                rationale: "The failure output identifies the state, not its producer".into(),
                supporting_evidence_ids: Vec::new(),
                neutral_evidence_ids: vec!["health:run:check-1".into()],
                contradicting_evidence_ids: Vec::new(),
                falsifier: "A different producer owns the value".into(),
                next_experiment: "Trace the value without changing behavior".into(),
                confidence_percent: 72,
                source: HypothesisSource::Manual,
            }],
        )
        .unwrap();
        let hypothesis = &updated.hypotheses[0];
        assert!(hypothesis.supporting_evidence_ids.is_empty());
        assert_eq!(hypothesis.neutral_evidence_ids, vec!["health:run:check-1"]);
        assert_eq!(hypothesis.accepted_confidence_percent, 40);
        assert_eq!(load_case(&conn, &case.id).unwrap(), updated);
    }

    #[test]
    fn evidence_cannot_have_conflicting_relationships() {
        let (_repo, _db, conn, profile, report, cluster) = fixture();
        let case = create_case(&conn, &profile, &report, &cluster).unwrap();
        let result = record_hypotheses(
            &conn,
            &case.id,
            vec![HypothesisDraft {
                statement: "The state producer writes BAD".into(),
                rationale: "Candidate only".into(),
                supporting_evidence_ids: vec!["health:run:check-1".into()],
                neutral_evidence_ids: vec!["health:run:check-1".into()],
                contradicting_evidence_ids: Vec::new(),
                falsifier: "The producer writes GOOD".into(),
                next_experiment: "Change only the producer output".into(),
                confidence_percent: 50,
                source: HypothesisSource::Model,
            }],
        );
        assert!(matches!(
            result,
            Err(RootCauseError::EvidenceClassificationConflict(id)) if id == "health:run:check-1"
        ));
    }

    #[test]
    fn causal_experiment_supports_hypothesis_without_touching_original() {
        let (repo, _db, conn, profile, report, cluster) = fixture();
        let case = create_case(&conn, &profile, &report, &cluster).unwrap();
        let case = record_hypotheses(
            &conn,
            &case.id,
            vec![HypothesisDraft {
                statement: "state.txt is the causal input".into(),
                rationale: "The reproduction checks that file".into(),
                supporting_evidence_ids: vec!["health:run:check-1".into()],
                neutral_evidence_ids: Vec::new(),
                contradicting_evidence_ids: Vec::new(),
                falsifier: "Changing only state.txt still fails".into(),
                next_experiment: "Change BAD to GOOD".into(),
                confidence_percent: 70,
                source: HypothesisSource::Manual,
            }],
        )
        .unwrap();
        let workspace = create_fix_workspace(&conn, &case.id).unwrap();
        std::fs::write(
            Path::new(&workspace.project_path).join("state.txt"),
            "GOOD\n",
        )
        .unwrap();
        checkpoint_fix_workspace(&conn, &case.id).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("state.txt")).unwrap(),
            "BAD\n"
        );
        let updated =
            run_causal_experiment(&conn, &case.id, &case.hypotheses[0].id, Some(30), true).unwrap();
        assert_eq!(updated.state, InvestigationState::HypothesisSupported);
        assert_eq!(
            updated.experiments[0].conclusion,
            ExperimentConclusion::SupportsHypothesis
        );
        assert!(updated.experiments[0].original_unchanged);
        assert!(updated.experiments[0].workspace_unchanged_by_command);
        assert_eq!(
            std::fs::read_to_string(repo.path().join("state.txt")).unwrap(),
            "BAD\n"
        );
        discard_fix_workspace(&conn, &case.id).unwrap();
    }

    #[test]
    fn no_apply_api_exists_for_investigation_workspaces() {
        let (_repo, _db, conn, profile, report, cluster) = fixture();
        let case = create_case(&conn, &profile, &report, &cluster).unwrap();
        let workspace = create_fix_workspace(&conn, &case.id).unwrap();
        std::fs::write(
            Path::new(&workspace.project_path).join("state.txt"),
            "GOOD\n",
        )
        .unwrap();
        checkpoint_fix_workspace(&conn, &case.id).unwrap();
        let diff = fix_workspace_diff(&conn, &case.id).unwrap();
        assert!(diff.patch.contains("GOOD"));
        assert_eq!(
            std::fs::read_to_string(Path::new(&case.root_path).join("state.txt")).unwrap(),
            "BAD\n"
        );
        discard_fix_workspace(&conn, &case.id).unwrap();
    }
}
