use crate::evidence::{self, ArtifactRecord};
use crate::permissions::{self, Permission, PermissionDecision};
use crate::redaction;
use crate::runner::{self, CommandError, CommandSpec};
use crate::shadow_session;
use crate::state_machine::{self, SessionState};
use crate::timeline;
use crate::verification;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Timeline(#[from] timeline::TimelineError),
    #[error(transparent)]
    Shadow(#[from] shadow_session::ShadowSessionError),
    #[error(transparent)]
    Evidence(#[from] evidence::EvidenceError),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error(transparent)]
    State(#[from] state_machine::StateError),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("reproduction step not found: {0}")]
    StepNotFound(String),
    #[error("command denied: {0}")]
    PermissionDenied(String),
    #[error("command requires approval: {0}")]
    ApprovalRequired(String),
    #[error("the current Before baseline is locked; reset it explicitly before replacing it")]
    BaselineLocked,
    #[error("there is no Before baseline to reset in the active verification cycle")]
    BaselineMissing,
    #[error(transparent)]
    Verification(#[from] verification::VerificationError),
}

pub type Result<T> = std::result::Result<T, WorkflowError>;

struct ReproductionStepRow {
    id: String,
    session_id: String,
    ordering: i64,
    executable: String,
    args_json: String,
    expected_exit_code: i32,
    active_cycle: i64,
    created_at: i64,
}

struct EnvironmentSnapshotRow {
    id: String,
    session_id: String,
    captured_at: i64,
    os: String,
    arch: String,
    git_version: Option<String>,
    runtimes_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionMeta {
    pub title: String,
    pub expected: String,
    pub actual: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionStep {
    pub id: String,
    pub session_id: String,
    pub ordering: i64,
    pub executable: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    pub active_cycle: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReproductionPhase {
    Before,
    After,
}
impl std::fmt::Display for ReproductionPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Before => write!(f, "Before"),
            Self::After => write!(f, "After"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionRun {
    pub id: String,
    pub step_id: String,
    pub phase: ReproductionPhase,
    pub action_id: String,
    pub receipt_id: Option<String>,
    pub exit_code: Option<i32>,
    pub status: String,
    pub cycle: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSnapshot {
    pub id: String,
    pub session_id: String,
    pub captured_at: i64,
    pub os: String,
    pub arch: String,
    pub git_version: Option<String>,
    pub runtimes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutcome {
    pub run: ReproductionRun,
    pub permission: PermissionDecision,
    pub stdout_artifact: Option<ArtifactRecord>,
    pub stderr_artifact: Option<ArtifactRecord>,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

pub fn create_bug_session(
    conn: &Connection,
    id: &str,
    meta: &SessionMeta,
) -> Result<timeline::SessionRecord> {
    timeline::create_session(conn, id, "Draft", Some(&serde_json::to_string(meta)?))?;
    timeline::get_session_record(conn, id)?
        .ok_or_else(|| WorkflowError::SessionNotFound(id.to_owned()))
}

pub fn session_meta(session: &timeline::SessionRecord) -> SessionMeta {
    session
        .meta
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or_default()
}

pub fn add_reproduction_step(
    conn: &Connection,
    session_id: &str,
    executable: &str,
    args: &[String],
    expected_exit_code: i32,
) -> Result<ReproductionStep> {
    if timeline::get_session_record(conn, session_id)?.is_none() {
        return Err(WorkflowError::SessionNotFound(session_id.to_owned()));
    }
    let ordering: i64 = conn.query_row(
        "SELECT COALESCE(MAX(ordering), -1) + 1 FROM reproduction_steps WHERE session_id=?1",
        rusqlite::params![session_id],
        |row| row.get(0),
    )?;
    let step = ReproductionStep {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_owned(),
        ordering,
        executable: executable.trim().to_owned(),
        args: args.to_vec(),
        expected_exit_code,
        active_cycle: 1,
        created_at: unix_time_secs()?,
    };
    conn.execute(
        "INSERT INTO reproduction_steps(id,session_id,ordering,executable,args_json,expected_exit_code,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![step.id,step.session_id,step.ordering,step.executable,serde_json::to_string(&step.args)?,step.expected_exit_code,step.created_at],
    )?;
    Ok(step)
}

fn step_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReproductionStepRow> {
    Ok(ReproductionStepRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        ordering: row.get(2)?,
        executable: row.get(3)?,
        args_json: row.get(4)?,
        expected_exit_code: row.get(5)?,
        active_cycle: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn decode_step(raw: ReproductionStepRow) -> Result<ReproductionStep> {
    Ok(ReproductionStep {
        id: raw.id,
        session_id: raw.session_id,
        ordering: raw.ordering,
        executable: raw.executable,
        args: serde_json::from_str(&raw.args_json)?,
        expected_exit_code: raw.expected_exit_code,
        active_cycle: raw.active_cycle,
        created_at: raw.created_at,
    })
}

pub fn list_reproduction_steps(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ReproductionStep>> {
    let mut stmt = conn.prepare("SELECT id,session_id,ordering,executable,args_json,expected_exit_code,active_cycle,created_at FROM reproduction_steps WHERE session_id=?1 ORDER BY ordering ASC")?;
    let raws = stmt
        .query_map(rusqlite::params![session_id], step_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    raws.into_iter().map(decode_step).collect()
}

pub fn get_reproduction_step(conn: &Connection, id: &str) -> Result<ReproductionStep> {
    let raw = conn.query_row(
        "SELECT id,session_id,ordering,executable,args_json,expected_exit_code,active_cycle,created_at FROM reproduction_steps WHERE id=?1",
        rusqlite::params![id], step_from_row,
    ).optional()?.ok_or_else(|| WorkflowError::StepNotFound(id.to_owned()))?;
    decode_step(raw)
}

#[cfg(windows)]
fn resolve_node_package_manager(
    executable: &str,
    args: &[String],
) -> Option<(String, Vec<String>)> {
    let name = Path::new(executable)
        .file_name()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    let cli_name = match name.as_str() {
        "npm" | "npm.cmd" => "npm-cli.js",
        "npx" | "npx.cmd" => "npx-cli.js",
        _ => return None,
    };

    // `CreateProcessW` cannot directly execute .cmd files. ReproDeck's Windows
    // runner deliberately avoids cmd.exe, so npm/npx are resolved to their
    // JavaScript entrypoint and executed with node.exe instead. This keeps
    // user arguments as argv rather than shell syntax.
    let lookup = if name.ends_with(".cmd") {
        name.clone()
    } else {
        format!("{name}.cmd")
    };
    let where_output = Command::new("where.exe").arg(&lookup).output().ok()?;
    if !where_output.status.success() {
        return None;
    }
    let command_path = String::from_utf8_lossy(&where_output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)?;
    let root = command_path.parent()?;
    let cli = root
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join(cli_name);
    if !cli.is_file() {
        return None;
    }

    let bundled_node = root.join("node.exe");
    let node = if bundled_node.is_file() {
        bundled_node
    } else {
        let output = Command::new("where.exe").arg("node.exe").output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(PathBuf::from)?
    };

    let mut normalized = Vec::with_capacity(args.len() + 1);
    normalized.push(cli.to_string_lossy().into_owned());
    normalized.extend(args.iter().cloned());
    Some((node.to_string_lossy().into_owned(), normalized))
}

pub(crate) fn normalized_command(executable: &str, args: &[String]) -> (String, Vec<String>) {
    #[cfg(windows)]
    if let Some(value) = resolve_node_package_manager(executable, args) {
        return value;
    }
    (executable.to_owned(), args.to_vec())
}

fn probe(executable: &str, args: &[&str]) -> Option<String> {
    let args = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let (executable, args) = normalized_command(executable, &args);
    let output = Command::new(executable).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let text = if stdout.is_empty() { stderr } else { stdout };
    (!text.is_empty()).then_some(text.lines().next().unwrap_or(&text).to_owned())
}

pub fn capture_environment(conn: &Connection, session_id: &str) -> Result<EnvironmentSnapshot> {
    if timeline::get_session_record(conn, session_id)?.is_none() {
        return Err(WorkflowError::SessionNotFound(session_id.to_owned()));
    }
    let mut runtimes = BTreeMap::new();
    for (name, exe, args) in [
        ("Node.js", "node", vec!["--version"]),
        ("npm", "npm", vec!["--version"]),
        ("Rust", "rustc", vec!["--version"]),
        ("Cargo", "cargo", vec!["--version"]),
        ("Python", "python", vec!["--version"]),
    ] {
        if let Some(version) = probe(exe, &args) {
            runtimes.insert(name.to_owned(), version);
        }
    }
    let snapshot = EnvironmentSnapshot {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_owned(),
        captured_at: unix_time_secs()?,
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        git_version: probe("git", &["--version"]),
        runtimes,
    };
    conn.execute(
        "INSERT INTO environment_snapshots(id,session_id,captured_at,os,arch,git_version,runtimes_json) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![snapshot.id,snapshot.session_id,snapshot.captured_at,snapshot.os,snapshot.arch,snapshot.git_version,serde_json::to_string(&snapshot.runtimes)?],
    )?;
    let meta = serde_json::json!({"os":snapshot.os,"arch":snapshot.arch,"git":snapshot.git_version,"runtimes":snapshot.runtimes});
    let action = timeline::new_action(
        session_id,
        "environment:capture",
        "Succeeded",
        Some(meta.to_string()),
    )?;
    timeline::create_action(conn, &action)?;
    Ok(snapshot)
}

pub fn latest_environment(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<EnvironmentSnapshot>> {
    let raw: Option<EnvironmentSnapshotRow> = conn
        .query_row(
            "SELECT id,session_id,captured_at,os,arch,git_version,runtimes_json FROM environment_snapshots WHERE session_id=?1 ORDER BY captured_at DESC,rowid DESC LIMIT 1",
            rusqlite::params![session_id],
            |row| {
                Ok(EnvironmentSnapshotRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    captured_at: row.get(2)?,
                    os: row.get(3)?,
                    arch: row.get(4)?,
                    git_version: row.get(5)?,
                    runtimes_json: row.get(6)?,
                })
            },
        )
        .optional()?;
    raw.map(|row| {
        Ok(EnvironmentSnapshot {
            id: row.id,
            session_id: row.session_id,
            captured_at: row.captured_at,
            os: row.os,
            arch: row.arch,
            git_version: row.git_version,
            runtimes: serde_json::from_str(&row.runtimes_json)?,
        })
    })
    .transpose()
}

fn store_run(conn: &Connection, run: &ReproductionRun) -> Result<()> {
    conn.execute(
        "INSERT INTO reproduction_runs(id,step_id,phase,action_id,receipt_id,exit_code,status,cycle,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        rusqlite::params![run.id,run.step_id,run.phase.to_string(),run.action_id,run.receipt_id,run.exit_code,run.status,run.cycle,run.created_at],
    )?;
    Ok(())
}

pub fn list_reproduction_runs(conn: &Connection, session_id: &str) -> Result<Vec<ReproductionRun>> {
    let mut stmt = conn.prepare(
        "SELECT rr.id,rr.step_id,rr.phase,rr.action_id,rr.receipt_id,rr.exit_code,rr.status,rr.cycle,rr.created_at FROM reproduction_runs rr JOIN reproduction_steps rs ON rs.id=rr.step_id WHERE rs.session_id=?1 ORDER BY rr.created_at DESC,rr.rowid DESC"
    )?;
    let raw = stmt
        .query_map(rusqlite::params![session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i32>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(raw
        .into_iter()
        .map(|r| ReproductionRun {
            id: r.0,
            step_id: r.1,
            phase: if r.2 == "After" {
                ReproductionPhase::After
            } else {
                ReproductionPhase::Before
            },
            action_id: r.3,
            receipt_id: r.4,
            exit_code: r.5,
            status: r.6,
            cycle: r.7,
            created_at: r.8,
        })
        .collect())
}

fn transition_for_run_start(
    conn: &Connection,
    session_id: &str,
    phase: ReproductionPhase,
) -> Result<()> {
    let state: SessionState = timeline::get_session_record(conn, session_id)?
        .ok_or_else(|| WorkflowError::SessionNotFound(session_id.to_owned()))?
        .state
        .parse()?;
    let target = match phase {
        ReproductionPhase::Before => SessionState::Reproducing,
        ReproductionPhase::After => SessionState::Verifying,
    };
    if state != target {
        state_machine::transition_session(conn, session_id, target)?;
    }
    Ok(())
}

fn transition_for_run_result(
    conn: &Connection,
    session_id: &str,
    phase: ReproductionPhase,
    status: &str,
) -> Result<()> {
    match (phase, status) {
        (ReproductionPhase::Before, "Failed") => {
            state_machine::transition_session(conn, session_id, SessionState::FailureCaptured)?;
        }
        (ReproductionPhase::Before, "Passed") => {
            state_machine::transition_session(conn, session_id, SessionState::Ready)?;
        }
        (ReproductionPhase::After, "Passed") => {
            // The caller records an exact patch proof before advancing. A
            // successful exit code alone is never sufficient for Apply.
        }
        (ReproductionPhase::After, "Failed") => {
            state_machine::transition_session(conn, session_id, SessionState::Fixing)?;
        }
        (ReproductionPhase::Before, _) => {
            state_machine::transition_session(conn, session_id, SessionState::Ready)?;
        }
        (ReproductionPhase::After, _) => {
            state_machine::transition_session(conn, session_id, SessionState::Fixing)?;
        }
    }
    Ok(())
}

pub fn execute_reproduction_step(
    conn: &mut Connection,
    artifact_store: &Path,
    step_id: &str,
    phase: ReproductionPhase,
    explicitly_approved_once: bool,
) -> Result<RunOutcome> {
    let step = get_reproduction_step(conn, step_id)?;
    let shadow = match shadow_session::get_session_shadow(conn, &step.session_id)? {
        Some(value) => value,
        None => shadow_session::create_session_shadow(conn, &step.session_id)?,
    };

    if matches!(phase, ReproductionPhase::After) {
        // Verification is meaningful only for a deterministic checkpoint. This
        // also makes added/deleted/index changes visible instead of silently
        // testing bytes that Apply would not carry.
        shadow_session::current_patch_identity(conn, &step.session_id)?;
    }

    if matches!(phase, ReproductionPhase::Before) {
        let existing: Option<String> = conn.query_row(
            "SELECT status FROM reproduction_runs WHERE step_id=?1 AND phase='Before' AND cycle=?2 ORDER BY created_at DESC,rowid DESC LIMIT 1",
            rusqlite::params![step.id, step.active_cycle],
            |row| row.get(0),
        ).optional()?;
        if existing.is_some() {
            return Err(WorkflowError::BaselineLocked);
        }
    }

    let decision = permissions::command_permission(
        &step.executable,
        &step.args,
        Permission::Ask,
        explicitly_approved_once,
        false,
    );
    match decision.permission {
        Permission::Deny => return Err(WorkflowError::PermissionDenied(decision.explanation)),
        Permission::Ask => return Err(WorkflowError::ApprovalRequired(decision.explanation)),
        Permission::Allow => {}
    }

    verification::invalidate_proof(conn, &step.session_id)?;

    transition_for_run_start(conn, &step.session_id, phase)?;

    let meta = serde_json::json!({
        "phase": phase.to_string(), "step_id": step.id,
        "command": { "executable": redaction::redact_text(&step.executable), "args": step.args.iter().map(|a| redaction::redact_text(a)).collect::<Vec<_>>(), "cwd": shadow.worktree_path },
        "expected_exit_code": step.expected_exit_code,
    });
    let action = timeline::new_action(
        &step.session_id,
        "reproduction:command",
        "Running",
        Some(meta.to_string()),
    )?;
    timeline::create_action(conn, &action)?;
    let execution_id = timeline::start_execution(conn, &action.id)?;

    let (runner_executable, runner_args) = normalized_command(&step.executable, &step.args);
    let spec = CommandSpec {
        executable: runner_executable,
        args: runner_args,
        cwd: Some(PathBuf::from(&shadow.worktree_path)),
        env: None,
        clear_env: false,
        timeout: Some(Duration::from_secs(10 * 60)),
        output_limit: Some(10 * 1024 * 1024),
    };

    let run_id = Uuid::new_v4().to_string();
    let created_at = unix_time_secs()?;
    match runner::run_command(spec, Permission::Allow, None) {
        Ok(result) => {
            let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
            let meets_expected = result.exit_code == Some(step.expected_exit_code);
            let status = if meets_expected { "Passed" } else { "Failed" };
            let receipt_id = timeline::finish_execution(
                conn,
                &execution_id,
                status,
                Some(&stdout),
                Some(&stderr),
            )?;
            conn.execute(
                "UPDATE actions SET state=?1 WHERE id=?2",
                rusqlite::params![status, action.id],
            )?;
            let stdout_artifact = if stdout.is_empty() {
                None
            } else {
                Some(evidence::persist_text_artifact(
                    conn,
                    artifact_store,
                    &receipt_id,
                    &stdout,
                    Some("text/plain; stream=stdout"),
                )?)
            };
            let stderr_artifact = if stderr.is_empty() {
                None
            } else {
                Some(evidence::persist_text_artifact(
                    conn,
                    artifact_store,
                    &receipt_id,
                    &stderr,
                    Some("text/plain; stream=stderr"),
                )?)
            };
            let evidence_artifact = stderr_artifact.as_ref().or(stdout_artifact.as_ref());
            let evidence_source = format!("{} reproduction", phase);
            let evidence_summary = format!(
                "{} {} returned {:?}; expected {}",
                step.executable,
                step.args.join(" "),
                result.exit_code,
                step.expected_exit_code
            );
            evidence::create_evidence_item(
                conn,
                evidence::NewEvidenceItem {
                    session_id: &step.session_id,
                    action_id: Some(&action.id),
                    receipt_id: Some(&receipt_id),
                    kind: if status == "Passed" {
                        evidence::EvidenceKind::CommandSuccess
                    } else {
                        evidence::EvidenceKind::CommandFailure
                    },
                    source: &evidence_source,
                    summary: &evidence_summary,
                    artifact: evidence_artifact,
                },
            )?;
            let run = ReproductionRun {
                id: run_id,
                step_id: step.id.clone(),
                phase,
                action_id: action.id,
                receipt_id: Some(receipt_id),
                exit_code: result.exit_code,
                status: status.into(),
                cycle: step.active_cycle,
                created_at,
            };
            store_run(conn, &run)?;
            transition_for_run_result(conn, &step.session_id, phase, status)?;
            if matches!(phase, ReproductionPhase::Before) && status == "Failed" {
                verification::activate_handoff_after_baseline(conn, &step.session_id)?;
            }
            if matches!(phase, ReproductionPhase::After) && status == "Passed" {
                verification::record_after_success(conn, &step, &run.id)?;
                verification::mark_verified_state(conn, &step.session_id)?;
            }
            Ok(RunOutcome {
                run,
                permission: decision,
                stdout_artifact,
                stderr_artifact,
            })
        }
        Err(error) => {
            let (status, diagnostic) = match error {
                CommandError::Cancelled => ("Interrupted", "Command was cancelled.".to_owned()),
                CommandError::Timeout => (
                    "Error",
                    "Command exceeded the 10 minute timeout.".to_owned(),
                ),
                other => ("Error", format!("Command runner error: {other}")),
            };
            let receipt_id =
                timeline::finish_execution(conn, &execution_id, status, None, Some(&diagnostic))?;
            conn.execute(
                "UPDATE actions SET state=?1 WHERE id=?2",
                rusqlite::params![status, action.id],
            )?;
            let stderr_artifact = Some(evidence::persist_text_artifact(
                conn,
                artifact_store,
                &receipt_id,
                &diagnostic,
                Some("text/plain; stream=diagnostic"),
            )?);
            let evidence_source = format!("{} reproduction", phase);
            evidence::create_evidence_item(
                conn,
                evidence::NewEvidenceItem {
                    session_id: &step.session_id,
                    action_id: Some(&action.id),
                    receipt_id: Some(&receipt_id),
                    kind: evidence::EvidenceKind::CommandFailure,
                    source: &evidence_source,
                    summary: &diagnostic,
                    artifact: stderr_artifact.as_ref(),
                },
            )?;
            let run = ReproductionRun {
                id: run_id,
                step_id: step.id,
                phase,
                action_id: action.id,
                receipt_id: Some(receipt_id),
                exit_code: None,
                status: status.into(),
                cycle: step.active_cycle,
                created_at,
            };
            store_run(conn, &run)?;
            transition_for_run_result(conn, &step.session_id, phase, status)?;
            Ok(RunOutcome {
                run,
                permission: decision,
                stdout_artifact: None,
                stderr_artifact,
            })
        }
    }
}

pub fn reset_reproduction_baseline(conn: &Connection, step_id: &str) -> Result<ReproductionStep> {
    let step = get_reproduction_step(conn, step_id)?;
    let session = timeline::get_session_record(conn, &step.session_id)?
        .ok_or_else(|| WorkflowError::SessionNotFound(step.session_id.clone()))?;
    let state: SessionState = session.state.parse()?;
    if matches!(
        state,
        SessionState::Applied | SessionState::Discarded | SessionState::Applying
    ) {
        return Err(WorkflowError::State(
            state_machine::StateError::InvalidTransition {
                from: state,
                to: SessionState::Ready,
            },
        ));
    }
    let has_baseline: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM reproduction_runs WHERE step_id=?1 AND phase='Before' AND cycle=?2)",
        rusqlite::params![step.id, step.active_cycle],
        |row| row.get(0),
    )?;
    if !has_baseline {
        return Err(WorkflowError::BaselineMissing);
    }
    let next_cycle = step.active_cycle.saturating_add(1);
    conn.execute(
        "UPDATE reproduction_steps SET active_cycle=?1 WHERE id=?2",
        rusqlite::params![next_cycle, step_id],
    )?;
    verification::invalidate_proof(conn, &step.session_id)?;
    conn.execute(
        "UPDATE regression_checks SET status='Pending',receipt_id=NULL,verified_patch_sha256=NULL,updated_at=?1 WHERE session_id=?2",
        rusqlite::params![unix_time_secs()?,step.session_id],
    )?;
    timeline::update_session_state(conn, &step.session_id, "Ready")?;
    let action = timeline::new_action(
        &step.session_id,
        "verification:baseline-reset",
        "Succeeded",
        Some(serde_json::json!({"step_id":step_id,"previous_cycle":step.active_cycle,"active_cycle":next_cycle}).to_string()),
    )?;
    timeline::create_action(conn, &action)?;
    get_reproduction_step(conn, step_id)
}

pub fn outcome_for_step(conn: &Connection, step_id: &str) -> Result<String> {
    let step = get_reproduction_step(conn, step_id)?;
    Ok(verification::status(conn, &step.session_id)?.outcome)
}

pub fn outcome_for_session(conn: &Connection, session_id: &str) -> Result<String> {
    Ok(verification::status(conn, session_id)?.outcome)
}
