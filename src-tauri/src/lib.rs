mod shadow_bridge;

use reprodeck_core::{db, evidence, repository, timeline, verification};
use serde::{Deserialize, Serialize};
use shadow_bridge::{
    apply_shadow_workspace, create_shadow_workspace, discard_shadow_workspace,
    finalize_shadow_workspace, get_shadow_workspace, refresh_shadow_workspace,
};
use std::fmt::{self, Display};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}

impl BridgeError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn database(context: &str) -> Self {
        Self::new("database_error", context)
    }
}

impl Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BridgeError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDto {
    pub id: String,
    pub repo_id: Option<String>,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub state: String,
    pub meta: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryDto {
    pub id: Option<String>,
    pub path: String,
    pub head_commit: String,
    pub branch: String,
    pub is_dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionDto {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub meta: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionDto {
    pub id: String,
    pub action_id: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptDto {
    pub id: String,
    pub execution_id: String,
    pub summary: Option<String>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDto {
    pub id: String,
    pub receipt_id: String,
    pub checksum: String,
    pub size: i64,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelineEntryDto {
    pub action: ActionDto,
    pub execution: Option<ExecutionDto>,
    pub receipt: Option<ReceiptDto>,
    pub artifacts: Vec<ArtifactDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractDto {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub version: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationRunDto {
    pub id: String,
    pub check_id: Option<String>,
    pub phase: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub receipt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeCheckSummaryDto {
    pub check_id: String,
    pub stable_id: String,
    pub description: String,
    pub required: bool,
    pub before: Option<String>,
    pub after: Option<String>,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeSummaryDto {
    pub contract_id: String,
    pub overall: String,
    pub checks: Vec<OutcomeCheckSummaryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerdictDto {
    pub verdict: String,
}

impl From<timeline::SessionRecord> for SessionDto {
    fn from(value: timeline::SessionRecord) -> Self {
        Self {
            id: value.id,
            repo_id: value.repo_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
            state: value.state,
            meta: value.meta,
        }
    }
}

impl From<repository::RepositoryInfo> for RepositoryDto {
    fn from(value: repository::RepositoryInfo) -> Self {
        Self {
            id: value.id,
            path: value.path,
            head_commit: value.head_commit,
            branch: value.branch,
            is_dirty: value.is_dirty,
        }
    }
}

impl From<timeline::ActionRecord> for ActionDto {
    fn from(value: timeline::ActionRecord) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            state: value.state,
            meta: value.meta,
            created_at: value.created_at,
        }
    }
}

impl From<timeline::ExecutionRecord> for ExecutionDto {
    fn from(value: timeline::ExecutionRecord) -> Self {
        Self {
            id: value.id,
            action_id: value.action_id,
            status: value.status,
            started_at: value.started_at,
            finished_at: value.finished_at,
            duration_ms: value.duration_ms,
        }
    }
}

impl From<timeline::ReceiptRecord> for ReceiptDto {
    fn from(value: timeline::ReceiptRecord) -> Self {
        Self {
            id: value.id,
            execution_id: value.execution_id,
            summary: value.summary,
            stdout_preview: value.stdout_preview,
            stderr_preview: value.stderr_preview,
            stdout_truncated: value.stdout_truncated,
            stderr_truncated: value.stderr_truncated,
            created_at: value.created_at,
        }
    }
}

impl From<evidence::ArtifactRecord> for ArtifactDto {
    fn from(value: evidence::ArtifactRecord) -> Self {
        Self {
            id: value.id,
            receipt_id: value.receipt_id,
            checksum: value.checksum,
            size: value.size,
            media_type: value.media_type,
        }
    }
}

impl From<verification::OutcomeContract> for ContractDto {
    fn from(value: verification::OutcomeContract) -> Self {
        Self {
            id: value.id,
            session_id: value.session_id,
            title: value.title,
            description: value.description,
            state: value.state,
            version: value.version,
            created_at: value.created_at,
        }
    }
}

impl From<verification::VerificationRun> for VerificationRunDto {
    fn from(value: verification::VerificationRun) -> Self {
        Self {
            id: value.id,
            check_id: value.check_id,
            phase: value.phase.to_string(),
            status: value.status.to_string(),
            started_at: value.started_at,
            finished_at: value.finished_at,
            receipt_id: value.receipt_id,
        }
    }
}

impl From<verification::OutcomeSummary> for OutcomeSummaryDto {
    fn from(value: verification::OutcomeSummary) -> Self {
        Self {
            contract_id: value.contract_id,
            overall: value.overall.to_string(),
            checks: value
                .checks
                .into_iter()
                .map(|item| OutcomeCheckSummaryDto {
                    check_id: item.check.id,
                    stable_id: item.check.stable_id,
                    description: item.check.description,
                    required: item.check.required,
                    before: item.before.map(|status| status.to_string()),
                    after: item.after.map(|status| status.to_string()),
                    outcome: item.outcome.to_string(),
                })
                .collect(),
        }
    }
}

fn app_db_path() -> PathBuf {
    if let Some(base) = directories::BaseDirs::new() {
        let mut path = base.data_local_dir().to_path_buf();
        path.push("reprodeck");
        let _ = std::fs::create_dir_all(&path);
        path.push("reprodeck.db");
        path
    } else {
        std::env::temp_dir().join("reprodeck.db")
    }
}

fn open_conn() -> Result<rusqlite::Connection, BridgeError> {
    db::init_db(&app_db_path())
        .map_err(|_| BridgeError::database("Unable to initialize ReproDeck storage."))
}

fn repository_error(error: repository::RepositoryError) -> BridgeError {
    match error {
        repository::RepositoryError::SessionNotFound(_) => {
            BridgeError::new("not_found", "Session not found.")
        }
        repository::RepositoryError::UnbornRepository => BridgeError::new(
            "repository_unborn",
            "The Git repository needs at least one commit before ReproDeck can attach it.",
        ),
        repository::RepositoryError::NonUtf8Path => BridgeError::new(
            "repository_path_unsupported",
            "This repository path cannot be represented safely by the desktop bridge.",
        ),
        _ => BridgeError::new(
            "repository_invalid",
            "The selected path is not an accessible Git working repository.",
        ),
    }
}

pub fn list_sessions_service() -> Result<Vec<SessionDto>, BridgeError> {
    let conn = open_conn()?;
    timeline::list_sessions(&conn, None, 200)
        .map(|values| values.into_iter().map(SessionDto::from).collect())
        .map_err(|_| BridgeError::database("Unable to list sessions."))
}

pub fn create_session_service(id: &str) -> Result<SessionDto, BridgeError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "Session id must not be empty.",
        ));
    }

    let conn = open_conn()?;
    timeline::create_session(&conn, id, "Active", None)
        .map_err(|_| BridgeError::database("Unable to create the session."))?;
    timeline::get_session_record(&conn, id)
        .map_err(|_| BridgeError::database("Unable to read the created session."))?
        .map(SessionDto::from)
        .ok_or_else(|| BridgeError::database("Created session could not be loaded."))
}

pub fn inspect_repository_service(path: &str) -> Result<RepositoryDto, BridgeError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "Repository path must not be empty.",
        ));
    }
    repository::inspect_repository(Path::new(path))
        .map(RepositoryDto::from)
        .map_err(repository_error)
}

pub fn attach_repository_service(
    session_id: &str,
    path: &str,
) -> Result<RepositoryDto, BridgeError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "Repository path must not be empty.",
        ));
    }
    let mut conn = open_conn()?;
    repository::attach_repository_to_session(&mut conn, session_id, Path::new(path))
        .map(RepositoryDto::from)
        .map_err(repository_error)
}

