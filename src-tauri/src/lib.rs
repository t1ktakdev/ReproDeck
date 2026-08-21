use reprodeck_core::ai::AiProvider;
use reprodeck_core::{
    ai, bug_hunter, capsule, context_compiler, db, demo, evidence, github, project_health,
    project_intelligence, recovery, redaction, repository, root_cause, settings, shadow_session,
    state_machine, timeline, verification, workflow,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BridgeError {
    code: String,
    message: String,
}
impl BridgeError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    fn storage(message: impl Into<String>) -> Self {
        Self::new("storage_error", message)
    }
}
impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for BridgeError {}

#[derive(Debug, Clone, Serialize)]
struct RuntimeHealth {
    runtime: &'static str,
    bridge_version: u32,
}

#[derive(Debug, Clone, Serialize)]
struct TimelineEntry {
    action: timeline::ActionRecord,
    execution: Option<timeline::ExecutionRecord>,
    receipt: Option<timeline::ReceiptRecord>,
    artifacts: Vec<evidence::ArtifactRecord>,
}

fn app_data_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.data_local_dir().join("reprodeck"))
        .unwrap_or_else(|| std::env::temp_dir().join("reprodeck"))
}
fn app_db_path() -> PathBuf {
    app_data_dir().join("reprodeck.db")
}
fn artifact_dir() -> PathBuf {
    app_data_dir().join("artifacts")
}
fn capsule_dir() -> PathBuf {
    app_data_dir().join("capsules")
}

fn capture_shadow_diff_evidence(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    source: &str,
) -> Result<(), BridgeError> {
    let diff = shadow_session::session_shadow_diff(conn, session_id).map_err(map_shadow)?;
    if diff.patch.is_empty() {
        return Ok(());
    }
    let sensitive = diff.files.iter().any(|path| {
        !matches!(
            redaction::redact_path(Path::new(path)),
            redaction::RedactionResult::Included(_)
        )
    });
    let action = timeline::new_action(
        session_id,
        "changes:diff",
        "Succeeded",
        Some(serde_json::json!({"files": diff.files, "source": source, "sensitive_paths_omitted": sensitive}).to_string()),
    ).map_err(|error| BridgeError::storage(error.to_string()))?;
    timeline::create_action(conn, &action)
        .map_err(|error| BridgeError::storage(error.to_string()))?;
    let execution_id = timeline::start_execution(conn, &action.id)
        .map_err(|error| BridgeError::storage(error.to_string()))?;
    let receipt_id = timeline::finish_execution(
        conn,
        &execution_id,
        "Succeeded",
        None,
        if sensitive {
            Some("Patch payload omitted because a changed path matched the secret-deny rules.")
        } else {
            None
        },
    )
    .map_err(|error| BridgeError::storage(error.to_string()))?;
    let artifact = if sensitive {
        None
    } else {
        Some(
            evidence::persist_text_artifact(
                conn,
                &artifact_dir(),
                &receipt_id,
                &diff.patch,
                Some("text/x-diff"),
            )
            .map_err(|error| BridgeError::storage(error.to_string()))?,
        )
    };
    evidence::create_evidence_item(
        conn,
        evidence::NewEvidenceItem {
            session_id,
            action_id: Some(&action.id),
            receipt_id: Some(&receipt_id),
            kind: evidence::EvidenceKind::GitDiff,
            source,
            summary: if sensitive {
                "Git diff captured; payload omitted by secret path policy."
            } else {
                "Reviewed Git diff captured from the isolated workspace."
            },
            artifact: artifact.as_ref(),
        },
    )
    .map_err(|error| BridgeError::storage(error.to_string()))?;
    Ok(())
}

fn open_conn() -> Result<rusqlite::Connection, BridgeError> {
    std::fs::create_dir_all(app_data_dir())
        .map_err(|_| BridgeError::storage("Unable to create ReproDeck local data directory."))?;
    db::init_db(&app_db_path())
        .map_err(|_| BridgeError::storage("Unable to initialize the ReproDeck database."))
}

fn map_repo(error: repository::RepositoryError) -> BridgeError {
    use repository::RepositoryError::*;
    match error {
        SessionNotFound(_) => BridgeError::new("not_found", "Session not found."),
        UnbornRepository => BridgeError::new(
            "repository_unborn",
            "The repository needs at least one commit before ReproDeck can use it.",
        ),
        NonUtf8Path => BridgeError::new(
            "repository_path_unsupported",
            "The selected repository path cannot be represented safely.",
        ),
        _ => BridgeError::new(
            "repository_invalid",
            "The selected path is not an accessible Git working repository.",
        ),
    }
}

fn map_shadow(error: shadow_session::ShadowSessionError) -> BridgeError {
    use shadow_session::ShadowSessionError::*;
    match error {
        SessionNotFound(_) => BridgeError::new("not_found", "Session not found."),
        RepositoryNotAttached(_) => BridgeError::new("repository_required", "Attach a Git repository first."),
        ShadowNotFound(_) => BridgeError::new("shadow_not_found", "This session has no isolated workspace."),
        StaleShadow => BridgeError::new("shadow_stale", "The isolated workspace can no longer be resumed safely."),
        NoChanges => BridgeError::new("no_changes", "There are no uncommitted changes in the isolated workspace."),
        AppliedStateCleanupFailed => BridgeError::new("applied_cleanup_pending", "The patch was applied, but ReproDeck could not clear the local shadow record. Do not apply it again."),
        DiscardedStateCleanupFailed => BridgeError::new("discarded_cleanup_pending", "The workspace was discarded, but ReproDeck could not clear its local record."),
        other => BridgeError::new("shadow_error", other.to_string()),
    }
}

fn map_state(error: state_machine::StateError) -> BridgeError {
    match error {
        state_machine::StateError::SessionNotFound(_) => BridgeError::new("not_found", "Session not found."),
        state_machine::StateError::InvalidTransition { from, to } => BridgeError::new(
            "invalid_session_state",
            format!("This action is not available while the session is {from}. Expected a valid transition to {to}."),
        ),
        other => BridgeError::new("session_state_error", other.to_string()),
    }
}

