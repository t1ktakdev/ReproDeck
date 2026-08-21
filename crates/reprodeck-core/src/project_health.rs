use crate::git_shadow::{GitShadowError, Shadow};
use crate::permissions::{self, Permission};
use crate::problem::ProblemStatus;
use crate::project_intelligence::{ProjectCommand, ProjectCommandKind, ProjectProfile};
use crate::redaction;
use crate::runner::{self, CommandError, CommandSpec};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_TIMEOUT_SECS: u64 = 180;
const MIN_TIMEOUT_SECS: u64 = 5;
const MAX_TIMEOUT_SECS: u64 = 15 * 60;
const OUTPUT_LIMIT_BYTES: usize = 2 * 1024 * 1024;
const PREVIEW_CHARS: usize = 24_000;
const MAX_CHECKS_PER_RUN: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GitSnapshot {
    head: String,
    status: Vec<u8>,
    tracked_diff_sha256: String,
}

#[derive(Debug)]
struct HealthExecutionOutcome {
    checks: Vec<HealthCheckResult>,
    original_after: GitSnapshot,
    finished_at: i64,
}

#[derive(Debug, Error)]
pub enum ProjectHealthError {
    #[error(transparent)]
    Git(#[from] GitShadowError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("Project Health requires a Git repository with a committed HEAD.")]
    GitRequired,
    #[error("project path is outside the Git worktree")]
    ProjectOutsideRepository,
    #[error("no runnable checks were selected")]
    NoRunnableChecks,
    #[error("check execution requires explicit confirmation")]
    ConfirmationRequired,
    #[error("unable to inspect original repository state: {0}")]
    GitState(String),
    #[error("isolated workspace cleanup failed: {0}")]
    Cleanup(String),
}

pub type Result<T> = std::result::Result<T, ProjectHealthError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthRunStatus {
    Clean,
    ProblemsFound,
    Incomplete,
    OriginalChanged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthCheckStatus {
    Passed,
    Failed,
    Blocked,
    TimedOut,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthRunOptions {
    pub command_ids: Vec<String>,
    pub timeout_secs: u64,
    pub confirmed_execution: bool,
}

impl Default for HealthRunOptions {
    fn default() -> Self {
        Self {
            command_ids: Vec::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            confirmed_execution: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthCheckResult {
    pub id: String,
    pub command_id: String,
    pub label: String,
    pub kind: ProjectCommandKind,
    pub executable: String,
    pub args: Vec<String>,
    pub status: HealthCheckStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub evidence_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectProblemRecord {
    pub id: String,
    pub problem_key: String,
    pub root_path: String,
    pub status: ProblemStatus,
    /// True when the most recent conclusive run of this command still reproduces the failure.
    /// A passing rerun clears this flag but does not promote the problem to `Verified`;
    /// verification belongs to the explicit fix/proof workflow.
    pub active: bool,
    pub title: String,
    pub summary: String,
    pub command_id: String,
    pub health_run_id: String,
    pub check_run_id: String,
    pub evidence_ids: Vec<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub cleared_at: Option<i64>,
    pub occurrences: u32,
}

#[derive(Debug)]
struct StoredProblemRow {
    id: String,
    problem_key: String,
    root_path: String,
    status: String,
    active: bool,
    title: String,
    summary: String,
    command_id: String,
    health_run_id: String,
    check_run_id: String,
    evidence_json: String,
    first_seen_at: i64,
    last_seen_at: i64,
    cleared_at: Option<i64>,
    occurrences: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectHealthReport {
    pub id: String,
    pub root_path: String,
    pub project_name: String,
    pub base_commit: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub status: HealthRunStatus,
    pub original_unchanged: bool,
    pub source_had_local_changes: bool,
    pub checks: Vec<HealthCheckResult>,
    pub problems: Vec<ProjectProblemRecord>,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn runnable_kind(kind: ProjectCommandKind) -> bool {
    matches!(
        kind,
        ProjectCommandKind::Build
            | ProjectCommandKind::Test
            | ProjectCommandKind::Lint
            | ProjectCommandKind::Typecheck
            | ProjectCommandKind::Check
    )
}

fn all_runnable_commands(profile: &ProjectProfile) -> Vec<ProjectCommand> {
    profile
        .commands
        .iter()
        .filter(|command| runnable_kind(command.kind))
        .cloned()
        .collect()
}

pub fn runnable_commands(profile: &ProjectProfile) -> Vec<ProjectCommand> {
    all_runnable_commands(profile)
        .into_iter()
        .take(MAX_CHECKS_PER_RUN)
        .collect()
}

fn selected_commands(profile: &ProjectProfile, ids: &[String]) -> Vec<ProjectCommand> {
    if ids.is_empty() {
        return runnable_commands(profile);
    }

    // Preserve the caller's explicit order. Bug Hunter uses this to run cheap,
    // high-signal diagnostics before broader tests/builds instead of falling
    // back to discovery order. Build the allow-map from every deterministic
    // command, then apply the execution budget after ordering; otherwise a
    // planned high-signal command could disappear merely because discovery
    // found eight lower-value commands first. Unknown/duplicate ids are ignored.
    let by_id = all_runnable_commands(profile)
        .into_iter()
        .map(|command| (command.id.clone(), command))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert((*id).clone()))
        .filter_map(|id| by_id.get(id).cloned())
        .take(MAX_CHECKS_PER_RUN)
        .collect()
}

fn timeout(options: &HealthRunOptions) -> Duration {
    Duration::from_secs(
        options
            .timeout_secs
            .clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS),
    )
}

pub(crate) fn git_snapshot(repo: &Path) -> Result<GitSnapshot> {
    let head = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map_err(|error| ProjectHealthError::GitState(error.to_string()))?;
    if !head.status.success() {
        return Err(ProjectHealthError::GitState(
            String::from_utf8_lossy(&head.stderr).trim().to_owned(),
        ));
    }
    let status = Command::new("git")
        .current_dir(repo)
        .args(["status", "--porcelain=v1", "-z"])
        .output()
        .map_err(|error| ProjectHealthError::GitState(error.to_string()))?;
    if !status.status.success() {
        return Err(ProjectHealthError::GitState(
            String::from_utf8_lossy(&status.stderr).trim().to_owned(),
        ));
    }
    // Porcelain status alone is not enough when the source tree is already dirty:
    // changing `M file.rs` from one set of contents to another leaves the status
    // line unchanged. Hash the complete tracked diff as well so Project Health can
    // detect content changes to already-modified tracked files.
    let diff = Command::new("git")
        .current_dir(repo)
        .args([
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ])
        .output()
        .map_err(|error| ProjectHealthError::GitState(error.to_string()))?;
    if !diff.status.success() {
        return Err(ProjectHealthError::GitState(
            String::from_utf8_lossy(&diff.stderr).trim().to_owned(),
        ));
    }
    let tracked_diff_sha256 = hex::encode(Sha256::digest(&diff.stdout));
    Ok(GitSnapshot {
        head: String::from_utf8_lossy(&head.stdout).trim().to_owned(),
        status: status.stdout,
        tracked_diff_sha256,
    })
}

fn filtered_environment_from<I>(vars: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    const ALLOWED: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PROGRAMW6432",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ];
    vars.into_iter()
        .filter(|(key, _)| ALLOWED.contains(&key.to_ascii_uppercase().as_str()))
        .collect()
}

fn safe_environment() -> HashMap<String, String> {
    filtered_environment_from(std::env::vars())
}

fn executable_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .to_ascii_lowercase()
}

fn package_has_dependencies(package_json: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(package_json) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    ["dependencies", "devDependencies", "optionalDependencies"]
        .into_iter()
        .any(|key| {
            value
                .get(key)
                .and_then(Value::as_object)
                .is_some_and(|items| !items.is_empty())
        })
}

fn has_node_dependency_environment(cwd: &Path) -> bool {
    cwd.join("node_modules").is_dir()
        || cwd.join(".pnp.cjs").is_file()
        || cwd.join(".pnp.js").is_file()
}

fn check_precondition(command: &ProjectCommand, cwd: &Path) -> Option<String> {
    let executable = executable_name(&command.executable);
    let first_arg = command.args.first().map(|value| value.to_ascii_lowercase());

    // Dynamic package executors may download and execute code that is not present
    // in the repository. A generic "run project checks" confirmation is not
    // sufficient consent for that extra supply-chain/network action.
    if matches!(executable.as_str(), "npx" | "npx.cmd")
        || matches!(
            (executable.as_str(), first_arg.as_deref()),
            ("npm" | "npm.cmd", Some("exec" | "x"))
                | ("pnpm" | "pnpm.cmd" | "yarn" | "yarn.cmd", Some("dlx"))
        )
    {
        return Some(
            "Dynamic package execution is blocked by Project Health. Run or install the required tool explicitly, then use a deterministic local command."
                .into(),
        );
    }

    if matches!(
        executable.as_str(),
        "npm" | "npm.cmd" | "pnpm" | "pnpm.cmd" | "yarn" | "yarn.cmd"
    ) {
        let package_json = cwd.join("package.json");
        if package_json.is_file()
            && package_has_dependencies(&package_json)
            && !has_node_dependency_environment(cwd)
        {
            return Some(
                "Project dependencies are not present in the isolated worktree. ReproDeck does not install packages automatically, so this check is blocked instead of being reported as a project failure."
                    .into(),
            );
        }
    }
    None
}

fn preview(bytes: &[u8]) -> (String, bool) {
    let text = redaction::redact_text(&String::from_utf8_lossy(bytes));
    let mut chars = text.chars();
    let value = chars.by_ref().take(PREVIEW_CHARS).collect::<String>();
    (value, chars.next().is_some())
}

fn first_meaningful_line(stderr: &str, stdout: &str) -> Option<String> {
    stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(240).collect())
}

fn failure_summary(
    command: &ProjectCommand,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> String {
    let detail = first_meaningful_line(stderr, stdout)
        .unwrap_or_else(|| format!("{} exited with {:?}", command.label, exit_code));
    redaction::redact_text(&detail)
}

fn problem_key(root_path: &str, command_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root_path.as_bytes());
    hasher.update([0]);
    hasher.update(command_id.as_bytes());
    format!("problem:{}", &hex::encode(hasher.finalize())[..24])
}

fn project_relative_path(profile: &ProjectProfile, repo: &Path) -> Result<PathBuf> {
    let project = Path::new(&profile.root_path).canonicalize()?;
    let repo = repo.canonicalize()?;
    let relative = project
        .strip_prefix(&repo)
        .map_err(|_| ProjectHealthError::ProjectOutsideRepository)?;
    Ok(relative.to_path_buf())
}

pub(crate) fn run_one_check(
    run_id: &str,
    command: &ProjectCommand,
    cwd: &Path,
    options: &HealthRunOptions,
) -> HealthCheckResult {
    let check_id = Uuid::new_v4().to_string();
    let evidence_id = format!("health:{run_id}:{check_id}");
    let decision = permissions::command_permission(
        &command.executable,
        &command.args,
        Permission::Ask,
        options.confirmed_execution,
        false,
    );

    if decision.permission != Permission::Allow {
        return HealthCheckResult {
            id: check_id,
            command_id: command.id.clone(),
            label: command.label.clone(),
            kind: command.kind,
            executable: command.executable.clone(),
            args: command.args.clone(),
            status: HealthCheckStatus::Blocked,
            exit_code: None,
            duration_ms: 0,
            stdout_preview: String::new(),
            stderr_preview: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            evidence_id,
            summary: decision.explanation,
        };
    }

    if let Some(summary) = check_precondition(command, cwd) {
        return HealthCheckResult {
            id: check_id,
            command_id: command.id.clone(),
            label: command.label.clone(),
            kind: command.kind,
            executable: command.executable.clone(),
            args: command.args.clone(),
            status: HealthCheckStatus::Blocked,
            exit_code: None,
            duration_ms: 0,
            stdout_preview: String::new(),
            stderr_preview: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            evidence_id,
            summary,
        };
    }

    #[cfg(windows)]
    {
        let executable = command.executable.to_ascii_lowercase();
        if (executable.ends_with(".cmd") || executable.ends_with(".bat"))
            && !matches!(executable.as_str(), "npm.cmd" | "npx.cmd")
        {
            return HealthCheckResult {
                id: check_id,
                command_id: command.id.clone(),
                label: command.label.clone(),
                kind: command.kind,
                executable: command.executable.clone(),
                args: command.args.clone(),
                status: HealthCheckStatus::Blocked,
                exit_code: None,
                duration_ms: 0,
                stdout_preview: String::new(),
                stderr_preview: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                evidence_id,
                summary: "Windows batch wrappers require shell execution and are not run automatically by Project Health.".into(),
            };
        }
    }

    let (executable, args) =
        crate::workflow::normalized_command(&command.executable, &command.args);
    let spec = CommandSpec {
        executable,
        args,
        cwd: Some(cwd.to_path_buf()),
        env: Some(safe_environment()),
        clear_env: true,
        timeout: Some(timeout(options)),
        output_limit: Some(OUTPUT_LIMIT_BYTES),
    };

    match runner::run_command(spec, Permission::Allow, None) {
        Ok(result) => {
            let (stdout_preview, stdout_preview_truncated) = preview(&result.stdout);
            let (stderr_preview, stderr_preview_truncated) = preview(&result.stderr);
            let status = if result.exit_code == Some(0) {
                HealthCheckStatus::Passed
            } else {
                HealthCheckStatus::Failed
            };
            let summary = if status == HealthCheckStatus::Passed {
                format!("{} passed.", command.label)
            } else {
                failure_summary(command, result.exit_code, &stdout_preview, &stderr_preview)
            };
            HealthCheckResult {
                id: check_id,
                command_id: command.id.clone(),
                label: command.label.clone(),
                kind: command.kind,
                executable: command.executable.clone(),
                args: command.args.clone(),
                status,
                exit_code: result.exit_code,
                duration_ms: result
                    .finished_at
                    .duration_since(result.started_at)
                    .as_millis() as u64,
                stdout_preview,
                stderr_preview,
                stdout_truncated: result.stdout_truncated || stdout_preview_truncated,
                stderr_truncated: result.stderr_truncated || stderr_preview_truncated,
                evidence_id,
                summary,
            }
        }
        Err(error) => {
            let (status, summary) = match error {
                CommandError::Timeout => (
                    HealthCheckStatus::TimedOut,
                    format!(
                        "{} exceeded the {} second time limit.",
                        command.label,
                        timeout(options).as_secs()
                    ),
                ),
                CommandError::PermissionDenied | CommandError::DecisionRequired => (
                    HealthCheckStatus::Blocked,
                    format!("{} was blocked by command policy.", command.label),
                ),
                other => (
                    HealthCheckStatus::Error,
                    redaction::redact_text(&format!("{} could not run: {other}", command.label)),
                ),
            };
            HealthCheckResult {
                id: check_id,
                command_id: command.id.clone(),
                label: command.label.clone(),
                kind: command.kind,
                executable: command.executable.clone(),
                args: command.args.clone(),
                status,
                exit_code: None,
                duration_ms: 0,
                stdout_preview: String::new(),
                stderr_preview: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                evidence_id,
                summary,
            }
        }
    }
}

fn build_problem(
    profile: &ProjectProfile,
    health_run_id: &str,
    check: &HealthCheckResult,
    observed_at: i64,
) -> ProjectProblemRecord {
    let key = problem_key(&profile.root_path, &check.command_id);
    ProjectProblemRecord {
        id: Uuid::new_v4().to_string(),
        problem_key: key,
        root_path: profile.root_path.clone(),
        status: ProblemStatus::Reproduced,
        active: true,
        title: format!("{} failed", check.label),
        summary: check.summary.clone(),
        command_id: check.command_id.clone(),
        health_run_id: health_run_id.to_owned(),
        check_run_id: check.id.clone(),
        evidence_ids: vec![check.evidence_id.clone()],
        first_seen_at: observed_at,
        last_seen_at: observed_at,
        cleared_at: None,
        occurrences: 1,
    }
}

pub fn run_project_health(
    profile: &ProjectProfile,
    options: &HealthRunOptions,
) -> Result<ProjectHealthReport> {
    if !options.confirmed_execution {
        return Err(ProjectHealthError::ConfirmationRequired);
    }
    let git = profile
        .git
        .as_ref()
        .ok_or(ProjectHealthError::GitRequired)?;
    let base_commit = git
        .head_commit
        .clone()
        .ok_or(ProjectHealthError::GitRequired)?;
    let commands = selected_commands(profile, &options.command_ids);
    if commands.is_empty() {
        return Err(ProjectHealthError::NoRunnableChecks);
    }

    let run_id = Uuid::new_v4().to_string();
    let started_at = unix_time_secs()?;
    let repo_path = Path::new(&git.root_path);
    let project_relative = project_relative_path(profile, repo_path)?;
    let before = git_snapshot(repo_path)?;
    let shadow = Shadow::create(repo_path, Some(&base_commit))?;
    let cwd = shadow.worktree.join(project_relative);

    let run_result = (|| -> Result<HealthExecutionOutcome> {
        let checks = commands
            .iter()
            .map(|command| run_one_check(&run_id, command, &cwd, options))
            .collect::<Vec<_>>();
        let original_after = git_snapshot(repo_path)?;
        let finished_at = unix_time_secs()?;
        Ok(HealthExecutionOutcome {
            checks,
            original_after,
            finished_at,
        })
    })();
    let cleanup_result = shadow.discard();
    if let Err(error) = cleanup_result {
        return Err(ProjectHealthError::Cleanup(error.to_string()));
    }
    let outcome = run_result?;
    let original_unchanged = before == outcome.original_after;
    let checks = outcome.checks;
    let finished_at = outcome.finished_at;
    let problems = checks
        .iter()
        .filter(|check| check.status == HealthCheckStatus::Failed)
        .map(|check| build_problem(profile, &run_id, check, finished_at))
        .collect::<Vec<_>>();

    let status = if !original_unchanged {
        HealthRunStatus::OriginalChanged
    } else if !problems.is_empty() {
        HealthRunStatus::ProblemsFound
    } else if checks.iter().any(|check| {
        matches!(
            check.status,
            HealthCheckStatus::Blocked | HealthCheckStatus::TimedOut | HealthCheckStatus::Error
        )
    }) {
        HealthRunStatus::Incomplete
    } else {
        HealthRunStatus::Clean
    };

    Ok(ProjectHealthReport {
        id: run_id,
        root_path: profile.root_path.clone(),
        project_name: profile.name.clone(),
        base_commit,
        started_at,
        finished_at,
        status,
        original_unchanged,
        source_had_local_changes: git.is_dirty,
        checks,
        problems,
    })
}

pub fn save_report(conn: &mut Connection, report: &ProjectHealthReport) -> Result<()> {
    let report_json = serde_json::to_string(report)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO project_health_runs(id,root_path,base_commit,started_at,finished_at,status,report_json) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![
            report.id,
            report.root_path,
            report.base_commit,
            report.started_at,
            report.finished_at,
            format!("{:?}", report.status),
            report_json,
        ],
    )?;

    // A passing result is conclusive evidence that an earlier health failure for the
    // same command is not currently reproducing. Keep the historical record, but
    // never call it `Verified`: fix verification is a separate proof workflow.
    for check in &report.checks {
        if check.status == HealthCheckStatus::Passed {
            tx.execute(
                "UPDATE project_problems SET active=0,cleared_at=?1 WHERE root_path=?2 AND command_id=?3 AND active=1",
                rusqlite::params![report.finished_at, report.root_path, check.command_id],
            )?;
        }
    }

    for problem in &report.problems {
        let existing: Option<(String, i64)> = tx
            .query_row(
                "SELECT id,occurrences FROM project_problems WHERE problem_key=?1",
                rusqlite::params![problem.problem_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let evidence_json = serde_json::to_string(&problem.evidence_ids)?;
        if let Some((id, occurrences)) = existing {
            tx.execute(
                "UPDATE project_problems SET status=?1,active=1,title=?2,summary=?3,command_id=?4,health_run_id=?5,check_run_id=?6,evidence_json=?7,last_seen_at=?8,cleared_at=NULL,occurrences=?9 WHERE id=?10",
                rusqlite::params![
                    format!("{:?}", problem.status),
                    problem.title,
                    problem.summary,
                    problem.command_id,
                    problem.health_run_id,
                    problem.check_run_id,
                    evidence_json,
                    problem.last_seen_at,
                    occurrences.saturating_add(1),
                    id,
                ],
            )?;
        } else {
            tx.execute(
                "INSERT INTO project_problems(id,problem_key,root_path,status,active,title,summary,command_id,health_run_id,check_run_id,evidence_json,first_seen_at,last_seen_at,cleared_at,occurrences) VALUES (?1,?2,?3,?4,1,?5,?6,?7,?8,?9,?10,?11,?12,NULL,?13)",
                rusqlite::params![
                    problem.id,
                    problem.problem_key,
                    problem.root_path,
                    format!("{:?}", problem.status),
                    problem.title,
                    problem.summary,
                    problem.command_id,
                    problem.health_run_id,
                    problem.check_run_id,
                    evidence_json,
                    problem.first_seen_at,
                    problem.last_seen_at,
                    i64::from(problem.occurrences),
                ],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn parse_problem_status(value: &str) -> ProblemStatus {
    match value {
        "Suspected" => ProblemStatus::Suspected,
        "Reproduced" => ProblemStatus::Reproduced,
        "RootCaused" => ProblemStatus::RootCaused,
        "FixProposed" => ProblemStatus::FixProposed,
        "Verified" => ProblemStatus::Verified,
        "Applied" => ProblemStatus::Applied,
        "Dismissed" => ProblemStatus::Dismissed,
        _ => ProblemStatus::Signal,
    }
}

pub fn list_project_problems(
    conn: &Connection,
    root_path: &str,
    limit: usize,
) -> Result<Vec<ProjectProblemRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id,problem_key,root_path,status,active,title,summary,command_id,health_run_id,check_run_id,evidence_json,first_seen_at,last_seen_at,cleared_at,occurrences FROM project_problems WHERE root_path=?1 ORDER BY active DESC,last_seen_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![root_path, limit as i64], |row| {
        Ok(StoredProblemRow {
            id: row.get(0)?,
            problem_key: row.get(1)?,
            root_path: row.get(2)?,
            status: row.get(3)?,
            active: row.get(4)?,
            title: row.get(5)?,
            summary: row.get(6)?,
            command_id: row.get(7)?,
            health_run_id: row.get(8)?,
            check_run_id: row.get(9)?,
            evidence_json: row.get(10)?,
            first_seen_at: row.get(11)?,
            last_seen_at: row.get(12)?,
            cleared_at: row.get(13)?,
            occurrences: row.get(14)?,
        })
    })?;
    let mut problems = Vec::new();
    for row in rows {
        let row = row?;
        problems.push(ProjectProblemRecord {
            id: row.id,
            problem_key: row.problem_key,
            root_path: row.root_path,
            status: parse_problem_status(&row.status),
            active: row.active,
            title: row.title,
            summary: row.summary,
            command_id: row.command_id,
            health_run_id: row.health_run_id,
            check_run_id: row.check_run_id,
            evidence_ids: serde_json::from_str(&row.evidence_json)?,
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
            cleared_at: row.cleared_at,
            occurrences: row.occurrences.clamp(0, i64::from(u32::MAX)) as u32,
        });
    }
    Ok(problems)
}

pub fn latest_report(conn: &Connection, root_path: &str) -> Result<Option<ProjectHealthReport>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT report_json FROM project_health_runs WHERE root_path=?1 ORDER BY finished_at DESC LIMIT 1",
            rusqlite::params![root_path],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| serde_json::from_str(&value).map_err(ProjectHealthError::from))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn profile(root: &Path) -> ProjectProfile {
        let head = String::from_utf8(
            Command::new("git")
                .current_dir(root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        ProjectProfile {
            schema_version: 1,
            fingerprint: "project:test".into(),
            root_path: root.canonicalize().unwrap().to_string_lossy().into_owned(),
            name: "fixture".into(),
            version: None,
            description: None,
            analyzed_at: 0,
            git: Some(ProjectGitState {
                root_path: root.canonicalize().unwrap().to_string_lossy().into_owned(),
                branch: "master".into(),
                head_commit: Some(head),
                is_dirty: false,
                changed_files: Vec::new(),
            }),
            languages: Vec::new(),
            technologies: Vec::new(),
            commands: vec![
                ProjectCommand {
                    id: "check:fail".into(),
                    label: "reproduction".into(),
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
                },
                ProjectCommand {
                    id: "check:pass".into(),
                    label: "git status".into(),
                    kind: ProjectCommandKind::Check,
                    executable: "git".into(),
                    args: vec!["status".into(), "--porcelain".into()],
                    source: "fixture".into(),
                    confidence: CommandConfidence::Declared,
                },
                ProjectCommand {
                    id: "dev".into(),
                    label: "dev server".into(),
                    kind: ProjectCommandKind::Dev,
                    executable: "git".into(),
                    args: vec!["status".into()],
                    source: "fixture".into(),
                    confidence: CommandConfidence::Declared,
                },
            ],
            entrypoints: Vec::new(),
            test_paths: Vec::new(),
            documentation: Vec::new(),
            ci_files: Vec::new(),
            signals: Vec::new(),
            stats: ProjectStats::default(),
        }
    }

    #[test]
    fn runnable_selection_excludes_dev_commands() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("state.txt"), "BAD\n").unwrap();
        git(dir.path(), &["add", "state.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);
        let profile = profile(dir.path());
        let commands = runnable_commands(&profile);
        assert_eq!(commands.len(), 2);
        assert!(commands
            .iter()
            .all(|command| command.kind != ProjectCommandKind::Dev));
    }

    #[test]
    fn selected_commands_preserve_explicit_bug_hunter_order() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("state.txt"), "BAD\n").unwrap();
        git(dir.path(), &["add", "state.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);
        let mut profile = profile(dir.path());
        profile.commands = vec![
            ProjectCommand {
                id: "build".into(),
                label: "build".into(),
                kind: ProjectCommandKind::Build,
                executable: "cargo".into(),
                args: vec!["build".into()],
                source: "fixture".into(),
                confidence: crate::project_intelligence::CommandConfidence::Declared,
            },
            ProjectCommand {
                id: "check".into(),
                label: "check".into(),
                kind: ProjectCommandKind::Check,
                executable: "cargo".into(),
                args: vec!["check".into()],
                source: "fixture".into(),
                confidence: crate::project_intelligence::CommandConfidence::Declared,
            },
            ProjectCommand {
                id: "test".into(),
                label: "test".into(),
                kind: ProjectCommandKind::Test,
                executable: "cargo".into(),
                args: vec!["test".into()],
                source: "fixture".into(),
                confidence: crate::project_intelligence::CommandConfidence::Declared,
            },
        ];

        let selected = selected_commands(
            &profile,
            &[
                "check".into(),
                "test".into(),
                "build".into(),
                "check".into(),
            ],
        );
        let ids = selected
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["check", "test", "build"]);
    }

    #[test]
    fn explicit_selection_can_choose_high_value_command_beyond_discovery_budget_order() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        std::fs::write(dir.path().join("state.txt"), "OK\n").unwrap();
        git(dir.path(), &["add", "state.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);
        let mut profile = profile(dir.path());
        profile.commands = (0..9)
            .map(|index| ProjectCommand {
                id: format!("cmd-{index}"),
                label: format!("cmd-{index}"),
                kind: if index == 8 {
                    ProjectCommandKind::Check
                } else {
                    ProjectCommandKind::Build
                },
                executable: "git".into(),
                args: vec!["status".into(), format!("--porcelain={index}")],
                source: "fixture".into(),
                confidence: crate::project_intelligence::CommandConfidence::Declared,
            })
            .collect();

        assert_eq!(runnable_commands(&profile).len(), MAX_CHECKS_PER_RUN);
        let selected = selected_commands(&profile, &["cmd-8".into(), "cmd-0".into()]);
        assert_eq!(
            selected
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["cmd-8", "cmd-0"]
        );
    }

    #[test]
    fn safe_environment_excludes_secret_and_injection_variables() {
        let filtered = filtered_environment_from([
            ("PATH".into(), "bin".into()),
            ("HOME".into(), "/home/test".into()),
            ("GITHUB_TOKEN".into(), "secret".into()),
            ("NPM_TOKEN".into(), "secret".into()),
            ("NODE_OPTIONS".into(), "--require malware.js".into()),
        ]);
        assert_eq!(filtered.get("PATH").map(String::as_str), Some("bin"));
        assert_eq!(filtered.get("HOME").map(String::as_str), Some("/home/test"));
        assert!(!filtered.contains_key("GITHUB_TOKEN"));
        assert!(!filtered.contains_key("NPM_TOKEN"));
        assert!(!filtered.contains_key("NODE_OPTIONS"));
    }

    #[test]
    fn node_precondition_blocks_missing_dependency_tree_instead_of_reporting_a_bug() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest"},"devDependencies":{"vitest":"1.0.0"}}"#,
        )
        .unwrap();
        let command = ProjectCommand {
            id: "test:npm".into(),
            label: "npm test".into(),
            kind: ProjectCommandKind::Test,
            executable: "npm".into(),
            args: vec!["test".into()],
            source: "package.json".into(),
            confidence: CommandConfidence::Declared,
        };
        let reason = check_precondition(&command, dir.path()).expect("dependency precondition");
        assert!(reason.contains("dependencies are not present"));
    }

    #[test]
    fn node_precondition_allows_dependency_free_script() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"node test.js"}}"#,
        )
        .unwrap();
        let command = ProjectCommand {
            id: "test:npm".into(),
            label: "npm test".into(),
            kind: ProjectCommandKind::Test,
            executable: "npm".into(),
            args: vec!["test".into()],
            source: "package.json".into(),
            confidence: CommandConfidence::Declared,
        };
        assert!(check_precondition(&command, dir.path()).is_none());
    }

    #[test]
    fn dynamic_package_executor_requires_a_separate_explicit_action() {
        let dir = tempdir().unwrap();
        let command = ProjectCommand {
            id: "check:npx".into(),
            label: "npx eslint".into(),
            kind: ProjectCommandKind::Lint,
            executable: "npx".into(),
            args: vec!["eslint".into(), ".".into()],
            source: "fixture".into(),
            confidence: CommandConfidence::Declared,
        };
        let reason = check_precondition(&command, dir.path()).expect("npx must be blocked");
        assert!(reason.contains("Dynamic package execution"));
    }

    #[test]
    fn git_snapshot_detects_content_changes_when_porcelain_shape_is_unchanged() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "tracked.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);

        std::fs::write(dir.path().join("tracked.txt"), "two\n").unwrap();
        let before = git_snapshot(dir.path()).unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "three\n").unwrap();
        let after = git_snapshot(dir.path()).unwrap();

        assert_eq!(before.status, after.status);
        assert_ne!(before.tracked_diff_sha256, after.tracked_diff_sha256);
        assert_ne!(before, after);
    }