pub fn get_session_repository_service(
    session_id: &str,
) -> Result<Option<RepositoryDto>, BridgeError> {
    let conn = open_conn()?;
    repository::get_session_repository(&conn, session_id)
        .map(|value| value.map(RepositoryDto::from))
        .map_err(repository_error)
}

pub fn list_actions_service(session_id: &str) -> Result<Vec<ActionDto>, BridgeError> {
    let conn = open_conn()?;
    timeline::list_actions(&conn, session_id, None, 500)
        .map(|values| values.into_iter().map(ActionDto::from).collect())
        .map_err(|_| BridgeError::database("Unable to load the session timeline."))
}

pub fn list_timeline_entries_service(
    session_id: &str,
) -> Result<Vec<TimelineEntryDto>, BridgeError> {
    let conn = open_conn()?;
    let actions = timeline::list_actions(&conn, session_id, None, 500)
        .map_err(|_| BridgeError::database("Unable to load the session timeline."))?;
    let mut entries = Vec::with_capacity(actions.len());

    for action in actions {
        let action_id = action.id.clone();
        let execution = timeline::list_executions(&conn, &action_id)
            .map_err(|_| BridgeError::database("Unable to load action executions."))?
            .into_iter()
            .last();

        let (receipt, artifacts) = if let Some(execution) = execution.as_ref() {
            let receipt = timeline::list_receipts(&conn, &execution.id)
                .map_err(|_| BridgeError::database("Unable to load execution receipts."))?
                .into_iter()
                .last();
            let artifacts = if let Some(receipt) = receipt.as_ref() {
                evidence::list_artifacts_for_receipt(&conn, &receipt.id)
                    .map_err(|_| BridgeError::database("Unable to load receipt evidence."))?
                    .into_iter()
                    .map(ArtifactDto::from)
                    .collect()
            } else {
                Vec::new()
            };
            (receipt.map(ReceiptDto::from), artifacts)
        } else {
            (None, Vec::new())
        };

        entries.push(TimelineEntryDto {
            action: ActionDto::from(action),
            execution: execution.map(ExecutionDto::from),
            receipt,
            artifacts,
        });
    }

    Ok(entries)
}