fn map_workflow(error: workflow::WorkflowError) -> BridgeError {
    match error {
        workflow::WorkflowError::ApprovalRequired(message) => BridgeError::new("approval_required", message),
        workflow::WorkflowError::PermissionDenied(message) => BridgeError::new("permission_denied", message),
        workflow::WorkflowError::SessionNotFound(_) => BridgeError::new("not_found", "Session not found."),
        workflow::WorkflowError::StepNotFound(_) => BridgeError::new("not_found", "Reproduction step not found."),
        workflow::WorkflowError::BaselineLocked => BridgeError::new("baseline_locked", "The current Before result is protected. Reset the baseline explicitly before running Before again."),
        workflow::WorkflowError::BaselineMissing => BridgeError::new("baseline_missing", "There is no Before baseline to reset in the active verification cycle."),
        other => BridgeError::new("workflow_error", other.to_string()),
    }
}

fn map_verification(error: verification::VerificationError) -> BridgeError {
    use verification::VerificationError::*;
    match error {
        ApprovalRequired(message) => BridgeError::new("approval_required", message),
        PermissionDenied(message) => BridgeError::new("permission_denied", message),
        HandoffHashMismatch | TransferredPatchMismatch => BridgeError::new(
            "patch_identity_mismatch",
            "The investigation patch no longer matches its recorded SHA-256 identity.",
        ),
        HandoffSourceMismatch => BridgeError::new(
            "source_commit_mismatch",
            "The investigation and verification workspace do not share the same source commit.",
        ),
        BaselineNotFailed => BridgeError::new(
            "baseline_required",
            "Capture the failing Before baseline before transferring the investigation patch.",
        ),
        UncheckpointedAfter => BridgeError::new(
            "checkpoint_required",
            "Checkpoint every workspace change before running After.",
        ),
        ProofMismatch => BridgeError::new(
            "verification_required",
            "Apply is blocked because the current patch is not the exact verified patch.",
        ),
        RegressionDemotion => BridgeError::new(
            "regression_demotion_blocked",
            "Required verification checks cannot be demoted.",
        ),
        other => BridgeError::new("verification_error", other.to_string()),
    }
}

#[tauri::command]
fn runtime_health() -> RuntimeHealth {
    RuntimeHealth {
        runtime: "tauri",
        bridge_version: 5,
    }
}

#[tauri::command]
fn list_sessions() -> Result<Vec<timeline::SessionRecord>, BridgeError> {
    let conn = open_conn()?;
    timeline::list_sessions(&conn, None, 200).map_err(|e| BridgeError::storage(e.to_string()))
}

#[tauri::command]
fn create_bug_session(
    id: String,
    meta: workflow::SessionMeta,
) -> Result<timeline::SessionRecord, BridgeError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "Session ID cannot be empty.",
        ));
    }
    if id.len() > 96 {
        return Err(BridgeError::new(
            "invalid_request",
            "Session ID is too long.",
        ));
    }
    let conn = open_conn()?;
    workflow::create_bug_session(&conn, id, &meta).map_err(map_workflow)
}

#[tauri::command]
fn inspect_repository(path: String) -> Result<repository::RepositoryInfo, BridgeError> {
    if path.trim().is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "Repository path cannot be empty.",
        ));
    }
    repository::inspect_repository(Path::new(path.trim())).map_err(map_repo)
}

#[tauri::command]
fn attach_repository(
    session_id: String,
    path: String,
) -> Result<repository::RepositoryInfo, BridgeError> {
    let mut conn = open_conn()?;
    let info =
        repository::attach_repository_to_session(&mut conn, &session_id, Path::new(path.trim()))
            .map_err(map_repo)?;
    if let Some(session) = timeline::get_session_record(&conn, &session_id)
        .map_err(|error| BridgeError::storage(error.to_string()))?
    {
        if session.state == "Draft" {
            state_machine::transition_session(
                &conn,
                &session_id,
                state_machine::SessionState::Preparing,
            )
            .map_err(map_state)?;
        }
    }
    Ok(info)
}

#[tauri::command]
fn get_session_repository(
    session_id: String,
) -> Result<Option<repository::RepositoryInfo>, BridgeError> {
    let conn = open_conn()?;
    repository::get_session_repository(&conn, &session_id).map_err(map_repo)
}

#[tauri::command]
fn capture_environment(session_id: String) -> Result<workflow::EnvironmentSnapshot, BridgeError> {
    let conn = open_conn()?;
    if let Some(session) = timeline::get_session_record(&conn, &session_id)
        .map_err(|error| BridgeError::storage(error.to_string()))?
    {
        if matches!(session.state.as_str(), "Preparing" | "Draft") {
            if session.state == "Draft" {
                state_machine::transition_session(
                    &conn,
                    &session_id,
                    state_machine::SessionState::Preparing,
                )
                .map_err(map_state)?;
            }
            state_machine::transition_session(
                &conn,
                &session_id,
                state_machine::SessionState::CapturingEnvironment,
            )
            .map_err(map_state)?;
        }
    }
    workflow::capture_environment(&conn, &session_id).map_err(map_workflow)
}

#[tauri::command]
fn latest_environment(
    session_id: String,
) -> Result<Option<workflow::EnvironmentSnapshot>, BridgeError> {
    let conn = open_conn()?;
    workflow::latest_environment(&conn, &session_id).map_err(map_workflow)
}

#[tauri::command]
fn add_reproduction_step(
    session_id: String,
    executable: String,
    args: Vec<String>,
    expected_exit_code: i32,
) -> Result<workflow::ReproductionStep, BridgeError> {
    if executable.trim().is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "Executable cannot be empty.",
        ));
    }
    let conn = open_conn()?;
    workflow::add_reproduction_step(&conn, &session_id, &executable, &args, expected_exit_code)
        .map_err(map_workflow)
}

