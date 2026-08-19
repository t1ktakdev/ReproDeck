use regex::Regex;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

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
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("execution not found: {0}")]
    ExecutionNotFound(String),
}

fn unix_time_secs() -> Result<i64, TimelineError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
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
    let mut s = bearer_regex().replace_all(input, "[REDACTED]").into_owned();
    s = key_value_regex()
        .replace_all(&s, "$1=[REDACTED]")
        .into_owned();
    s = jwt_regex().replace_all(&s, "[REDACTED_JWT]").into_owned();
    s = aws_key_regex()
        .replace_all(&s, "[REDACTED_AWS_KEY]")
        .into_owned();
    s = long_hex_regex()
        .replace_all(&s, "[REDACTED_TOKEN]")
        .into_owned();
    long_token_regex()
        .replace_all(&s, "[REDACTED_TOKEN]")
        .into_owned()
}

/// Truncate a UTF-8 string to at most `max_bytes` without slicing inside a
/// multi-byte scalar value.
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
    let now = unix_time_secs()?;

    conn.execute(
        "INSERT INTO executions (id, action_id, status, started_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![exec_id, action_id, "Running", now],
    )?;
    Ok(exec_id)
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
            "SELECT started_at FROM executions WHERE id = ?1",
            rusqlite::params![execution_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| TimelineError::ExecutionNotFound(execution_id.to_owned()))?;
    let duration_ms = now.saturating_sub(started_at).saturating_mul(1000);

    let updated = tx.execute(
        "UPDATE executions SET status = ?1, finished_at = ?2, duration_ms = ?3 WHERE id = ?4",
        rusqlite::params![status, now, duration_ms, execution_id],
    )?;
    if updated != 1 {
        return Err(TimelineError::ExecutionNotFound(execution_id.to_owned()));
    }

    const MAX_PREVIEW: usize = 1024;
    let (stdout_preview, stdout_truncated) = match stdout_preview {
        Some(s) => {
            let sanitized = sanitize_preview(s);
            let (bounded, truncated) = truncate_utf8_bytes(&sanitized, MAX_PREVIEW);
            (Some(bounded), if truncated { 1_i64 } else { 0_i64 })
        }
        None => (None, 0),
    };
    let (stderr_preview, stderr_truncated) = match stderr_preview {
        Some(s) => {
            let sanitized = sanitize_preview(s);
            let (bounded, truncated) = truncate_utf8_bytes(&sanitized, MAX_PREVIEW);
            (Some(bounded), if truncated { 1_i64 } else { 0_i64 })
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

/// Finish an execution and create its receipt in a single transaction.
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

/// Recovery: mark Running -> Interrupted on startup.
pub fn recover_running(conn: &mut Connection) -> Result<usize, TimelineError> {
    let tx = conn.transaction()?;
    let res = tx.execute(
        "UPDATE executions SET status = 'Interrupted' WHERE status = 'Running' AND finished_at IS NULL",
        [],
    )?;
    tx.commit()?;
    Ok(res)
}

#[cfg(test)]
#[allow(unused_mut)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::NamedTempFile;

    #[test]
    fn create_and_read_action() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = init_db(path).expect("init db");

        let a = Action {
            id: "act-1".to_string(),
            session_id: "s-1".to_string(),
            parent_id: None,
            kind: "test".to_string(),
            meta: Some("{}".to_string()),
            state: "Created".to_string(),
            created_at: 1,
        };

        conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["s-1","repo-x",1,1,"Active"]).unwrap();

        create_action(&conn, &a).expect("insert");

        let mut stmt = conn
            .prepare("SELECT id, session_id, kind FROM actions WHERE id = ?1")
            .unwrap();
        let mut rows = stmt.query(rusqlite::params!["act-1"]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let id: String = row.get(0).unwrap();
        assert_eq!(id, "act-1");
    }

    #[test]
    fn duplicate_uuid_rejected() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");
        conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["dup-s","r",1,1,"Active"]).unwrap();
        let res = conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["dup-s","r",2,2,"Active"]);
        assert!(res.is_err());
    }

    #[test]
    fn action_ordering_and_pagination() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");
        conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["s-pag","r",1,1,"Active"]).unwrap();

        for i in 0..5 {
            let id = format!("a-{}", i);
            conn.execute("INSERT INTO actions(id, session_id, kind, state, created_at) VALUES (?1,?2,?3,?4,?5)", rusqlite::params![id, "s-pag", "k", "Created", 1000]).unwrap();
        }

        let mut stmt = conn
            .prepare("SELECT id FROM actions WHERE session_id = ?1 ORDER BY created_seq LIMIT 2")
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params!["s-pag"], |r| r.get::<_, String>(0))
            .unwrap();
        let ids: Vec<String> = rows.map(|r| r.unwrap()).collect();
        assert_eq!(ids.len(), 2);

        let mut stmt2 = conn.prepare("SELECT id FROM actions WHERE session_id = ?1 AND created_seq > (SELECT created_seq FROM actions WHERE id = ?2) ORDER BY created_seq LIMIT 10").unwrap();
        let rows2 = stmt2
            .query_map(rusqlite::params!["s-pag", ids.last().unwrap()], |r| {
                r.get::<_, String>(0)
            })
            .unwrap();
        let ids2: Vec<String> = rows2.map(|r| r.unwrap()).collect();
        assert!(!ids2.is_empty());
    }

    #[test]
    fn sanitization_precedes_persistence() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");
        create_session(&conn, "s-sec", "Active", None).unwrap();
        let a = Action {
            id: "asec".to_string(),
            session_id: "s-sec".to_string(),
            parent_id: None,
            kind: "k".to_string(),
            meta: None,
            state: "Created".to_string(),
            created_at: 1,
        };
        create_action(&conn, &a).unwrap();
        let exec_id = start_execution(&conn, "asec").unwrap();
        let token = "This has Bearer abcdef12345== inside";
        let receipt =
            finish_execution(&mut conn, &exec_id, "Succeeded", Some(token), None).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT stdout_preview FROM receipts WHERE id = ?1",
                rusqlite::params![receipt],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!stored.contains("abcdef12345"));
        assert!(stored.contains("[REDACTED]") || stored.contains("REDACTED"));
    }

    #[test]
    fn sanitization_detects_jwt_and_aws() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");
        create_session(&conn, "s-sec2", "Active", None).unwrap();
        let a = Action {
            id: "ajwt".to_string(),
            session_id: "s-sec2".to_string(),
            parent_id: None,
            kind: "k".to_string(),
            meta: None,
            state: "Created".to_string(),
            created_at: 1,
        };
        create_action(&conn, &a).unwrap();
        let exec_id = start_execution(&conn, "ajwt").unwrap();
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.sgnature";
        let aws = format!("AKIA{}", "A".repeat(16));
        let input = format!("start {} middle {} end", jwt, aws);
        let receipt =
            finish_execution(&mut conn, &exec_id, "Succeeded", Some(&input), None).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT stdout_preview FROM receipts WHERE id = ?1",
                rusqlite::params![receipt],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!stored.contains("eyJhbGci"));
        assert!(!stored.contains("AKIA"));
        assert!(stored.contains("REDACTED_JWT") || stored.contains("REDACTED"));
        assert!(
            stored.contains("REDACTED_AWS_KEY")
                || stored.contains("REDACTED_TOKEN")
                || stored.contains("REDACTED")
        );
    }

    #[test]
    fn unicode_preview_truncation_is_utf8_safe() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");
        create_session(&conn, "s-unicode", "Active", None).unwrap();
        let a = Action {
            id: "a-unicode".to_string(),
            session_id: "s-unicode".to_string(),
            parent_id: None,
            kind: "command".to_string(),
            meta: None,
            state: "Created".to_string(),
            created_at: 1,
        };
        create_action(&conn, &a).unwrap();
        let exec_id = start_execution(&conn, &a.id).unwrap();
        let unicode_output = format!("{}{}", "😀".repeat(300), "русский-текст".repeat(50));

        let receipt = finish_execution(
            &mut conn,
            &exec_id,
            "Succeeded",
            Some(&unicode_output),
            None,
        )
        .unwrap();

        let (stored, truncated): (String, i64) = conn
            .query_row(
                "SELECT stdout_preview, stdout_truncated FROM receipts WHERE id = ?1",
                rusqlite::params![receipt],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(stored.len() <= MAX_PREVIEW_FOR_TEST);
        assert_eq!(truncated, 1);
        assert!(stored.is_char_boundary(stored.len()));
    }

    const MAX_PREVIEW_FOR_TEST: usize = 1024;

    #[test]
    fn session_action_foreign_key() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");

        create_session(&conn, "s-123", "Active", Some("{}")).expect("create session");

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

        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params!["s-123"],
        )
        .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM actions WHERE id = ?1",
                rusqlite::params!["act-2"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn running_to_interrupted_on_recover() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");

        create_session(&conn, "s-rcv", "Active", None).unwrap();
        let a = Action {
            id: "act-r".to_string(),
            session_id: "s-rcv".to_string(),
            parent_id: None,
            kind: "k".to_string(),
            meta: None,
            state: "Created".to_string(),
            created_at: 1,
        };
        create_action(&conn, &a).unwrap();

        let exec_id = start_execution(&conn, "act-r").unwrap();
        let changed = recover_running(&mut conn).unwrap();
        assert!(changed >= 1);

        let status: String = conn
            .query_row(
                "SELECT status FROM executions WHERE id = ?1",
                rusqlite::params![exec_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "Interrupted");
    }
}
