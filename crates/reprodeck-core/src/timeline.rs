use crate::redaction;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub created_seq: i64,
    pub id: String,
    pub repo_id: Option<String>,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub state: String,
    pub meta: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub meta: Option<String>,
    pub state: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionRecord {
    pub created_seq: i64,
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub meta: Option<String>,
    pub state: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub created_seq: i64,
    pub id: String,
    pub action_id: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptRecord {
    pub created_seq: i64,
    pub id: String,
    pub execution_id: String,
    pub summary: Option<String>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub created_at: i64,
}

#[derive(Debug, Error)]
pub enum TimelineError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("execution not found or already finished: {0}")]
    ExecutionNotFound(String),
    #[error("pagination limit must be between 1 and 500")]
    InvalidLimit,
}

pub type Result<T> = std::result::Result<T, TimelineError>;

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn unix_time_millis() -> Result<i64> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(millis.min(i64::MAX as u128) as i64)
}

fn checked_limit(limit: usize) -> Result<i64> {
    if !(1..=500).contains(&limit) {
        return Err(TimelineError::InvalidLimit);
    }
    Ok(limit as i64)
}

fn truncate_utf8_bytes(input: &str, max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (input.to_owned(), false);
    }
    let mut end = max_bytes.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (input[..end].to_owned(), true)
}

pub fn create_session(
    conn: &Connection,
    public_id: &str,
    state: &str,
    meta: Option<&str>,
) -> Result<()> {
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO sessions (id, created_at, updated_at, state, meta) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![public_id, now, now, state, meta],
    )?;
    Ok(())
}