#[tauri::command]
fn list_reproduction_steps(
    session_id: String,
) -> Result<Vec<workflow::ReproductionStep>, BridgeError> {
    let conn = open_conn()?;
    workflow::list_reproduction_steps(&conn, &session_id).map_err(map_workflow)
}

#[tauri::command]
fn list_reproduction_runs(
    session_id: String,
) -> Result<Vec<workflow::ReproductionRun>, BridgeError> {
    let conn = open_conn()?;
    workflow::list_reproduction_runs(&conn, &session_id).map_err(map_workflow)
}

#[tauri::command]
fn execute_reproduction_step(
    step_id: String,
    phase: workflow::ReproductionPhase,
    approved_once: bool,
) -> Result<workflow::RunOutcome, BridgeError> {
    let mut conn = open_conn()?;
    std::fs::create_dir_all(artifact_dir())
        .map_err(|_| BridgeError::storage("Unable to create the evidence store."))?;
    workflow::execute_reproduction_step(&mut conn, &artifact_dir(), &step_id, phase, approved_once)
        .map_err(map_workflow)
}

#[tauri::command]
fn reproduction_outcome(step_id: String) -> Result<String, BridgeError> {
    let conn = open_conn()?;
    workflow::outcome_for_step(&conn, &step_id).map_err(map_workflow)
}

#[tauri::command]
fn session_verification_outcome(session_id: String) -> Result<String, BridgeError> {
    let conn = open_conn()?;
    workflow::outcome_for_session(&conn, &session_id).map_err(map_workflow)
}

#[tauri::command]
fn session_verification_status(
    session_id: String,
) -> Result<verification::VerificationStatus, BridgeError> {
    let conn = open_conn()?;
    verification::status(&conn, &session_id).map_err(map_verification)
}

#[tauri::command]
fn promote_regression_check(
    check_id: String,
    level: verification::RegressionLevel,
) -> Result<verification::RegressionCheck, BridgeError> {
    let conn = open_conn()?;
    verification::promote_regression(&conn, &check_id, level).map_err(map_verification)
}

#[tauri::command]
fn run_regression_check(
    check_id: String,
    approved_once: bool,
) -> Result<verification::RegressionRunOutcome, BridgeError> {
    let mut conn = open_conn()?;
    std::fs::create_dir_all(artifact_dir())
        .map_err(|_| BridgeError::storage("Unable to create the evidence store."))?;
    verification::run_regression(&mut conn, &artifact_dir(), &check_id, approved_once)
        .map_err(map_verification)
}

#[tauri::command]
fn get_shadow_workspace(
    session_id: String,
) -> Result<Option<shadow_session::ShadowWorkspaceRecord>, BridgeError> {
    let conn = open_conn()?;
    shadow_session::get_session_shadow(&conn, &session_id).map_err(map_shadow)
}

#[tauri::command]
fn create_shadow_workspace(
    session_id: String,
) -> Result<shadow_session::ShadowWorkspaceRecord, BridgeError> {
    let conn = open_conn()?;
    let session = timeline::get_session_record(&conn, &session_id)
        .map_err(|error| BridgeError::storage(error.to_string()))?
        .ok_or_else(|| BridgeError::new("not_found", "Session not found."))?;
    if matches!(
        session.state.as_str(),
        "Preparing" | "CapturingEnvironment" | "Draft"
    ) {
        if session.state == "Draft" {
            state_machine::transition_session(
                &conn,
                &session_id,
                state_machine::SessionState::Preparing,
            )
            .map_err(map_state)?;
        }
        state_machine::transition_session(
            &conn,
            &session_id,
            state_machine::SessionState::CreatingWorkspace,
        )
        .map_err(map_state)?;
    }
    let shadow = shadow_session::create_session_shadow(&conn, &session_id).map_err(map_shadow)?;
    let current = timeline::get_session_record(&conn, &session_id)
        .map_err(|error| BridgeError::storage(error.to_string()))?;
    if current
        .as_ref()
        .is_some_and(|value| value.state == "CreatingWorkspace")
    {
        state_machine::transition_session(&conn, &session_id, state_machine::SessionState::Ready)
            .map_err(map_state)?;
    }
    Ok(shadow)
}

#[tauri::command]
fn finalize_shadow_workspace(
    session_id: String,
) -> Result<shadow_session::ShadowWorkspaceRecord, BridgeError> {
    let mut conn = open_conn()?;
    shadow_session::finalize_session_shadow(&conn, &session_id).map_err(map_shadow)?;
    capture_shadow_diff_evidence(&mut conn, &session_id, "checkpoint")?;
    if let Some(session) = timeline::get_session_record(&conn, &session_id)
        .map_err(|error| BridgeError::storage(error.to_string()))?
    {
        if matches!(
            session.state.as_str(),
            "Ready" | "FailureCaptured" | "Verified" | "ReadyToApply"
        ) {
            state_machine::transition_session(
                &conn,
                &session_id,
                state_machine::SessionState::Fixing,
            )
            .map_err(map_state)?;
        }
    }
    shadow_session::get_session_shadow(&conn, &session_id)
        .map_err(map_shadow)?
        .ok_or_else(|| {
            BridgeError::new(
                "shadow_not_found",
                "Workspace disappeared after checkpoint.",
            )
        })
}

#[tauri::command]
fn shadow_diff(session_id: String) -> Result<shadow_session::ShadowDiff, BridgeError> {
    let conn = open_conn()?;
    shadow_session::session_shadow_diff(&conn, &session_id).map_err(map_shadow)
}