pub fn get_receipt_service(receipt_id: &str) -> Result<ReceiptDto, BridgeError> {
    let conn = open_conn()?;
    timeline::get_receipt(&conn, receipt_id)
        .map_err(|_| BridgeError::database("Unable to load the receipt."))?
        .map(ReceiptDto::from)
        .ok_or_else(|| BridgeError::new("not_found", "Receipt not found."))
}

pub fn list_contracts_service(session_id: Option<&str>) -> Result<Vec<ContractDto>, BridgeError> {
    let conn = open_conn()?;
    verification::list_outcome_contracts(&conn, session_id)
        .map(|values| values.into_iter().map(ContractDto::from).collect())
        .map_err(|_| BridgeError::database("Unable to list outcome contracts."))
}

pub fn list_verification_runs_service(
    contract_id: &str,
) -> Result<Vec<VerificationRunDto>, BridgeError> {
    let conn = open_conn()?;
    verification::list_verification_runs(&conn, contract_id)
        .map(|values| values.into_iter().map(VerificationRunDto::from).collect())
        .map_err(|_| BridgeError::database("Unable to list verification runs."))
}

pub fn get_outcome_summary_service(contract_id: &str) -> Result<OutcomeSummaryDto, BridgeError> {
    let conn = open_conn()?;
    verification::get_outcome_summary(&conn, contract_id)
        .map(OutcomeSummaryDto::from)
        .map_err(|_| {
            BridgeError::new(
                "evaluation_failed",
                "Unable to evaluate this outcome contract.",
            )
        })
}

pub fn evaluate_contract_service(contract_id: &str) -> Result<VerdictDto, BridgeError> {
    get_outcome_summary_service(contract_id).map(|summary| VerdictDto {
        verdict: summary.overall,
    })
}

#[tauri::command]
fn list_sessions() -> Result<Vec<SessionDto>, BridgeError> {
    list_sessions_service()
}

#[tauri::command]
fn create_session(id: String) -> Result<SessionDto, BridgeError> {
    create_session_service(&id)
}

#[tauri::command]
fn inspect_repository(path: String) -> Result<RepositoryDto, BridgeError> {
    inspect_repository_service(&path)
}

#[tauri::command]
fn attach_repository(session_id: String, path: String) -> Result<RepositoryDto, BridgeError> {
    attach_repository_service(&session_id, &path)
}

#[tauri::command]
fn get_session_repository(session_id: String) -> Result<Option<RepositoryDto>, BridgeError> {
    get_session_repository_service(&session_id)
}

#[tauri::command]
fn list_actions(session_id: String) -> Result<Vec<ActionDto>, BridgeError> {
    list_actions_service(&session_id)
}

