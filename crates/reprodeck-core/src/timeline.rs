
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub meta: Option<String>,
    pub state: String,
    pub created_at: i64,
}

pub fn create_action(conn: &Connection, a: &Action) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO actions (id, session_id, parent_id, kind, meta, state, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![a.id, a.session_id, a.parent_id, a.kind, a.meta, a.state, a.created_at],
    )?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum TimelineError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

pub fn create_session(conn: &Connection, public_id: &str, state: &str, meta: Option<&str>) -> Result<(), TimelineError> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    conn.execute(
        "INSERT INTO sessions (id, created_at, updated_at, state, meta) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![public_id, now, now, state, meta],
    )?;
    Ok(())
}

pub fn get_session(conn: &Connection, public_id: &str) -> Result<Option<(String,i64)>, TimelineError> {
    let mut stmt = conn.prepare("SELECT id, created_at FROM sessions WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![public_id])?;
    if let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let created_at: i64 = r.get(1)?;
        Ok(Some((id, created_at)))
    } else {
        Ok(None)
    }
}

pub fn start_execution(conn: &Connection, action_id: &str) -> Result<String, TimelineError> {
    let exec_id = Uuid::new_v4().to_string();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;

    conn.execute("INSERT INTO executions (id, action_id, status, started_at) VALUES (?1,?2,?3,?4)", rusqlite::params![exec_id, action_id, "Running", now])?;
    Ok(exec_id)
}

/// finish_execution inserts receipt and optional artifact metadata atomically.
pub fn finish_execution(conn: &mut Connection, execution_id: &str, status: &str, stdout_preview: Option<&str>, stderr_preview: Option<&str>) -> Result<String, TimelineError> {
    let tx = conn.transaction()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;

    // update execution
    tx.execute("UPDATE executions SET status = ?1, finished_at = ?2 WHERE id = ?3", rusqlite::params![status, now, execution_id])?;

    // insert receipt
    let receipt_id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO receipts (id, execution_id, summary, stdout_preview, stderr_preview, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![receipt_id, execution_id, "", stdout_preview, stderr_preview, now])?;

    tx.commit()?;
    Ok(receipt_id)
}

/// Recovery: mark Running -> Interrupted on startup
pub fn recover_running(conn: &mut Connection) -> Result<usize, TimelineError> {
    let tx = conn.transaction()?;
    let res = tx.execute("UPDATE executions SET status = 'Interrupted' WHERE status = 'Running' AND finished_at IS NULL", [])?;
    tx.commit()?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::db::init_db;

    #[test]
    fn create_and_read_action() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = init_db(path).expect("init db");

        let a = Action {
            id: "act-1".to_string(),
            session_id: "s-1".to_string(),
            parent_id: None,
            kind: "test".to_string(),
            meta: Some("{}".to_string()),
            state: "Created".to_string(),
            created_at: 1,
        };

        // session must exist per FK; create minimal session
        conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["s-1","repo-x",1,1,"Active"]).unwrap();

        create_action(&conn, &a).expect("insert");

        let mut stmt = conn.prepare("SELECT id, session_id, kind FROM actions WHERE id = ?1").unwrap();
        let mut rows = stmt.query(rusqlite::params!["act-1"]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let id: String = row.get(0).unwrap();
        assert_eq!(id, "act-1");
    }

    #[test]
    fn session_action_foreign_key() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");

        // create session
        create_session(&conn, "s-123", "Active", Some("{}")) .expect("create session");

        let a = Action {
            id: "act-2".to_string(),
            session_id: "s-123".to_string(),
            parent_id: None,
            kind: "test".to_string(),
            meta: None,
            state: "Created".to_string(),
            created_at: 1,
        };

        create_action(&conn, &a).expect("insert action");

        // deleting session should cascade to actions
        conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params!["s-123"]).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM actions WHERE id = ?1", rusqlite::params!["act-2"], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn running_to_interrupted_on_recover() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");

        // prepare session & action
        create_session(&conn, "s-rcv", "Active", None).unwrap();
        let a = Action { id: "act-r".to_string(), session_id: "s-rcv".to_string(), parent_id: None, kind: "k".to_string(), meta: None, state: "Created".to_string(), created_at: 1 };
        create_action(&conn, &a).unwrap();

        // start execution
        let exec_id = start_execution(&conn, "act-r").unwrap();

        // simulate restart by calling recover_running
        let changed = recover_running(&mut conn).unwrap();
        assert!(changed >= 1);

        // check status
        let status: String = conn.query_row("SELECT status FROM executions WHERE id = ?1", rusqlite::params![exec_id], |r| r.get(0)).unwrap();
        assert_eq!(status, "Interrupted");
    }
}