#[tauri::command]
fn apply_shadow_workspace(session_id: String, confirmed: bool) -> Result<(), BridgeError> {
    if !confirmed {
        return Err(BridgeError::new(
            "confirmation_required",
            "Applying changes requires explicit confirmation.",
        ));
    }
    let mut conn = open_conn()?;
    let proof_status = verification::status(&conn, &session_id).map_err(map_verification)?;
    if !proof_status.ready_to_apply {
        return Err(BridgeError::new(
            "verification_required",
            proof_status.message,
        ));
    }
    capture_shadow_diff_evidence(&mut conn, &session_id, "pre-apply")?;
    state_machine::transition_session(&conn, &session_id, state_machine::SessionState::Applying)
        .map_err(map_state)?;
    match verification::apply_verified(&conn, &session_id) {
        Ok(()) => {
            let apply_action = timeline::new_action(
                &session_id,
                "changes:apply",
                "Succeeded",
                Some(
                    serde_json::json!({
                        "patch_sha256": proof_status.current_identity.as_ref().map(|value| &value.patch_sha256),
                        "source_commit": proof_status.current_identity.as_ref().map(|value| &value.source_commit),
                        "required_regressions": { "passed": proof_status.required_passed, "total": proof_status.required_total },
                        "original_head_unchanged": true,
                        "explicit_confirmation": true,
                    })
                    .to_string(),
                ),
            );
            if let Ok(action) = apply_action {
                if let Err(error) = timeline::create_action(&conn, &action) {
                    tracing::warn!(session_id = %session_id, error = %error, "applied patch but could not persist the compact Apply receipt");
                }
            }
            state_machine::transition_session(
                &conn,
                &session_id,
                state_machine::SessionState::Applied,
            )
            .map_err(map_state)?;
            tracing::info!(session_id = %session_id, "applied verified shadow patch");
            Ok(())
        }
        Err(error) => {
            // The shadow layer distinguishes "applied but local record cleanup failed" from
            // a true apply failure. Do not rewrite that state as a normal failure here.
            if matches!(
                &error,
                verification::VerificationError::Shadow(
                    shadow_session::ShadowSessionError::AppliedStateCleanupFailed
                )
            ) {
                let _ = state_machine::transition_session(
                    &conn,
                    &session_id,
                    state_machine::SessionState::Applied,
                );
                return Err(map_verification(error));
            }
            let _ = timeline::update_session_state(&conn, &session_id, "ReadyToApply");
            tracing::warn!(session_id = %session_id, error = %error, "shadow apply failed safely");
            Err(map_verification(error))
        }
    }
}

#[tauri::command]
fn discard_shadow_workspace(session_id: String, confirmed: bool) -> Result<(), BridgeError> {
    if !confirmed {
        return Err(BridgeError::new(
            "confirmation_required",
            "Discarding the isolated workspace requires explicit confirmation.",
        ));
    }
    let conn = open_conn()?;
    shadow_session::discard_session_shadow(&conn, &session_id).map_err(map_shadow)?;
    state_machine::transition_session(&conn, &session_id, state_machine::SessionState::Discarded)
        .map_err(map_state)?;
    Ok(())
}

#[tauri::command]
fn list_timeline_entries(session_id: String) -> Result<Vec<TimelineEntry>, BridgeError> {
    let conn = open_conn()?;
    let actions = timeline::list_actions(&conn, &session_id, None, 500)
        .map_err(|e| BridgeError::storage(e.to_string()))?;
    let mut entries = Vec::with_capacity(actions.len());
    for action in actions {
        let execution = timeline::list_executions(&conn, &action.id)
            .map_err(|e| BridgeError::storage(e.to_string()))?
            .into_iter()
            .last();
        let (receipt, artifacts) = if let Some(execution) = execution.as_ref() {
            let receipt = timeline::list_receipts(&conn, &execution.id)
                .map_err(|e| BridgeError::storage(e.to_string()))?
                .into_iter()
                .last();
            let artifacts = if let Some(receipt) = receipt.as_ref() {
                evidence::list_artifacts_for_receipt(&conn, &receipt.id)
                    .map_err(|e| BridgeError::storage(e.to_string()))?
            } else {
                Vec::new()
            };
            (receipt, artifacts)
        } else {
            (None, Vec::new())
        };
        entries.push(TimelineEntry {
            action,
            execution,
            receipt,
            artifacts,
        });
    }
    Ok(entries)
}

#[tauri::command]
fn read_artifact_text(artifact_id: String) -> Result<String, BridgeError> {
    let conn = open_conn()?;
    let bytes = evidence::read_artifact(&conn, &artifact_dir(), &artifact_id)
        .map_err(|e| BridgeError::storage(e.to_string()))?;
    String::from_utf8(bytes).map_err(|_| {
        BridgeError::new(
            "binary_artifact",
            "This artifact is binary and cannot be shown as text.",
        )
    })
}

#[tauri::command]
fn reset_reproduction_baseline(
    step_id: String,
    confirmed: bool,
) -> Result<workflow::ReproductionStep, BridgeError> {
    if !confirmed {
        return Err(BridgeError::new(
            "confirmation_required",
            "Resetting the Before baseline requires explicit confirmation.",
        ));
    }
    let conn = open_conn()?;
    workflow::reset_reproduction_baseline(&conn, &step_id).map_err(map_workflow)
}

#[tauri::command]
fn list_evidence_items(session_id: String) -> Result<Vec<evidence::EvidenceItem>, BridgeError> {
    let conn = open_conn()?;
    evidence::list_evidence_items(&conn, &session_id, 1000)
        .map_err(|error| BridgeError::storage(error.to_string()))
}

#[tauri::command]
fn list_repositories() -> Result<Vec<repository::StoredRepository>, BridgeError> {
    let conn = open_conn()?;
    repository::list_repositories(&conn).map_err(map_repo)
}

#[tauri::command]
fn analyze_project(path: String) -> Result<project_intelligence::ProjectProfile, BridgeError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(BridgeError::new(
            "invalid_project_path",
            "Choose a local project directory first.",
        ));
    }
    let profile = project_intelligence::analyze_project(Path::new(path))
        .map_err(|error| BridgeError::new("project_analysis_error", error.to_string()))?;
    let conn = open_conn()?;
    project_intelligence::save_profile(&conn, &profile)
        .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))?;
    Ok(profile)
}

