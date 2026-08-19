use reprodeck_core::{db, timeline, verification};
use rusqlite::OptionalExtension;
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

    fn database(context: &str, error: impl Display) -> Self {
        Self::new("database_error", format!("{context}: {error}"))
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
    pub updated_at: i64,
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
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub receipt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerdictDto {
    pub verdict: String,
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
    db::init_db(&app_db_path()).map_err(|error| BridgeError::database("initialize database", error))
}

pub fn list_sessions_service() -> Result<Vec<SessionDto>, BridgeError> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, updated_at, state, meta \
             FROM sessions ORDER BY created_at DESC, id DESC",
        )
        .map_err(|error| BridgeError::database("prepare session query", error))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SessionDto {
                id: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                state: row.get(3)?,
                meta: row.get(4)?,
            })
        })
        .map_err(|error| BridgeError::database("query sessions", error))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| BridgeError::database("decode session row", error))
}

pub fn create_session_service(id: &str) -> Result<SessionDto, BridgeError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(BridgeError::new(
            "invalid_request",
            "session id must not be empty",
        ));
    }

    let conn = open_conn()?;
    timeline::create_session(&conn, id, "Active", None)
        .map_err(|error| BridgeError::database("create session", error))?;

    conn.query_row(
        "SELECT id, created_at, updated_at, state, meta FROM sessions WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(SessionDto {
                id: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                state: row.get(3)?,
                meta: row.get(4)?,
            })
        },
    )
    .map_err(|error| BridgeError::database("read created session", error))
}

pub fn list_actions_service(session_id: &str) -> Result<Vec<ActionDto>, BridgeError> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, state, created_at FROM actions \
             WHERE session_id = ?1 ORDER BY created_seq DESC",
        )
        .map_err(|error| BridgeError::database("prepare action query", error))?;

    let rows = stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(ActionDto {
                id: row.get(0)?,
                kind: row.get(1)?,
                state: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|error| BridgeError::database("query actions", error))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| BridgeError::database("decode action row", error))
}

pub fn get_receipt_service(receipt_id: &str) -> Result<ReceiptDto, BridgeError> {
    let conn = open_conn()?;
    conn.query_row(
        "SELECT id, execution_id, stdout_preview, stderr_preview, \
         stdout_truncated, stderr_truncated, created_at \
         FROM receipts WHERE id = ?1",
        rusqlite::params![receipt_id],
        |row| {
            Ok(ReceiptDto {
                id: row.get(0)?,
                execution_id: row.get(1)?,
                stdout_preview: row.get(2)?,
                stderr_preview: row.get(3)?,
                stdout_truncated: row.get::<_, i64>(4)? != 0,
                stderr_truncated: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|error| BridgeError::database("query receipt", error))?
    .ok_or_else(|| BridgeError::new("not_found", "receipt not found"))
}

pub fn list_contracts_service(session_id: Option<&str>) -> Result<Vec<ContractDto>, BridgeError> {
    let conn = open_conn()?;
    let sql = match session_id {
        Some(_) => {
            "SELECT id, session_id, title, description, state, version, created_at \
             FROM outcome_contracts WHERE session_id = ?1 ORDER BY created_at DESC, id DESC"
        }
        None => {
            "SELECT id, session_id, title, description, state, version, created_at \
             FROM outcome_contracts ORDER BY created_at DESC, id DESC"
        }
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|error| BridgeError::database("prepare contract query", error))?;

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
            .map_err(|error| BridgeError::database("query contracts", error))?
            .collect::<Result<Vec<_>, _>>()
    } else {
        stmt.query_map([], map_row)
            .map_err(|error| BridgeError::database("query contracts", error))?
            .collect::<Result<Vec<_>, _>>()
    };

    contracts.map_err(|error| BridgeError::database("decode contract row", error))
}

pub fn list_verification_runs_service(
    contract_id: &str,
) -> Result<Vec<VerificationRunDto>, BridgeError> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, check_id, phase, status, started_at, finished_at, receipt_id \
             FROM verification_runs WHERE contract_id = ?1 \
             ORDER BY started_at DESC, id DESC",
        )
        .map_err(|error| BridgeError::database("prepare verification query", error))?;

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
        .map_err(|error| BridgeError::database("query verification runs", error))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| BridgeError::database("decode verification row", error))
}

pub fn evaluate_contract_service(contract_id: &str) -> Result<VerdictDto, BridgeError> {
    let conn = open_conn()?;
    let verdict = verification::evaluate_outcome(&conn, contract_id)
        .map_err(|error| BridgeError::new("evaluation_failed", error.to_string()))?;
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
fn list_verification_runs(
    contract_id: String,
) -> Result<Vec<VerificationRunDto>, BridgeError> {
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
}
