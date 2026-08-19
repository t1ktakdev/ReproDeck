use regex::Regex;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

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
    #[error("execution not found: {0}")]
    ExecutionNotFound(String),
    #[error("pagination limit must be between 1 and 500")]
    InvalidLimit,
}

fn unix_time_secs() -> Result<i64, TimelineError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn checked_limit(limit: usize) -> Result<i64, TimelineError> {
    if !(1..=500).contains(&limit) {
        return Err(TimelineError::InvalidLimit);
    }
    Ok(limit as i64)
}

fn bearer_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)bearer\s+[A-Za-z0-9\-\._~\+\/]+=*")
            .expect("static bearer regex must compile")
    })
}

fn key_value_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(password|token|secret)\s*[=:]\s*[^\s,;]+")
            .expect("static key/value regex must compile")
    })
}

fn jwt_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+")
            .expect("static JWT regex must compile")
    })
}

fn aws_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"AKIA[0-9A-Z]{16}").expect("static AWS access-key regex must compile")
    })
}

fn long_hex_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[0-9a-fA-F]{40,64}\b").expect("static long-hex regex must compile")
    })
}

fn long_token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9_\-]{40,}\b").expect("static long-token regex must compile")
    })
}

fn sanitize_preview(input: &str) -> String {
    let mut value = bearer_regex().replace_all(input, "[REDACTED]").into_owned();
    value = key_value_regex()
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned();
    value = jwt_regex()
        .replace_all(&value, "[REDACTED_JWT]")
        .into_owned();
    value = aws_key_regex()
        .replace_all(&value, "[REDACTED_AWS_KEY]")
        .into_owned();
    value = long_hex_regex()
        .replace_all(&value, "[REDACTED_TOKEN]")
        .into_owned();
    long_token_regex()
        .replace_all(&value, "[REDACTED_TOKEN]")
        .into_owned()
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
) -> Result<(), TimelineError> {
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO sessions (id, created_at, updated_at, state, meta) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![public_id, now, now, state, meta],
    )?;
    Ok(())
}

pub fn get_session(
    conn: &Connection,
    public_id: &str,
) -> Result<Option<(String, i64)>, TimelineError> {
    Ok(conn
        .query_row(
            "SELECT id, created_at FROM sessions WHERE id = ?1",
            rusqlite::params![public_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn get_session_record(
    conn: &Connection,
    public_id: &str,
) -> Result<Option<SessionRecord>, TimelineError> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, repo_id, created_at, updated_at, state, meta FROM sessions WHERE id = ?1",
            rusqlite::params![public_id],
            |row| {
                Ok(SessionRecord {
                    created_seq: row.get(0)?,
                    id: row.get(1)?,
                    repo_id: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    state: row.get(5)?,
                    meta: row.get(6)?,
                })
            },
        )
        .optional()?)
}

pub fn list_sessions(
    conn: &Connection,
    before_seq: Option<i64>,
    limit: usize,
) -> Result<Vec<SessionRecord>, TimelineError> {
    let limit = checked_limit(limit)?;
    let cursor = before_seq.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, repo_id, created_at, updated_at, state, meta FROM sessions WHERE created_seq < ?1 ORDER BY created_seq DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![cursor, limit], |row| {
        Ok(SessionRecord {
            created_seq: row.get(0)?,
            id: row.get(1)?,
            repo_id: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            state: row.get(5)?,
            meta: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn create_action(conn: &Connection, action: &Action) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO actions (id, session_id, parent_id, kind, meta, state, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![action.id, action.session_id, action.parent_id, action.kind, action.meta, action.state, action.created_at],
    )?;
    Ok(())
}

pub fn get_action(
    conn: &Connection,
    action_id: &str,
) -> Result<Option<ActionRecord>, TimelineError> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, session_id, parent_id, kind, meta, state, created_at FROM actions WHERE id = ?1",
            rusqlite::params![action_id],
            |row| {
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
            },
        )
        .optional()?)
}

pub fn list_actions(
    conn: &Connection,
    session_id: &str,
    before_seq: Option<i64>,
    limit: usize,
) -> Result<Vec<ActionRecord>, TimelineError> {
    let limit = checked_limit(limit)?;
    let cursor = before_seq.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, session_id, parent_id, kind, meta, state, created_at FROM actions WHERE session_id = ?1 AND created_seq < ?2 ORDER BY created_seq DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id, cursor, limit], |row| {
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
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn start_execution(conn: &Connection, action_id: &str) -> Result<String, TimelineError> {
    let execution_id = Uuid::new_v4().to_string();
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO executions (id, action_id, status, started_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![execution_id, action_id, "Running", now],
    )?;
    Ok(execution_id)
}

pub fn get_execution(
    conn: &Connection,
    execution_id: &str,
) -> Result<Option<ExecutionRecord>, TimelineError> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, action_id, status, started_at, finished_at, duration_ms FROM executions WHERE id = ?1",
            rusqlite::params![execution_id],
            |row| {
                Ok(ExecutionRecord {
                    created_seq: row.get(0)?,
                    id: row.get(1)?,
                    action_id: row.get(2)?,
                    status: row.get(3)?,
                    started_at: row.get(4)?,
                    finished_at: row.get(5)?,
                    duration_ms: row.get(6)?,
                })
            },
        )
        .optional()?)
}