#[tauri::command]
fn create_demo_project() -> Result<project_intelligence::ProjectProfile, BridgeError> {
    let path = demo::create_fixture()
        .map_err(|error| BridgeError::new("demo_fixture_error", error.to_string()))?;
    analyze_project(path)
}

#[tauri::command]
fn list_project_profiles() -> Result<Vec<project_intelligence::ProjectProfile>, BridgeError> {
    let conn = open_conn()?;
    project_intelligence::list_profiles(&conn, 200)
        .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))
}

#[tauri::command]
fn compile_project_context(
    path: String,
    query: String,
    max_files: Option<usize>,
    max_chars: Option<usize>,
) -> Result<context_compiler::ContextPacket, BridgeError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(BridgeError::new(
            "invalid_project_path",
            "Choose a local project directory first.",
        ));
    }
    if query.trim().is_empty() {
        return Err(BridgeError::new(
            "invalid_context_query",
            "Describe what ReproDeck should investigate.",
        ));
    }
    let request = context_compiler::ContextRequest::bounded(
        query,
        max_files.unwrap_or(12),
        max_chars.unwrap_or(36_000),
    );
    context_compiler::compile_context(Path::new(path), &request)
        .map_err(|error| BridgeError::new("context_compile_error", error.to_string()))
}

#[tauri::command]
async fn run_project_health(
    path: String,
    command_ids: Vec<String>,
    timeout_secs: Option<u64>,
    confirmed_execution: bool,
) -> Result<project_health::ProjectHealthReport, BridgeError> {
    if !confirmed_execution {
        return Err(BridgeError::new(
            "confirmation_required",
            "Running project checks requires explicit confirmation.",
        ));
    }
    let path = path.trim().to_owned();
    if path.is_empty() {
        return Err(BridgeError::new(
            "invalid_project_path",
            "Choose a local project directory first.",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let profile = project_intelligence::analyze_project(Path::new(&path))
            .map_err(|error| BridgeError::new("project_analysis_error", error.to_string()))?;
        let mut conn = open_conn()?;
        project_intelligence::save_profile(&conn, &profile)
            .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))?;
        let options = project_health::HealthRunOptions {
            command_ids,
            timeout_secs: timeout_secs.unwrap_or(180),
            confirmed_execution,
        };
        let report = project_health::run_project_health(&profile, &options)
            .map_err(|error| BridgeError::new("project_health_error", error.to_string()))?;
        project_health::save_report(&mut conn, &report)
            .map_err(|error| BridgeError::new("project_health_storage_error", error.to_string()))?;
        Ok(report)
    })
    .await
    .map_err(|error| BridgeError::new("project_health_task_error", error.to_string()))?
}

#[tauri::command]
fn latest_project_health(
    path: String,
) -> Result<Option<project_health::ProjectHealthReport>, BridgeError> {
    let conn = open_conn()?;
    project_health::latest_report(&conn, path.trim())
        .map_err(|error| BridgeError::new("project_health_storage_error", error.to_string()))
}

#[tauri::command]
fn list_project_problems(
    path: String,
) -> Result<Vec<project_health::ProjectProblemRecord>, BridgeError> {
    let conn = open_conn()?;
    project_health::list_project_problems(&conn, path.trim(), 200)
        .map_err(|error| BridgeError::new("project_problem_error", error.to_string()))
}

#[tauri::command]
fn build_bug_hunter_plan(path: String) -> Result<bug_hunter::BugHunterPlan, BridgeError> {
    let conn = open_conn()?;
    let root = path.trim();
    let profile = match project_intelligence::load_profile(&conn, root)
        .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))?
    {
        Some(profile) => profile,
        None => {
            let profile = project_intelligence::analyze_project(Path::new(root))
                .map_err(|error| BridgeError::new("project_analysis_error", error.to_string()))?;
            project_intelligence::save_profile(&conn, &profile)
                .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))?;
            profile
        }
    };
    Ok(bug_hunter::build_plan(&profile))
}

#[tauri::command]
fn analyze_bug_hunter_failures(
    path: String,
) -> Result<Option<bug_hunter::BugHunterAnalysis>, BridgeError> {
    let conn = open_conn()?;
    let Some(report) = project_health::latest_report(&conn, path.trim())
        .map_err(|error| BridgeError::new("project_health_storage_error", error.to_string()))?
    else {
        return Ok(None);
    };
    let problems = project_health::list_project_problems(&conn, path.trim(), 200)
        .map_err(|error| BridgeError::new("project_problem_error", error.to_string()))?;
    Ok(Some(bug_hunter::analyze_failures(&report, &problems)))
}

#[tauri::command]
fn create_investigation_case(
    path: String,
    cluster_id: String,
) -> Result<root_cause::InvestigationCase, BridgeError> {
    let root = path.trim();
    if root.is_empty() || cluster_id.trim().is_empty() {
        return Err(BridgeError::new(
            "invalid_investigation",
            "Project path and failure cluster are required.",
        ));
    }
    let conn = open_conn()?;
    let report = project_health::latest_report(&conn, root)
        .map_err(|error| BridgeError::new("project_health_storage_error", error.to_string()))?
        .ok_or_else(|| {
            BridgeError::new(
                "health_required",
                "Run Project Health before starting an investigation.",
            )
        })?;
    let profile = match project_intelligence::load_profile(&conn, root)
        .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))?
    {
        Some(profile) => profile,
        None => project_intelligence::analyze_project(Path::new(root))
            .map_err(|error| BridgeError::new("project_analysis_error", error.to_string()))?,
    };
    let problems = project_health::list_project_problems(&conn, root, 200)
        .map_err(|error| BridgeError::new("project_problem_error", error.to_string()))?;
    let analysis = bug_hunter::analyze_failures(&report, &problems);
    let cluster = analysis
        .clusters
        .iter()
        .find(|cluster| cluster.id == cluster_id)
        .ok_or_else(|| {
            BridgeError::new(
                "cluster_not_found",
                "The failure cluster is no longer part of the latest health run.",
            )
        })?;
    root_cause::create_case(&conn, &profile, &report, cluster)
        .map_err(|error| BridgeError::new("investigation_error", error.to_string()))
}