    #[test]
    fn health_run_uses_shadow_and_turns_failure_into_evidence_backed_problem() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("state.txt"), "BAD\n").unwrap();
        git(dir.path(), &["add", "state.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);
        let profile = profile(dir.path());
        let report = run_project_health(
            &profile,
            &HealthRunOptions {
                command_ids: Vec::new(),
                timeout_secs: 30,
                confirmed_execution: true,
            },
        )
        .unwrap();
        assert_eq!(report.status, HealthRunStatus::ProblemsFound);
        assert!(report.original_unchanged);
        assert_eq!(
            report
                .checks
                .iter()
                .filter(|check| check.status == HealthCheckStatus::Passed)
                .count(),
            1
        );
        assert_eq!(
            report
                .checks
                .iter()
                .filter(|check| check.status == HealthCheckStatus::Failed)
                .count(),
            1
        );
        assert_eq!(report.problems.len(), 1);
        assert_eq!(report.problems[0].status, ProblemStatus::Reproduced);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("state.txt")).unwrap(),
            "BAD\n"
        );
        assert!(Command::new("git")
            .current_dir(dir.path())
            .args(["status", "--porcelain"])
            .output()
            .unwrap()
            .stdout
            .is_empty());
    }

    #[test]
    fn report_and_problem_round_trip_through_storage() {
        let db = NamedTempFile::new().unwrap();
        let mut conn = init_db(db.path()).unwrap();
        let report = ProjectHealthReport {
            id: "run-1".into(),
            root_path: "C:/fixture".into(),
            project_name: "fixture".into(),
            base_commit: "abc".into(),
            started_at: 10,
            finished_at: 20,
            status: HealthRunStatus::ProblemsFound,
            original_unchanged: true,
            source_had_local_changes: false,
            checks: Vec::new(),
            problems: vec![ProjectProblemRecord {
                id: "problem-1".into(),
                problem_key: "key-1".into(),
                root_path: "C:/fixture".into(),
                status: ProblemStatus::Reproduced,
                active: true,
                title: "tests failed".into(),
                summary: "assertion failed".into(),
                command_id: "test".into(),
                health_run_id: "run-1".into(),
                check_run_id: "check-1".into(),
                evidence_ids: vec!["e1".into()],
                first_seen_at: 20,
                last_seen_at: 20,
                cleared_at: None,
                occurrences: 1,
            }],
        };
        save_report(&mut conn, &report).unwrap();
        assert_eq!(
            latest_report(&conn, "C:/fixture").unwrap(),
            Some(report.clone())
        );
        let problems = list_project_problems(&conn, "C:/fixture", 10).unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].problem_key, "key-1");
        assert_eq!(problems[0].evidence_ids, vec!["e1"]);
    }
    #[test]
    fn passing_rerun_marks_previous_health_problem_inactive_without_claiming_verified() {
        let db = NamedTempFile::new().unwrap();
        let mut conn = init_db(db.path()).unwrap();
        let mut failing = ProjectHealthReport {
            id: "run-fail".into(),
            root_path: "C:/fixture".into(),
            project_name: "fixture".into(),
            base_commit: "abc".into(),
            started_at: 10,
            finished_at: 20,
            status: HealthRunStatus::ProblemsFound,
            original_unchanged: true,
            source_had_local_changes: false,
            checks: vec![HealthCheckResult {
                id: "check-fail".into(),
                command_id: "test".into(),
                label: "tests".into(),
                kind: ProjectCommandKind::Test,
                executable: "git".into(),
                args: vec!["status".into()],
                status: HealthCheckStatus::Failed,
                exit_code: Some(1),
                duration_ms: 1,
                stdout_preview: String::new(),
                stderr_preview: "failed".into(),
                stdout_truncated: false,
                stderr_truncated: false,
                evidence_id: "health:run-fail:check-fail".into(),
                summary: "failed".into(),
            }],
            problems: vec![ProjectProblemRecord {
                id: "problem-1".into(),
                problem_key: "key-1".into(),
                root_path: "C:/fixture".into(),
                status: ProblemStatus::Reproduced,
                active: true,
                title: "tests failed".into(),
                summary: "failed".into(),
                command_id: "test".into(),
                health_run_id: "run-fail".into(),
                check_run_id: "check-fail".into(),
                evidence_ids: vec!["health:run-fail:check-fail".into()],
                first_seen_at: 20,
                last_seen_at: 20,
                cleared_at: None,
                occurrences: 1,
            }],
        };
        save_report(&mut conn, &failing).unwrap();

        failing.id = "run-pass".into();
        failing.started_at = 30;
        failing.finished_at = 40;
        failing.status = HealthRunStatus::Clean;
        failing.problems.clear();
        failing.checks[0].id = "check-pass".into();
        failing.checks[0].status = HealthCheckStatus::Passed;
        failing.checks[0].exit_code = Some(0);
        failing.checks[0].evidence_id = "health:run-pass:check-pass".into();
        failing.checks[0].summary = "tests passed.".into();
        save_report(&mut conn, &failing).unwrap();

        let problems = list_project_problems(&conn, "C:/fixture", 10).unwrap();
        assert_eq!(problems.len(), 1);
        assert!(!problems[0].active);
        assert_eq!(problems[0].status, ProblemStatus::Reproduced);
        assert_eq!(problems[0].cleared_at, Some(40));
    }

    #[test]
    fn health_report_and_problem_updates_are_atomic() {
        let db = NamedTempFile::new().unwrap();
        let mut conn = init_db(db.path()).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_project_problem_insert BEFORE INSERT ON project_problems BEGIN SELECT RAISE(FAIL, 'forced problem failure'); END;",
        )
        .unwrap();
        let report = ProjectHealthReport {
            id: "run-atomic".into(),
            root_path: "C:/fixture".into(),
            project_name: "fixture".into(),
            base_commit: "abc".into(),
            started_at: 10,
            finished_at: 20,
            status: HealthRunStatus::ProblemsFound,
            original_unchanged: true,
            source_had_local_changes: false,
            checks: Vec::new(),
            problems: vec![ProjectProblemRecord {
                id: "problem-atomic".into(),
                problem_key: "key-atomic".into(),
                root_path: "C:/fixture".into(),
                status: ProblemStatus::Reproduced,
                active: true,
                title: "tests failed".into(),
                summary: "failed".into(),
                command_id: "test".into(),
                health_run_id: "run-atomic".into(),
                check_run_id: "check-atomic".into(),
                evidence_ids: vec!["health:run-atomic:check-atomic".into()],
                first_seen_at: 20,
                last_seen_at: 20,
                cleared_at: None,
                occurrences: 1,
            }],
        };
        assert!(save_report(&mut conn, &report).is_err());
        let run_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM project_health_runs WHERE id='run-atomic'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(run_count, 0);
    }
}