#[tauri::command]
fn list_timeline_entries(session_id: String) -> Result<Vec<TimelineEntryDto>, BridgeError> {
    list_timeline_entries_service(&session_id)
}

#[tauri::command]
fn get_receipt(receipt_id: String) -> Result<ReceiptDto, BridgeError> {
    get_receipt_service(&receipt_id)
}

#[tauri::command]
fn list_contracts(session_id: Option<String>) -> Result<Vec<ContractDto>, BridgeError> {
    list_contracts_service(session_id.as_deref())
}

#[tauri::command]
fn list_verification_runs(contract_id: String) -> Result<Vec<VerificationRunDto>, BridgeError> {
    list_verification_runs_service(&contract_id)
}

#[tauri::command]
fn get_outcome_summary(contract_id: String) -> Result<OutcomeSummaryDto, BridgeError> {
    get_outcome_summary_service(&contract_id)
}

#[tauri::command]
fn evaluate_contract(contract_id: String) -> Result<VerdictDto, BridgeError> {
    evaluate_contract_service(&contract_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            create_session,
            inspect_repository,
            attach_repository,
            get_session_repository,
            list_actions,
            list_timeline_entries,
            get_receipt,
            list_contracts,
            list_verification_runs,
            get_outcome_summary,
            evaluate_contract,
            get_shadow_workspace,
            create_shadow_workspace,
            refresh_shadow_workspace,
            finalize_shadow_workspace,
            apply_shadow_workspace,
            discard_shadow_workspace,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_error_serializes_stably() {
        let error = BridgeError::new("not_found", "missing");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "not_found");
        assert_eq!(json["message"], "missing");
    }

    #[test]
    fn verdict_dto_serializes_as_object() {
        let value = serde_json::to_value(VerdictDto {
            verdict: "VerifiedFix".to_string(),
        })
        .unwrap();
        assert_eq!(value["verdict"], "VerifiedFix");
    }

    #[test]
    fn repository_dto_preserves_runtime_status() {
        let dto = RepositoryDto::from(repository::RepositoryInfo {
            id: Some("repo".to_string()),
            path: "C:/work/repo".to_string(),
            head_commit: "abcdef".to_string(),
            branch: "main".to_string(),
            is_dirty: true,
        });
        assert_eq!(dto.id.as_deref(), Some("repo"));
        assert_eq!(dto.branch, "main");
        assert!(dto.is_dirty);
    }

    #[test]
    fn action_dto_keeps_sanitized_metadata_surface() {
        let dto = ActionDto::from(timeline::ActionRecord {
            created_seq: 1,
            id: "action".to_string(),
            session_id: "session".to_string(),
            parent_id: None,
            kind: "verification".to_string(),
            meta: Some("{\"phase\":\"Before\"}".to_string()),
            state: "Completed".to_string(),
            created_at: 10,
        });
        assert_eq!(dto.meta.as_deref(), Some("{\"phase\":\"Before\"}"));
    }

    #[test]
    fn outcome_summary_dto_keeps_business_logic_result() {
        let summary = verification::OutcomeSummary {
            contract_id: "contract".to_string(),
            overall: verification::OutcomeState::VerifiedFix,
            checks: vec![verification::VerificationCheckSummary {
                check: verification::VerificationCheck {
                    id: "check".to_string(),
                    contract_id: "contract".to_string(),
                    stable_id: "regression".to_string(),
                    description: "Regression".to_string(),
                    command_ref: None,
                    expected_condition: None,
                    required: true,
                    ordering: 0,
                },
                before: Some(verification::RunStatus::Failed),
                after: Some(verification::RunStatus::Passed),
                outcome: verification::OutcomeState::VerifiedFix,
            }],
        };
        let dto = OutcomeSummaryDto::from(summary);
        assert_eq!(dto.overall, "VerifiedFix");
        assert_eq!(dto.checks[0].before.as_deref(), Some("Failed"));
        assert_eq!(dto.checks[0].after.as_deref(), Some("Passed"));
    }

    #[test]
    fn bridge_error_does_not_require_internal_error_text() {
        let error = BridgeError::database("Unable to load timeline.");
        assert_eq!(error.code, "database_error");
        assert_eq!(error.message, "Unable to load timeline.");
    }
}