#[tauri::command]
fn list_investigation_cases(
    path: String,
) -> Result<Vec<root_cause::InvestigationCase>, BridgeError> {
    let conn = open_conn()?;
    root_cause::list_cases(&conn, path.trim(), 200)
        .map_err(|error| BridgeError::new("investigation_error", error.to_string()))
}

#[tauri::command]
fn compile_investigation_context(
    case_id: String,
) -> Result<context_compiler::ContextPacket, BridgeError> {
    let conn = open_conn()?;
    root_cause::compile_case_context(&conn, &case_id, 12, 36_000)
        .map_err(|error| BridgeError::new("investigation_context_error", error.to_string()))
}

#[tauri::command]
fn record_investigation_hypotheses(
    case_id: String,
    hypotheses: Vec<root_cause::HypothesisDraft>,
) -> Result<root_cause::InvestigationCase, BridgeError> {
    let conn = open_conn()?;
    root_cause::record_hypotheses(&conn, &case_id, hypotheses)
        .map_err(|error| BridgeError::new("investigation_hypothesis_error", error.to_string()))
}

#[tauri::command]
async fn generate_investigation_hypotheses(
    case_id: String,
    api_key: Option<String>,
    confirmed_network: bool,
) -> Result<root_cause::InvestigationCase, BridgeError> {
    if !confirmed_network {
        return Err(BridgeError::new(
            "confirmation_required",
            "AI hypothesis generation requires explicit network confirmation.",
        ));
    }
    let (app_settings, case, context) = {
        let conn = open_conn()?;
        let app_settings = settings::load(&conn)
            .map_err(|error| BridgeError::new("settings_error", error.to_string()))?;
        if !app_settings.ai.enabled || app_settings.ai.model.trim().is_empty() {
            return Err(BridgeError::new(
                "ai_not_configured",
                "Enable AI and configure a model in Settings first.",
            ));
        }
        let context = root_cause::compile_case_context(&conn, &case_id, 12, 36_000)
            .map_err(|error| BridgeError::new("investigation_context_error", error.to_string()))?;
        let case = root_cause::load_case(&conn, &case_id)
            .map_err(|error| BridgeError::new("investigation_error", error.to_string()))?;
        (app_settings, case, context)
    };

    let provider = ai::OpenAiCompatibleProvider::new(
        &app_settings.ai.base_url,
        &app_settings.ai.model,
        api_key,
        app_settings.ai.timeout_secs,
        app_settings.ai.max_tokens,
        app_settings.ai.temperature,
    )
    .map_err(|error| BridgeError::new("ai_config_error", error.to_string()))?;
    let observation = format!(
        "Failure cluster: {}\nSignature: {}\nSummary: {}\nCriterion: {} {:?} exit {:?}\nBaseline evidence: {}",
        case.cluster.title, case.cluster.signature, case.cluster.summary, case.criterion.label, case.criterion.baseline_status,
        case.criterion.baseline_exit_code, case.criterion.baseline_evidence_id
    );
    let candidates = provider
        .generate_root_cause_hypotheses(
            &observation,
            &case.evidence_ids,
            &context,
            &app_settings.language,
            confirmed_network,
        )
        .await
        .map_err(|error| BridgeError::new("ai_hypothesis_error", error.to_string()))?;
    let drafts = candidates
        .into_iter()
        .map(|candidate| {
            let explicitly_classified = candidate
                .supporting_evidence_ids
                .iter()
                .chain(&candidate.contradicting_evidence_ids)
                .chain(&candidate.neutral_evidence_ids)
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let mut neutral_evidence_ids = candidate.neutral_evidence_ids;
            neutral_evidence_ids.extend(
                case.evidence_ids
                    .iter()
                    .filter(|id| !explicitly_classified.contains(*id))
                    .cloned(),
            );
            neutral_evidence_ids.sort();
            neutral_evidence_ids.dedup();
            root_cause::HypothesisDraft {
                statement: candidate.statement,
                rationale: candidate.rationale,
                supporting_evidence_ids: candidate.supporting_evidence_ids,
                neutral_evidence_ids,
                contradicting_evidence_ids: candidate.contradicting_evidence_ids,
                falsifier: candidate.falsifier,
                next_experiment: candidate.next_experiment,
                confidence_percent: candidate.confidence_percent,
                source: root_cause::HypothesisSource::Model,
            }
        })
        .collect();
    let conn = open_conn()?;
    root_cause::record_hypotheses(&conn, &case_id, drafts)
        .map_err(|error| BridgeError::new("investigation_hypothesis_error", error.to_string()))
}

#[tauri::command]
fn get_fix_workspace(
    case_id: String,
) -> Result<Option<root_cause::FixWorkspaceRecord>, BridgeError> {
    let conn = open_conn()?;
    root_cause::get_fix_workspace(&conn, &case_id)
        .map_err(|error| BridgeError::new("investigation_workspace_error", error.to_string()))
}

#[tauri::command]
fn create_fix_workspace(case_id: String) -> Result<root_cause::FixWorkspaceRecord, BridgeError> {
    let conn = open_conn()?;
    root_cause::create_fix_workspace(&conn, &case_id)
        .map_err(|error| BridgeError::new("investigation_workspace_error", error.to_string()))
}

#[tauri::command]
fn checkpoint_fix_workspace(
    case_id: String,
) -> Result<root_cause::FixWorkspaceRecord, BridgeError> {
    let conn = open_conn()?;
    root_cause::checkpoint_fix_workspace(&conn, &case_id)
        .map_err(|error| BridgeError::new("investigation_workspace_error", error.to_string()))
}