pub fn get_session(conn: &Connection, public_id: &str) -> Result<Option<(String, i64)>> {
    Ok(conn
        .query_row(
            "SELECT id, created_at FROM sessions WHERE id = ?1",
            rusqlite::params![public_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn get_session_record(conn: &Connection, public_id: &str) -> Result<Option<SessionRecord>> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, repo_id, created_at, updated_at, state, meta FROM sessions WHERE id = ?1",
            rusqlite::params![public_id],
            |row| {
                Ok(SessionRecord {
                    created_seq: row.get(0)?, id: row.get(1)?, repo_id: row.get(2)?,
                    created_at: row.get(3)?, updated_at: row.get(4)?, state: row.get(5)?, meta: row.get(6)?,
                })
            },
        )
        .optional()?)
}

pub fn list_sessions(
    conn: &Connection,
    before_seq: Option<i64>,
    limit: usize,
) -> Result<Vec<SessionRecord>> {
    let limit = checked_limit(limit)?;
    let cursor = before_seq.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, repo_id, created_at, updated_at, state, meta FROM sessions WHERE created_seq < ?1 ORDER BY created_seq DESC LIMIT ?2",
    )?;
    let records = stmt
        .query_map(rusqlite::params![cursor, limit], |row| {
            Ok(SessionRecord {
                created_seq: row.get(0)?,
                id: row.get(1)?,
                repo_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                state: row.get(5)?,
                meta: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn update_session_state(conn: &Connection, session_id: &str, state: &str) -> Result<()> {
    let now = unix_time_secs()?;
    conn.execute(
        "UPDATE sessions SET state = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![state, now, session_id],
    )?;
    Ok(())
}

pub fn update_session_meta(conn: &Connection, session_id: &str, meta: Option<&str>) -> Result<()> {
    let now = unix_time_secs()?;
    conn.execute(
        "UPDATE sessions SET meta = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![meta, now, session_id],
    )?;
    Ok(())
}

pub fn create_action(
    conn: &Connection,
    action: &Action,
) -> std::result::Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO actions (id, session_id, parent_id, kind, meta, state, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![action.id, action.session_id, action.parent_id, action.kind, action.meta, action.state, action.created_at],
    )?;
    Ok(())
}

pub fn new_action(
    session_id: &str,
    kind: &str,
    state: &str,
    meta: Option<String>,
) -> Result<Action> {
    Ok(Action {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_owned(),
        parent_id: None,
        kind: kind.to_owned(),
        meta,
        state: state.to_owned(),
        created_at: unix_time_secs()?,
    })
}

pub fn get_action(conn: &Connection, action_id: &str) -> Result<Option<ActionRecord>> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, session_id, parent_id, kind, meta, state, created_at FROM actions WHERE id = ?1",
            rusqlite::params![action_id],
            |row| {
                Ok(ActionRecord {
                    created_seq: row.get(0)?, id: row.get(1)?, session_id: row.get(2)?, parent_id: row.get(3)?,
                    kind: row.get(4)?, meta: row.get(5)?, state: row.get(6)?, created_at: row.get(7)?,
                })
            },
        )
        .optional()?)
}

pub fn list_actions(
    conn: &Connection,
    session_id: &str,
    before_seq: Option<i64>,
    limit: usize,
) -> Result<Vec<ActionRecord>> {
    let limit = checked_limit(limit)?;
    let cursor = before_seq.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, session_id, parent_id, kind, meta, state, created_at FROM actions WHERE session_id = ?1 AND created_seq < ?2 ORDER BY created_seq DESC LIMIT ?3",
    )?;
    let records = stmt
        .query_map(rusqlite::params![session_id, cursor, limit], |row| {
            Ok(ActionRecord {
                created_seq: row.get(0)?,
                id: row.get(1)?,
                session_id: row.get(2)?,
                parent_id: row.get(3)?,
                kind: row.get(4)?,
                meta: row.get(5)?,
                state: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn start_execution(conn: &Connection, action_id: &str) -> Result<String> {
    let execution_id = Uuid::new_v4().to_string();
    let now = unix_time_millis()?;
    conn.execute(
        "INSERT INTO executions (id, action_id, status, started_at) VALUES (?1,?2,'Running',?3)",
        rusqlite::params![execution_id, action_id, now],
    )?;
    Ok(execution_id)
}

pub fn get_execution(conn: &Connection, execution_id: &str) -> Result<Option<ExecutionRecord>> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, action_id, status, started_at, finished_at, duration_ms FROM executions WHERE id = ?1",
            rusqlite::params![execution_id],
            |row| Ok(ExecutionRecord { created_seq: row.get(0)?, id: row.get(1)?, action_id: row.get(2)?, status: row.get(3)?, started_at: row.get(4)?, finished_at: row.get(5)?, duration_ms: row.get(6)? }),
        )
        .optional()?)
}

pub fn list_executions(conn: &Connection, action_id: &str) -> Result<Vec<ExecutionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, action_id, status, started_at, finished_at, duration_ms FROM executions WHERE action_id = ?1 ORDER BY created_seq ASC",
    )?;
    let records = stmt
        .query_map(rusqlite::params![action_id], |row| {
            Ok(ExecutionRecord {
                created_seq: row.get(0)?,
                id: row.get(1)?,
                action_id: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                duration_ms: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(records)
}

pub(crate) fn finish_execution_in_transaction(
    tx: &Transaction<'_>,
    execution_id: &str,
    status: &str,
    stdout_preview: Option<&str>,
    stderr_preview: Option<&str>,
) -> Result<String> {
    let finished_at = unix_time_millis()?;
    let receipt_created_at = unix_time_secs()?;
    let started_at = tx
        .query_row(
            "SELECT started_at FROM executions WHERE id = ?1 AND finished_at IS NULL",
            rusqlite::params![execution_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| TimelineError::ExecutionNotFound(execution_id.to_owned()))?;
    // Older development snapshots stored execution timestamps in seconds. Keep
    // finishing those rows sane while new rows use millisecond precision.
    let started_at_ms = if started_at < 10_000_000_000 {
        started_at.saturating_mul(1000)
    } else {
        started_at
    };
    let duration_ms = finished_at.saturating_sub(started_at_ms);
    let updated = tx.execute(
        "UPDATE executions SET status = ?1, finished_at = ?2, duration_ms = ?3 WHERE id = ?4 AND finished_at IS NULL",
        rusqlite::params![status, finished_at, duration_ms, execution_id],
    )?;
    if updated != 1 {
        return Err(TimelineError::ExecutionNotFound(execution_id.to_owned()));
    }

    const MAX_PREVIEW: usize = 16 * 1024;
    let (stdout_preview, stdout_truncated) = stdout_preview
        .map(|value| truncate_utf8_bytes(&redaction::redact_text(value), MAX_PREVIEW))
        .map(|(v, t)| (Some(v), i64::from(t)))
        .unwrap_or((None, 0));
    let (stderr_preview, stderr_truncated) = stderr_preview
        .map(|value| truncate_utf8_bytes(&redaction::redact_text(value), MAX_PREVIEW))
        .map(|(v, t)| (Some(v), i64::from(t)))
        .unwrap_or((None, 0));

    let receipt_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO receipts (id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at) VALUES (?1,?2,NULL,?3,?4,?5,?6,?7)",
        rusqlite::params![receipt_id, execution_id, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, receipt_created_at],
    )?;
    Ok(receipt_id)
}

pub fn finish_execution(
    conn: &mut Connection,
    execution_id: &str,
    status: &str,
    stdout_preview: Option<&str>,
    stderr_preview: Option<&str>,
) -> Result<String> {
    let tx = conn.transaction()?;
    let receipt_id =
        finish_execution_in_transaction(&tx, execution_id, status, stdout_preview, stderr_preview)?;
    tx.commit()?;
    Ok(receipt_id)
}

pub fn get_receipt(conn: &Connection, receipt_id: &str) -> Result<Option<ReceiptRecord>> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at FROM receipts WHERE id = ?1",
            rusqlite::params![receipt_id],
            |row| Ok(ReceiptRecord { created_seq: row.get(0)?, id: row.get(1)?, execution_id: row.get(2)?, summary: row.get(3)?, stdout_preview: row.get(4)?, stderr_preview: row.get(5)?, stdout_truncated: row.get::<_, i64>(6)? != 0, stderr_truncated: row.get::<_, i64>(7)? != 0, created_at: row.get(8)? }),
        )
        .optional()?)
}

pub fn list_receipts(conn: &Connection, execution_id: &str) -> Result<Vec<ReceiptRecord>> {
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at FROM receipts WHERE execution_id = ?1 ORDER BY created_seq ASC",
    )?;
    let records = stmt
        .query_map(rusqlite::params![execution_id], |row| {
            Ok(ReceiptRecord {
                created_seq: row.get(0)?,
                id: row.get(1)?,
                execution_id: row.get(2)?,
                summary: row.get(3)?,
                stdout_preview: row.get(4)?,
                stderr_preview: row.get(5)?,
                stdout_truncated: row.get::<_, i64>(6)? != 0,
                stderr_truncated: row.get::<_, i64>(7)? != 0,
                created_at: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn recover_running(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE executions SET status = 'Interrupted', finished_at = COALESCE(finished_at, started_at), duration_ms = COALESCE(duration_ms, 0) WHERE status = 'Running' AND finished_at IS NULL",
        [],
    )?;
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::NamedTempFile;

    fn action(id: &str, session_id: &str) -> Action {
        Action {
            id: id.into(),
            session_id: session_id.into(),
            parent_id: None,
            kind: "command".into(),
            meta: None,
            state: "Running".into(),
            created_at: 1,
        }
    }

    #[test]
    fn unicode_preview_truncation_is_safe() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s", "Draft", None).unwrap();
        create_action(&conn, &action("a", "s")).unwrap();
        let exec = start_execution(&conn, "a").unwrap();
        let output = "😀".repeat(8000);
        let receipt = finish_execution(&mut conn, &exec, "Succeeded", Some(&output), None).unwrap();
        let stored = get_receipt(&conn, &receipt)
            .unwrap()
            .unwrap()
            .stdout_preview
            .unwrap();
        assert!(stored.len() <= 16 * 1024);
        assert!(stored.is_char_boundary(stored.len()));
    }

    #[test]
    fn preview_is_redacted_before_persistence() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s", "Draft", None).unwrap();
        create_action(&conn, &action("a", "s")).unwrap();
        let exec = start_execution(&conn, "a").unwrap();
        let receipt =
            finish_execution(&mut conn, &exec, "Failed", Some("password=hunter2"), None).unwrap();
        let stored = get_receipt(&conn, &receipt)
            .unwrap()
            .unwrap()
            .stdout_preview
            .unwrap();
        assert!(!stored.contains("hunter2"));
    }
}