pub fn list_executions(
    conn: &Connection,
    action_id: &str,
) -> Result<Vec<ExecutionRecord>, TimelineError> {
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, action_id, status, started_at, finished_at, duration_ms FROM executions WHERE action_id = ?1 ORDER BY created_seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![action_id], |row| {
        Ok(ExecutionRecord {
            created_seq: row.get(0)?,
            id: row.get(1)?,
            action_id: row.get(2)?,
            status: row.get(3)?,
            started_at: row.get(4)?,
            finished_at: row.get(5)?,
            duration_ms: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn finish_execution_in_transaction(
    tx: &Transaction<'_>,
    execution_id: &str,
    status: &str,
    stdout_preview: Option<&str>,
    stderr_preview: Option<&str>,
) -> Result<String, TimelineError> {
    let now = unix_time_secs()?;
    let started_at = tx
        .query_row(
            "SELECT started_at FROM executions WHERE id = ?1 AND finished_at IS NULL",
            rusqlite::params![execution_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| TimelineError::ExecutionNotFound(execution_id.to_owned()))?;
    let duration_ms = now.saturating_sub(started_at).saturating_mul(1000);
    let updated = tx.execute(
        "UPDATE executions SET status = ?1, finished_at = ?2, duration_ms = ?3 WHERE id = ?4 AND finished_at IS NULL",
        rusqlite::params![status, now, duration_ms, execution_id],
    )?;
    if updated != 1 {
        return Err(TimelineError::ExecutionNotFound(execution_id.to_owned()));
    }

    const MAX_PREVIEW: usize = 1024;
    let (stdout_preview, stdout_truncated) = match stdout_preview {
        Some(value) => {
            let sanitized = sanitize_preview(value);
            let (bounded, truncated) = truncate_utf8_bytes(&sanitized, MAX_PREVIEW);
            (Some(bounded), i64::from(truncated))
        }
        None => (None, 0),
    };
    let (stderr_preview, stderr_truncated) = match stderr_preview {
        Some(value) => {
            let sanitized = sanitize_preview(value);
            let (bounded, truncated) = truncate_utf8_bytes(&sanitized, MAX_PREVIEW);
            (Some(bounded), i64::from(truncated))
        }
        None => (None, 0),
    };

    let receipt_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO receipts (id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![receipt_id, execution_id, "", stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, now],
    )?;
    Ok(receipt_id)
}

pub fn finish_execution(
    conn: &mut Connection,
    execution_id: &str,
    status: &str,
    stdout_preview: Option<&str>,
    stderr_preview: Option<&str>,
) -> Result<String, TimelineError> {
    let tx = conn.transaction()?;
    let receipt_id =
        finish_execution_in_transaction(&tx, execution_id, status, stdout_preview, stderr_preview)?;
    tx.commit()?;
    Ok(receipt_id)
}

pub fn get_receipt(
    conn: &Connection,
    receipt_id: &str,
) -> Result<Option<ReceiptRecord>, TimelineError> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at FROM receipts WHERE id = ?1",
            rusqlite::params![receipt_id],
            |row| {
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
            },
        )
        .optional()?)
}

pub fn list_receipts(
    conn: &Connection,
    execution_id: &str,
) -> Result<Vec<ReceiptRecord>, TimelineError> {
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at FROM receipts WHERE execution_id = ?1 ORDER BY created_seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![execution_id], |row| {
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
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn recover_running(conn: &mut Connection) -> Result<usize, TimelineError> {
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

    fn setup_session(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![id, "repo", 1, 1, "Active"],
        )
        .unwrap();
    }

    fn action(id: &str, session_id: &str) -> Action {
        Action {
            id: id.to_string(),
            session_id: session_id.to_string(),
            parent_id: None,
            kind: "command".to_string(),
            meta: Some("{}".to_string()),
            state: "Created".to_string(),
            created_at: 1,
        }
    }

    #[test]
    fn create_and_read_action() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        setup_session(&conn, "s-1");
        create_action(&conn, &action("act-1", "s-1")).unwrap();
        assert_eq!(get_action(&conn, "act-1").unwrap().unwrap().id, "act-1");
    }

    #[test]
    fn duplicate_uuid_rejected() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        setup_session(&conn, "dup-s");
        assert!(conn.execute(
            "INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params!["dup-s", "r", 2, 2, "Active"],
        ).is_err());
    }

    #[test]
    fn stable_action_pagination_has_no_overlap() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        setup_session(&conn, "s-pag");
        for i in 0..5 {
            create_action(&conn, &action(&format!("a-{i}"), "s-pag")).unwrap();
        }
        let first = list_actions(&conn, "s-pag", None, 2).unwrap();
        let second =
            list_actions(&conn, "s-pag", Some(first.last().unwrap().created_seq), 10).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 3);
        assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
    }

    #[test]
    fn invalid_pagination_limit_rejected() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        assert!(matches!(
            list_sessions(&conn, None, 0),
            Err(TimelineError::InvalidLimit)
        ));
        assert!(matches!(
            list_sessions(&conn, None, 501),
            Err(TimelineError::InvalidLimit)
        ));
    }

    #[test]
    fn execution_and_receipt_query_round_trip() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        setup_session(&conn, "s-roundtrip");
        create_action(&conn, &action("a-roundtrip", "s-roundtrip")).unwrap();
        let execution_id = start_execution(&conn, "a-roundtrip").unwrap();
        let receipt_id = finish_execution(
            &mut conn,
            &execution_id,
            "Succeeded",
            Some("hello"),
            Some("warning"),
        )
        .unwrap();
        let execution = get_execution(&conn, &execution_id).unwrap().unwrap();
        let receipt = get_receipt(&conn, &receipt_id).unwrap().unwrap();
        assert_eq!(execution.status, "Succeeded");
        assert_eq!(receipt.execution_id, execution_id);
        assert_eq!(receipt.stdout_preview.as_deref(), Some("hello"));
        assert_eq!(list_receipts(&conn, &execution.id).unwrap().len(), 1);
    }

    #[test]
    fn finishing_execution_twice_is_rejected() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        setup_session(&conn, "s-double");
        create_action(&conn, &action("a-double", "s-double")).unwrap();
        let execution_id = start_execution(&conn, "a-double").unwrap();
        finish_execution(&mut conn, &execution_id, "Succeeded", None, None).unwrap();
        assert!(matches!(
            finish_execution(&mut conn, &execution_id, "Succeeded", None, None),
            Err(TimelineError::ExecutionNotFound(_))
        ));
    }

    #[test]
    fn sanitization_precedes_persistence() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s-sec", "Active", None).unwrap();
        create_action(&conn, &action("asec", "s-sec")).unwrap();
        let execution_id = start_execution(&conn, "asec").unwrap();
        let receipt_id = finish_execution(
            &mut conn,
            &execution_id,
            "Succeeded",
            Some("Bearer abcdef12345== password=hunter2"),
            None,
        )
        .unwrap();
        let preview = get_receipt(&conn, &receipt_id)
            .unwrap()
            .unwrap()
            .stdout_preview
            .unwrap();
        assert!(!preview.contains("abcdef12345"));
        assert!(!preview.contains("hunter2"));
        assert!(preview.contains("REDACTED"));
    }

    #[test]
    fn sanitization_detects_jwt_and_aws() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s-sec2", "Active", None).unwrap();
        create_action(&conn, &action("ajwt", "s-sec2")).unwrap();
        let execution_id = start_execution(&conn, "ajwt").unwrap();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature";
        let aws = format!("AKIA{}", "A".repeat(16));
        let input = format!("start {jwt} middle {aws} end");
        let receipt_id =
            finish_execution(&mut conn, &execution_id, "Succeeded", Some(&input), None).unwrap();
        let preview = get_receipt(&conn, &receipt_id)
            .unwrap()
            .unwrap()
            .stdout_preview
            .unwrap();
        assert!(!preview.contains("eyJhbGci"));
        assert!(!preview.contains("AKIA"));
    }

    #[test]
    fn unicode_preview_truncation_is_utf8_safe() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s-unicode", "Active", None).unwrap();
        create_action(&conn, &action("a-unicode", "s-unicode")).unwrap();
        let execution_id = start_execution(&conn, "a-unicode").unwrap();
        let output = format!("{}{}", "😀".repeat(300), "русский-текст".repeat(50));
        let receipt_id =
            finish_execution(&mut conn, &execution_id, "Succeeded", Some(&output), None).unwrap();
        let receipt = get_receipt(&conn, &receipt_id).unwrap().unwrap();
        let preview = receipt.stdout_preview.unwrap();
        assert!(preview.len() <= 1024);
        assert!(receipt.stdout_truncated);
        assert!(preview.is_char_boundary(preview.len()));
    }

    #[test]
    fn session_action_foreign_key_cascades() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s-123", "Active", Some("{}")).unwrap();
        create_action(&conn, &action("act-2", "s-123")).unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params!["s-123"],
        )
        .unwrap();
        assert!(get_action(&conn, "act-2").unwrap().is_none());
    }

    #[test]
    fn running_to_interrupted_on_recover() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = init_db(tmp.path()).unwrap();
        create_session(&conn, "s-rcv", "Active", None).unwrap();
        create_action(&conn, &action("act-r", "s-rcv")).unwrap();
        let execution_id = start_execution(&conn, "act-r").unwrap();
        assert_eq!(recover_running(&mut conn).unwrap(), 1);
        let execution = get_execution(&conn, &execution_id).unwrap().unwrap();
        assert_eq!(execution.status, "Interrupted");
        assert!(execution.finished_at.is_some());
    }
}
