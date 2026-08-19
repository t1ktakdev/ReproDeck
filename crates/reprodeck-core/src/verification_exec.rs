use crate::evidence::{self, ArtifactRole};
use crate::permissions::{self, Permission, PermissionDecision};
use crate::redaction::{self, RedactionResult};
use crate::runner::{self, CommandError, CommandSpec};
use crate::verification::{self, RunPhase, RunStatus};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerificationExecutionError {
    #[error("verification command denied: {decision:?}")]
    PermissionDenied { decision: PermissionDecision },
    #[error("verification command requires approval: {decision:?}")]
    DecisionRequired { decision: PermissionDecision },
    #[error("unsupported verification expected condition: {0}")]
    UnsupportedExpectedCondition(String),
    #[error(transparent)]
    Verification(#[from] verification::VerificationError),
    #[error(transparent)]
    Evidence(#[from] evidence::EvidenceError),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, VerificationExecutionError>;

#[derive(Debug)]
pub struct VerificationExecutionRequest {
    pub contract_id: String,
    pub check_id: String,
    pub phase: RunPhase,
    pub spec: CommandSpec,
    pub configured_permission: Permission,
    /// True only for a single command the user has just approved in an Ask
    /// prompt. It never bypasses configured Deny or verification hard-denies.
    pub explicitly_approved_once: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationExecutionOutcome {
    pub run_id: String,
    pub receipt_id: String,
    pub phase: RunPhase,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub stdout_artifact_id: Option<String>,
    pub stderr_artifact_id: Option<String>,
    pub runner_issue: Option<String>,
}

fn phase_role(phase: RunPhase) -> ArtifactRole {
    match phase {
        RunPhase::Before => ArtifactRole::Before,
        RunPhase::After => ArtifactRole::After,
    }
}

/// Outcome checks currently support a deliberately small, deterministic
/// condition language. An absent condition means the conventional `exit 0`.
/// Anything richer is rejected instead of being silently interpreted as pass.
fn parse_expected_exit_code(condition: Option<&str>) -> Result<i32> {
    let Some(condition) = condition else {
        return Ok(0);
    };
    let condition = condition.trim();
    if condition.is_empty() {
        return Ok(0);
    }

    let lower = condition.to_ascii_lowercase();
    if let Some(value) = lower.strip_prefix("exit ") {
        return value
            .trim()
            .parse::<i32>()
            .map_err(|_| VerificationExecutionError::UnsupportedExpectedCondition(condition.into()));
    }

    let compact: String = lower.chars().filter(|ch| !ch.is_whitespace()).collect();
    for prefix in ["exit_code==", "exit_code=", "exit==", "exit="] {
        if let Some(value) = compact.strip_prefix(prefix) {
            return value.parse::<i32>().map_err(|_| {
                VerificationExecutionError::UnsupportedExpectedCondition(condition.into())
            });
        }
    }

    Err(VerificationExecutionError::UnsupportedExpectedCondition(
        condition.into(),
    ))
}

fn get_check(
    conn: &Connection,
    contract_id: &str,
    check_id: &str,
) -> Result<verification::VerificationCheck> {
    verification::list_verification_checks(conn, contract_id)?
        .into_iter()
        .find(|check| check.id == check_id)
        .ok_or_else(|| verification::VerificationError::CheckNotFound(check_id.to_owned()).into())
}

fn redacted_command_meta(
    request: &VerificationExecutionRequest,
    expected_exit_code: i32,
) -> serde_json::Value {
    let spec = &request.spec;
    let args: Vec<String> = spec
        .args
        .iter()
        .map(|arg| redaction::redact_text(arg))
        .collect();
    let env = spec.env.as_ref().map(|values| {
        values
            .iter()
            .map(|(key, value)| {
                let display = match redaction::redact_env(key, value) {
                    RedactionResult::Included(value) => value,
                    RedactionResult::Redacted { reason } => format!("[REDACTED: {reason}]"),
                    RedactionResult::Excluded { reason } => format!("[EXCLUDED: {reason}]"),
                };
                (key.clone(), display)
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    });

    serde_json::json!({
        "contract_id": request.contract_id,
        "check_id": request.check_id,
        "phase": request.phase.to_string(),
        "expected_exit_code": expected_exit_code,
        "command": {
            "executable": redaction::redact_text(&spec.executable),
            "args": args,
            "cwd": spec.cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
            "env": env,
            "timeout_ms": spec.timeout.map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64),
            "output_limit": spec.output_limit,
        }
    })
}

fn runner_issue(error: &CommandError) -> (&'static str, RunStatus) {
    match error {
        CommandError::Cancelled => ("cancelled", RunStatus::Interrupted),
        CommandError::Timeout => ("timeout", RunStatus::Error),
        CommandError::SpawnFailed(_) => ("spawn_failed", RunStatus::Error),
        CommandError::Io(_) => ("io_error", RunStatus::Error),
        CommandError::OutputLimitExceeded => ("output_limit_exceeded", RunStatus::Error),
        CommandError::PermissionDenied => {
            ("permission_denied_after_authorization", RunStatus::Error)
        }
        CommandError::DecisionRequired => {
            ("decision_required_after_authorization", RunStatus::Error)
        }
    }
}

fn persist_output(
    conn: &Connection,
    storage_dir: &Path,
    receipt_id: &str,
    run_id: &str,
    role: ArtifactRole,
    text: &str,
) -> Result<Option<String>> {
    if text.is_empty() {
        return Ok(None);
    }
    let artifact = evidence::persist_text_artifact(
        conn,
        storage_dir,
        receipt_id,
        text,
        Some("text/plain; charset=utf-8"),
    )?;
    evidence::link_artifact(conn, &artifact.id, Some(run_id), role)?;
    Ok(Some(artifact.id))
}

/// Execute one BEFORE/AFTER verification check through the accepted runner.
/// Permission and expected-condition validation happen before any run is
/// created. Once a run starts, every runner termination path is persisted as a
/// finished verification run and Timeline receipt.
pub fn execute_verification_check(
    conn: &mut Connection,
    storage_dir: &Path,
    request: VerificationExecutionRequest,
    cancel_token: Option<Arc<AtomicBool>>,
) -> Result<VerificationExecutionOutcome> {
    let check = get_check(conn, &request.contract_id, &request.check_id)?;
    let expected_exit_code = parse_expected_exit_code(check.expected_condition.as_deref())?;

    let decision = permissions::verification_command_permission_with_approval(
        &request.spec.executable,
        &request.spec.args,
        request.configured_permission,
        request.explicitly_approved_once,
    );
    match decision.permission {
        Permission::Deny => {
            return Err(VerificationExecutionError::PermissionDenied { decision });
        }
        Permission::Ask => {
            return Err(VerificationExecutionError::DecisionRequired { decision });
        }
        Permission::Allow => {}
    }

    let command_meta = redacted_command_meta(&request, expected_exit_code);
    let run_id = verification::start_verification_check_run(
        conn,
        &request.contract_id,
        &request.check_id,
        request.phase,
    )?;
    conn.execute(
        "UPDATE actions SET meta = ?1 WHERE id = ?2",
        rusqlite::params![serde_json::to_string(&command_meta)?, &run_id],
    )?;

    match runner::run_command(request.spec, Permission::Allow, cancel_token) {
        Ok(result) => {
            let status = if result.exit_code == Some(expected_exit_code) {
                RunStatus::Passed
            } else {
                RunStatus::Failed
            };
            let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
            let receipt_id = verification::finish_verification_run_with_output(
                conn,
                &run_id,
                status,
                Some(&stdout),
                Some(&stderr),
            )?;
            let stdout_artifact_id = persist_output(
                conn,
                storage_dir,
                &receipt_id,
                &run_id,
                phase_role(request.phase),
                &stdout,
            )?;
            let stderr_artifact_id = persist_output(
                conn,
                storage_dir,
                &receipt_id,
                &run_id,
                ArtifactRole::Diagnostic,
                &stderr,
            )?;
            Ok(VerificationExecutionOutcome {
                run_id,
                receipt_id,
                phase: request.phase,
                status,
                exit_code: result.exit_code,
                stdout_artifact_id,
                stderr_artifact_id,
                runner_issue: None,
            })
        }
        Err(error) => {
            let (issue, status) = runner_issue(&error);
            let diagnostic = format!("Verification runner ended with: {issue}");
            let receipt_id = verification::finish_verification_run_with_output(
                conn,
                &run_id,
                status,
                None,
                Some(&diagnostic),
            )?;
            let stderr_artifact_id = persist_output(
                conn,
                storage_dir,
                &receipt_id,
                &run_id,
                ArtifactRole::Diagnostic,
                &diagnostic,
            )?;
            Ok(VerificationExecutionOutcome {
                run_id,
                receipt_id,
                phase: request.phase,
                status,
                exit_code: None,
                stdout_artifact_id: None,
                stderr_artifact_id,
                runner_issue: Some(issue.to_string()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::{tempdir, NamedTempFile};

    fn setup() -> (
        NamedTempFile,
        Connection,
        verification::OutcomeContract,
        verification::VerificationCheck,
    ) {
        let db_file = NamedTempFile::new().unwrap();
        let conn = init_db(db_file.path()).unwrap();
        conn.execute(
            "INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES ('session','repo',1,1,'Active')",
            [],
        )
        .unwrap();
        let contract =
            verification::create_outcome_contract(&conn, "session", "Regression", None).unwrap();
        let check = verification::add_verification_check(
            &conn,
            &contract.id,
            "git-version",
            "Git command completes",
            Some("git --version"),
            Some("exit 0"),
            true,
            0,
        )
        .unwrap();
        (db_file, conn, contract, check)
    }

    fn git_spec(args: &[&str]) -> CommandSpec {
        CommandSpec {
            executable: "git".to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            cwd: None,
            env: None,
            timeout: Some(std::time::Duration::from_secs(10)),
            output_limit: Some(64 * 1024),
        }
    }

    fn request(
        contract: &verification::OutcomeContract,
        check: &verification::VerificationCheck,
        phase: RunPhase,
        spec: CommandSpec,
        permission: Permission,
    ) -> VerificationExecutionRequest {
        VerificationExecutionRequest {
            contract_id: contract.id.clone(),
            check_id: check.id.clone(),
            phase,
            spec,
            configured_permission: permission,
            explicitly_approved_once: false,
        }
    }

    fn run_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM verification_runs", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn expected_condition_parser_is_deliberately_small() {
        assert_eq!(parse_expected_exit_code(None).unwrap(), 0);
        assert_eq!(parse_expected_exit_code(Some("exit 0")).unwrap(), 0);
        assert_eq!(
            parse_expected_exit_code(Some("exit_code == 17")).unwrap(),
            17
        );
        assert_eq!(parse_expected_exit_code(Some("exit=-1")).unwrap(), -1);
        assert!(matches!(
            parse_expected_exit_code(Some("stdout contains success")),
            Err(VerificationExecutionError::UnsupportedExpectedCondition(_))
        ));
    }

    #[test]
    fn unsupported_expectation_is_rejected_before_creating_run() {
        let (_db, mut conn, contract, mut check) = setup();
        check.expected_condition = Some("stdout contains success".to_string());
        verification::update_verification_check(&conn, &check).unwrap();
        let storage = tempdir().unwrap();
        let result = execute_verification_check(
            &mut conn,
            storage.path(),
            request(
                &contract,
                &check,
                RunPhase::Before,
                git_spec(&["--version"]),
                Permission::Allow,
            ),
            None,
        );
        assert!(matches!(
            result,
            Err(VerificationExecutionError::UnsupportedExpectedCondition(_))
        ));
        assert_eq!(run_count(&conn), 0);
    }

    #[test]
    fn ask_returns_decision_required_before_creating_run() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let result = execute_verification_check(
            &mut conn,
            storage.path(),
            request(
                &contract,
                &check,
                RunPhase::Before,
                git_spec(&["--version"]),
                Permission::Ask,
            ),
            None,
        );
        assert!(matches!(
            result,
            Err(VerificationExecutionError::DecisionRequired { .. })
        ));
        assert_eq!(run_count(&conn), 0);
    }

    #[test]
    fn one_shot_approval_satisfies_ask_and_creates_run() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let mut request = request(
            &contract,
            &check,
            RunPhase::Before,
            git_spec(&["--version"]),
            Permission::Ask,
        );
        request.explicitly_approved_once = true;
        let outcome = execute_verification_check(
            &mut conn,
            storage.path(),
            request,
            None,
        )
        .unwrap();
        assert_eq!(outcome.status, RunStatus::Passed);
        assert_eq!(run_count(&conn), 1);
    }

    #[test]
    fn deny_returns_permission_denied_before_creating_run() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let result = execute_verification_check(
            &mut conn,
            storage.path(),
            request(
                &contract,
                &check,
                RunPhase::Before,
                git_spec(&["--version"]),
                Permission::Deny,
            ),
            None,
        );
        assert!(matches!(
            result,
            Err(VerificationExecutionError::PermissionDenied { .. })
        ));
        assert_eq!(run_count(&conn), 0);
    }

    #[test]
    fn allowed_command_creates_real_receipt_and_phase_evidence() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let outcome = execute_verification_check(
            &mut conn,
            storage.path(),
            request(
                &contract,
                &check,
                RunPhase::Before,
                git_spec(&["--version"]),
                Permission::Allow,
            ),
            None,
        )
        .unwrap();
        assert_eq!(outcome.status, RunStatus::Passed);
        assert_eq!(outcome.exit_code, Some(0));
        let run = verification::get_verification_run(&conn, &outcome.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.receipt_id.as_deref(), Some(outcome.receipt_id.as_str()));
        let receipt = crate::timeline::get_receipt(&conn, &outcome.receipt_id)
            .unwrap()
            .unwrap();
        assert!(receipt.stdout_preview.unwrap().contains("git version"));
        let artifact_id = outcome.stdout_artifact_id.expect("stdout artifact");
        let bytes = evidence::read_artifact(&conn, storage.path(), &artifact_id).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("git version"));
        let links = evidence::list_artifact_links_for_run(&conn, &outcome.run_id).unwrap();
        assert!(links.iter().any(|link| link.role == ArtifactRole::Before));
    }

    #[test]
    fn nonzero_exit_is_failed_not_runner_error() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let outcome = execute_verification_check(
            &mut conn,
            storage.path(),
            request(
                &contract,
                &check,
                RunPhase::After,
                git_spec(&[
                    "rev-parse",
                    "--verify",
                    "refs/heads/reprodeck-definitely-missing",
                ]),
                Permission::Allow,
            ),
            None,
        )
        .unwrap();
        assert_eq!(outcome.status, RunStatus::Failed);
        assert!(outcome.runner_issue.is_none());
        assert_ne!(outcome.exit_code, Some(0));
    }

    #[test]
    fn pre_cancelled_run_is_persisted_as_interrupted() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let token = Arc::new(AtomicBool::new(true));
        let outcome = execute_verification_check(
            &mut conn,
            storage.path(),
            request(
                &contract,
                &check,
                RunPhase::After,
                git_spec(&["--version"]),
                Permission::Allow,
            ),
            Some(token),
        )
        .unwrap();
        assert_eq!(outcome.status, RunStatus::Interrupted);
        assert_eq!(outcome.runner_issue.as_deref(), Some("cancelled"));
        let run = verification::get_verification_run(&conn, &outcome.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, RunStatus::Interrupted);
        assert!(run.receipt_id.is_some());
    }

    #[test]
    fn mutating_git_is_hard_denied_from_verification() {
        let (_db, mut conn, contract, check) = setup();
        let storage = tempdir().unwrap();
        let mut request = request(
            &contract,
            &check,
            RunPhase::Before,
            git_spec(&["push"]),
            Permission::Allow,
        );
        request.explicitly_approved_once = true;
        let result = execute_verification_check(
            &mut conn,
            storage.path(),
            request,
            None,
        );
        assert!(matches!(
            result,
            Err(VerificationExecutionError::PermissionDenied { .. })
        ));
        assert_eq!(run_count(&conn), 0);
    }
}
