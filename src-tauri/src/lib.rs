// Tauri command bridge to ReproDeck core APIs
// serde::Serialize imported by other modules when needed
use serde_json::json;
use std::path::PathBuf;

use reprodeck_core::db;
use reprodeck_core::timeline;
use reprodeck_core::verification;

fn app_db_path() -> PathBuf {
    if let Some(b) = directories::BaseDirs::new() {
        let mut p = b.data_local_dir().to_path_buf();
        p.push("reprodeck");
        std::fs::create_dir_all(&p).ok();
        p.push("reprodeck.db");
        p
    } else {
        // fallback to temp dir
        let mut p = std::env::temp_dir();
        p.push("reprodeck.db");
        p
    }
}

fn open_conn() -> Result<rusqlite::Connection, String> {
    let p = app_db_path();
    db::init_db(&p).map_err(|_e| "db init error".to_string())
}

#[tauri::command]
pub fn list_sessions() -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, updated_at, state, meta FROM sessions ORDER BY created_at DESC",
        )
        .map_err(|_| "query error")?;
    let mut rows = stmt.query([]).map_err(|_| "query error")?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(|_| "row error")? {
        let id: String = r.get(0).unwrap_or_default();
        let created_at: i64 = r.get(1).unwrap_or(0);
        let updated_at: Option<i64> = r.get(2).ok();
        let state: String = r.get(3).unwrap_or_default();
        let meta: Option<String> = r.get(4).ok();
        out.push(json!({"id": id, "created_at": created_at, "updated_at": updated_at, "state": state, "meta": meta }));
    }
    Ok(serde_json::Value::Array(out))
}

#[tauri::command]
pub fn create_session(id: &str) -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    timeline::create_session(&conn, id, "Active", None).map_err(|_| "create session failed")?;
    Ok(json!({"id": id}))
}

#[tauri::command]
pub fn list_actions(session_id: &str) -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, kind, state, created_at FROM actions WHERE session_id = ?1 ORDER BY created_seq DESC")
        .map_err(|_| "query error")?;
    let mut rows = stmt
        .query(rusqlite::params![session_id])
        .map_err(|_| "query error")?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(|_| "row error")? {
        let id: String = r.get(0).unwrap_or_default();
        let kind: String = r.get(1).unwrap_or_default();
        let state: String = r.get(2).unwrap_or_default();
        let created_at: i64 = r.get(3).unwrap_or(0);
        out.push(json!({"id": id, "kind": kind, "state": state, "created_at": created_at }));
    }
    Ok(serde_json::Value::Array(out))
}

#[tauri::command]
pub fn get_receipt(receipt_id: &str) -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, execution_id, stdout_preview, stderr_preview, created_at FROM receipts WHERE id = ?1")
        .map_err(|_| "query error")?;
    let row = stmt.query_row(rusqlite::params![receipt_id], |r| {
        let id: String = r.get(0)?;
        let execution_id: String = r.get(1)?;
        let stdout_preview: Option<String> = r.get(2)?;
        let stderr_preview: Option<String> = r.get(3)?;
        let created_at: i64 = r.get(4)?;
        Ok(json!({"id": id, "execution_id": execution_id, "stdout_preview": stdout_preview, "stderr_preview": stderr_preview, "created_at": created_at }))
    }).map_err(|_| "not found")?;
    Ok(row)
}

#[tauri::command]
pub fn list_contracts() -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, session_id, title, description, state, version, created_at FROM outcome_contracts ORDER BY created_at DESC")
        .map_err(|_| "query error")?;
    let mut rows = stmt.query([]).map_err(|_| "query error")?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(|_| "row error")? {
        let id: String = r.get(0).unwrap_or_default();
        let session_id: String = r.get(1).unwrap_or_default();
        let title: String = r.get(2).unwrap_or_default();
        let description: Option<String> = r.get(3).ok();
        let state: String = r.get(4).unwrap_or_default();
        let version: i64 = r.get(5).unwrap_or(1);
        let created_at: i64 = r.get(6).unwrap_or(0);
        out.push(json!({"id": id, "session_id": session_id, "title": title, "description": description, "state": state, "version": version, "created_at": created_at }));
    }
    Ok(serde_json::Value::Array(out))
}

#[tauri::command]
pub fn list_verification_runs(contract_id: &str) -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, check_id, phase, status, started_at, finished_at, receipt_id FROM verification_runs WHERE contract_id = ?1 ORDER BY started_at DESC")
        .map_err(|_| "query error")?;
    let mut rows = stmt
        .query(rusqlite::params![contract_id])
        .map_err(|_| "query error")?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(|_| "row error")? {
        let id: String = r.get(0).unwrap_or_default();
        let check_id: Option<String> = r.get(1).ok();
        let phase: String = r.get(2).unwrap_or_default();
        let status: String = r.get(3).unwrap_or_default();
        let started_at: Option<i64> = r.get(4).ok();
        let finished_at: Option<i64> = r.get(5).ok();
        let receipt_id: Option<String> = r.get(6).ok();
        out.push(json!({"id": id, "check_id": check_id, "phase": phase, "status": status, "started_at": started_at, "finished_at": finished_at, "receipt_id": receipt_id }));
    }
    Ok(serde_json::Value::Array(out))
}

#[tauri::command]
pub fn evaluate_contract(contract_id: &str) -> Result<serde_json::Value, String> {
    let conn = open_conn()?;
    match verification::evaluate_outcome(&conn, contract_id) {
        Ok(s) => Ok(json!({"verdict": s})),
        Err(_) => Err("evaluation failed".to_string()),
    }
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
