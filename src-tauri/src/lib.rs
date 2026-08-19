use reprodeck_core::{db, timeline, verification};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::path::PathBuf;

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
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub state: String,
    pub meta: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionDto {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptDto {
    pub id: String,
    pub execution_id: String,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub created_at: i64,
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
pub struct VerdictDto {
    pub verdict: String,
}

impl From<timeline::SessionRecord> for SessionDto {
    fn from(value: timeline::SessionRecord) -> Self {
        Self {
            id: value.id,
            created_at: value.created_at,
            updated_at: value.updated_at,
            state: value.state,
            meta: value.meta,
        }
    }
}

impl From<timeline::ActionRecord> for ActionDto {
    fn from(value: timeline::ActionRecord) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            state: value.state,
            created_at: value.created_at,
        }
    }
}

impl From<timeline::ReceiptRecord> for ReceiptDto {
    fn from(value: timeline::ReceiptRecord) -> Self {
        Self {
            id: value.id,
            execution_id: value.execution_id,
            stdout_preview: value.stdout_preview,
            stderr_preview: value.stderr_preview,
            stdout_truncated: value.stdout_truncated,
            stderr_truncated: value.stderr_truncated,
            created_at: value.created_at,
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

pub fn list_actions_service(session_id: &str) -> Result<Vec<ActionDto>, BridgeError> {
    let conn = open_conn()?;
    timeline::list_actions(&conn, session_id, None, 500)
        .map(|values| values.into_iter().map(ActionDto::from).collect())
        .map_err(|_| BridgeError::database("Unable to load the session timeline."))
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
    let sql = match session_id {
        Some(_) => {
            "SELECT id, session_id, title, description, state, version, created_at FROM outcome_contracts WHERE session_id = ?1 ORDER BY created_at DESC, id DESC"
        }
        None => {
            "SELECT id, session_id, title, description, state, version, created_at FROM outcome_contracts ORDER BY created_at DESC, id DESC"
        }
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|_| BridgeError::database("Unable to prepare the outcome query."))?;

    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ContractDto> {
        Ok(ContractDto {
            id: row.get(0)?,
            session_id: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            state: row.get(4)?,
            version: row.get(5)?,
            created_at: row.get(6)?,
        })
    };

    let contracts = if let Some(session_id) = session_id {
        stmt.query_map(rusqlite::params![session_id], map_row)
            .map_err(|_| BridgeError::database("Unable to query outcome contracts."))?
            .collect::<Result<Vec<_>, _>>()
    } else {
        stmt.query_map([], map_row)
            .map_err(|_| BridgeError::database("Unable to query outcome contracts."))?
            .collect::<Result<Vec<_>, _>>()
    };

    contracts.map_err(|_| BridgeError::database("Unable to decode outcome contracts."))
}

pub fn list_verification_runs_service(
    contract_id: &str,
) -> Result<Vec<VerificationRunDto>, BridgeError> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, check_id, phase, status, started_at, finished_at, receipt_id FROM verification_runs WHERE contract_id = ?1 ORDER BY started_at DESC, id DESC",
        )
        .map_err(|_| BridgeError::database("Unable to prepare the verification query."))?;

    let rows = stmt
        .query_map(rusqlite::params![contract_id], |row| {
            Ok(VerificationRunDto {
                id: row.get(0)?,
                check_id: row.get(1)?,
                phase: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                receipt_id: row.get(6)?,
            })
        })
        .map_err(|_| BridgeError::database("Unable to query verification runs."))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| BridgeError::database("Unable to decode verification runs."))
}

pub fn evaluate_contract_service(contract_id: &str) -> Result<VerdictDto, BridgeError> {
    let conn = open_conn()?;
    let verdict = verification::evaluate_outcome(&conn, contract_id).map_err(|_| {
        BridgeError::new(
            "evaluation_failed",
            "Unable to evaluate this outcome contract.",
        )
    })?;
    Ok(VerdictDto { verdict })
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
fn list_actions(session_id: String) -> Result<Vec<ActionDto>, BridgeError> {
    list_actions_service(&session_id)
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
            list_actions,
            get_receipt,
            list_contracts,
            list_verification_runs,
            evaluate_contract,
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
    fn bridge_error_does_not_require_internal_error_text() {
        let error = BridgeError::database("Unable to load timeline.");
        assert_eq!(error.code, "database_error");
        assert_eq!(error.message, "Unable to load timeline.");
    }
}