#[tauri::command]
fn fix_workspace_diff(case_id: String) -> Result<root_cause::FixWorkspaceDiff, BridgeError> {
    let conn = open_conn()?;
    root_cause::fix_workspace_diff(&conn, &case_id)
        .map_err(|error| BridgeError::new("investigation_workspace_error", error.to_string()))
}

#[tauri::command]
fn stage_investigation_verification(
    session_id: String,
    case_id: String,
    hypothesis_id: String,
    experiment_id: String,
    regressions: Vec<verification::RegressionDraft>,
) -> Result<verification::VerificationHandoff, BridgeError> {
    let conn = open_conn()?;
    let candidate =
        root_cause::verification_handoff_candidate(&conn, &case_id, &hypothesis_id, &experiment_id)
            .map_err(|error| BridgeError::new("investigation_handoff_error", error.to_string()))?;
    verification::stage_handoff(&conn, &session_id, candidate, &regressions)
        .map_err(map_verification)
}

#[tauri::command]
fn discard_fix_workspace(case_id: String, confirmed: bool) -> Result<(), BridgeError> {
    if !confirmed {
        return Err(BridgeError::new(
            "confirmation_required",
            "Discarding a fix workspace requires explicit confirmation.",
        ));
    }
    let conn = open_conn()?;
    root_cause::discard_fix_workspace(&conn, &case_id)
        .map_err(|error| BridgeError::new("investigation_workspace_error", error.to_string()))
}

#[tauri::command]
async fn run_causal_experiment(
    case_id: String,
    hypothesis_id: String,
    timeout_secs: Option<u64>,
    confirmed_execution: bool,
) -> Result<root_cause::InvestigationCase, BridgeError> {
    if !confirmed_execution {
        return Err(BridgeError::new(
            "confirmation_required",
            "Running a causal experiment requires explicit confirmation.",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_conn()?;
        root_cause::run_causal_experiment(
            &conn,
            &case_id,
            &hypothesis_id,
            timeout_secs,
            confirmed_execution,
        )
        .map_err(|error| BridgeError::new("causal_experiment_error", error.to_string()))
    })
    .await
    .map_err(|error| BridgeError::new("causal_experiment_task_error", error.to_string()))?
}

#[tauri::command]
fn load_settings() -> Result<settings::AppSettings, BridgeError> {
    let conn = open_conn()?;
    settings::load(&conn).map_err(|error| BridgeError::new("settings_error", error.to_string()))
}

#[tauri::command]
fn save_settings(value: settings::AppSettings) -> Result<settings::AppSettings, BridgeError> {
    let conn = open_conn()?;
    settings::save(&conn, &value)
        .map_err(|error| BridgeError::new("settings_error", error.to_string()))
}

#[tauri::command]
fn storage_location() -> String {
    app_data_dir().to_string_lossy().into_owned()
}

#[tauri::command]
fn list_pending_recovery() -> Result<Vec<recovery::RecoveryEntry>, BridgeError> {
    recovery::list_pending_cleanup()
        .map_err(|error| BridgeError::new("recovery_error", error.to_string()))
}

#[tauri::command]
async fn retry_pending_recovery(id: String, confirmed: bool) -> Result<(), BridgeError> {
    if !confirmed {
        return Err(BridgeError::new(
            "confirmation_required",
            "Recovery cleanup requires explicit confirmation.",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        recovery::retry_cleanup(&id)
            .map_err(|error| BridgeError::new("recovery_error", error.to_string()))
    })
    .await
    .map_err(|error| BridgeError::new("recovery_task_error", error.to_string()))?
}

#[tauri::command]
fn preview_session_capsule(
    session_id: String,
) -> Result<capsule::CapsuleExportPreview, BridgeError> {
    let conn = open_conn()?;
    capsule::preview_session_export(&conn, &artifact_dir(), &session_id)
        .map_err(|error| BridgeError::new("capsule_preview_error", error.to_string()))
}

#[tauri::command]
fn export_session_capsule(
    session_id: String,
    destination: String,
) -> Result<capsule::CapsuleSummary, BridgeError> {
    let conn = open_conn()?;
    capsule::export_session(
        &conn,
        &artifact_dir(),
        &session_id,
        Path::new(destination.trim()),
    )
    .map_err(|error| BridgeError::new("capsule_export_error", error.to_string()))
}

#[tauri::command]
fn inspect_capsule(path: String) -> Result<capsule::CapsuleSummary, BridgeError> {
    capsule::inspect_capsule(Path::new(path.trim()))
        .map_err(|error| BridgeError::new("capsule_invalid", error.to_string()))
}

#[tauri::command]
fn import_capsule(path: String) -> Result<capsule::ImportedCapsule, BridgeError> {
    let conn = open_conn()?;
    capsule::import_capsule(&conn, Path::new(path.trim()), &capsule_dir())
        .map_err(|error| BridgeError::new("capsule_import_error", error.to_string()))
}

#[tauri::command]
fn list_imported_capsules() -> Result<Vec<capsule::ImportedCapsule>, BridgeError> {
    let conn = open_conn()?;
    capsule::list_imported_capsules(&conn)
        .map_err(|error| BridgeError::new("capsule_error", error.to_string()))
}

#[tauri::command]
fn github_status() -> github::GitHubStatus {
    github::status()
}

#[tauri::command]
fn github_create_issue(
    session_id: String,
    title: String,
    body: String,
    confirmed: bool,
) -> Result<github::GitHubCreatedItem, BridgeError> {
    let conn = open_conn()?;
    let repo = repository::get_session_repository(&conn, &session_id)
        .map_err(map_repo)?
        .ok_or_else(|| {
            BridgeError::new(
                "repository_required",
                "Attach a repository before creating a GitHub issue.",
            )
        })?;
    github::create_issue(Path::new(&repo.path), &title, &body, confirmed)
        .map_err(|error| BridgeError::new("github_error", error.to_string()))
}

#[tauri::command]
fn github_create_draft_pr(
    session_id: String,
    title: String,
    body: String,
    confirmed: bool,
) -> Result<github::GitHubCreatedItem, BridgeError> {
    let conn = open_conn()?;
    let session = timeline::get_session_record(&conn, &session_id)
        .map_err(|error| BridgeError::storage(error.to_string()))?
        .ok_or_else(|| BridgeError::new("not_found", "Session not found."))?;
    if session.state != "Applied" {
        return Err(BridgeError::new(
            "apply_required",
            "Apply the verified patch before creating a draft pull request.",
        ));
    }
    let repo = repository::get_session_repository(&conn, &session_id)
        .map_err(map_repo)?
        .ok_or_else(|| {
            BridgeError::new(
                "repository_required",
                "Attach a repository before creating a draft pull request.",
            )
        })?;
    github::create_draft_pr(Path::new(&repo.path), &title, &body, confirmed)
        .map_err(|error| BridgeError::new("github_error", error.to_string()))
}

#[tauri::command]
async fn ai_test_connection(
    base_url: String,
    model: String,
    api_key: Option<String>,
    timeout_secs: u64,
    max_tokens: u32,
    temperature: f32,
    confirmed_network: bool,
) -> Result<ai::AiConnectionStatus, BridgeError> {
    let provider = ai::OpenAiCompatibleProvider::new(
        &base_url,
        &model,
        api_key,
        timeout_secs,
        max_tokens,
        temperature,
    )
    .map_err(|error| BridgeError::new("ai_error", error.to_string()))?;
    provider
        .test_connection(confirmed_network)
        .await
        .map_err(|error| BridgeError::new("ai_error", error.to_string()))
}

#[tauri::command]
async fn ai_investigate_project(
    path: String,
    question: String,
    api_key: Option<String>,
    confirmed_network: bool,
) -> Result<ai::ProjectInvestigation, BridgeError> {
    if question.trim().is_empty() {
        return Err(BridgeError::new(
            "invalid_context_query",
            "Describe what ReproDeck should investigate.",
        ));
    }
    let conn = open_conn()?;
    let app_settings = settings::load(&conn)
        .map_err(|error| BridgeError::new("settings_error", error.to_string()))?;
    if !app_settings.ai.enabled {
        return Err(BridgeError::new(
            "ai_disabled",
            "Enable the optional AI provider in Settings first.",
        ));
    }
    let profile = project_intelligence::analyze_project(Path::new(path.trim()))
        .map_err(|error| BridgeError::new("project_analysis_error", error.to_string()))?;
    project_intelligence::save_profile(&conn, &profile)
        .map_err(|error| BridgeError::new("project_profile_error", error.to_string()))?;
    let request = context_compiler::ContextRequest::bounded(question.clone(), 12, 36_000);
    let context = context_compiler::compile_context(Path::new(&profile.root_path), &request)
        .map_err(|error| BridgeError::new("context_compile_error", error.to_string()))?;
    let provider = ai::OpenAiCompatibleProvider::new(
        &app_settings.ai.base_url,
        &app_settings.ai.model,
        api_key,
        app_settings.ai.timeout_secs,
        app_settings.ai.max_tokens,
        app_settings.ai.temperature,
    )
    .map_err(|error| BridgeError::new("ai_error", error.to_string()))?;
    let health_report = project_health::latest_report(&conn, &profile.root_path)
        .map_err(|error| BridgeError::new("project_health_storage_error", error.to_string()))?;
    let analysis = provider
        .investigate_project(
            &profile,
            &question,
            &context,
            health_report.as_ref(),
            confirmed_network,
        )
        .await
        .map_err(|error| BridgeError::new("ai_error", error.to_string()))?;
    Ok(ai::ProjectInvestigation { analysis, context })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt().with_target(false).try_init();
    tracing::info!("starting ReproDeck desktop runtime");
    if let Ok(mut conn) = open_conn() {
        if let Err(error) = timeline::recover_running(&mut conn) {
            tracing::warn!(error = %error, "failed to recover interrupted executions");
        }
    }
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            runtime_health,
            list_sessions,
            create_bug_session,
            inspect_repository,
            attach_repository,
            get_session_repository,
            capture_environment,
            latest_environment,
            add_reproduction_step,
            list_reproduction_steps,
            list_reproduction_runs,
            execute_reproduction_step,
            reproduction_outcome,
            session_verification_outcome,
            session_verification_status,
            promote_regression_check,
            run_regression_check,
            get_shadow_workspace,
            create_shadow_workspace,
            finalize_shadow_workspace,
            shadow_diff,
            apply_shadow_workspace,
            discard_shadow_workspace,
            list_timeline_entries,
            read_artifact_text,
            list_evidence_items,
            reset_reproduction_baseline,
            list_repositories,
            analyze_project,
            create_demo_project,
            list_project_profiles,
            compile_project_context,
            run_project_health,
            latest_project_health,
            list_project_problems,
            build_bug_hunter_plan,
            analyze_bug_hunter_failures,
            create_investigation_case,
            list_investigation_cases,
            compile_investigation_context,
            record_investigation_hypotheses,
            generate_investigation_hypotheses,
            get_fix_workspace,
            create_fix_workspace,
            checkpoint_fix_workspace,
            fix_workspace_diff,
            stage_investigation_verification,
            discard_fix_workspace,
            run_causal_experiment,
            load_settings,
            save_settings,
            storage_location,
            list_pending_recovery,
            retry_pending_recovery,
            preview_session_capsule,
            export_session_capsule,
            inspect_capsule,
            import_capsule,
            list_imported_capsules,
            github_status,
            github_create_issue,
            github_create_draft_pr,
            ai_test_connection,
            ai_investigate_project,
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("ReproDeck runtime error: {error}");
    }
}
