use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
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
}

pub fn create_session(
    conn: &Connection,
    public_id: &str,
    state: &str,
    meta: Option<&str>,
) -> Result<(), TimelineError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO executions (id, action_id, status, started_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![exec_id, action_id, "Running", now],
    )?;
    Ok(exec_id)
}

/// finish_execution inserts receipt and optional artifact metadata atomically.
pub fn finish_execution(
    conn: &mut Connection,
    execution_id: &str,
    status: &str,
    stdout_preview: Option<&str>,
    stderr_preview: Option<&str>,
) -> Result<String, TimelineError> {
    let tx = conn.transaction()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // update execution
    tx.execute(
        "UPDATE executions SET status = ?1, finished_at = ?2 WHERE id = ?3",
        rusqlite::params![status, now, execution_id],
    )?;

    // insert receipt
    let receipt_id = Uuid::new_v4().to_string();
    // sanitize then apply preview bounding
    fn sanitize_preview(input: &str) -> String {
        // redact bearer tokens
        let bearer = Regex::new(r"(?i)bearer\s+[A-Za-z0-9\-\._~\+\/]+=*").unwrap();
        let mut s = bearer.replace_all(input, "[REDACTED]").into_owned();
        // redact common key=val patterns for token/password
        let kv = Regex::new(r"(?i)(password|token|secret)\s*[=:]\s*[^\s,;]+").unwrap();
        s = kv.replace_all(&s, "$1=[REDACTED]").into_owned();
        // redact JWT-like tokens (three dot-separated base64url segments)
        let jwt = Regex::new(r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap();
        s = jwt.replace_all(&s, "[REDACTED_JWT]").into_owned();
        // redact AWS-style access keys (AKIA...)
        let aws = Regex::new(r"AKIA[0-9A-Z]{16}").unwrap();
        s = aws.replace_all(&s, "[REDACTED_AWS_KEY]").into_owned();
        // redact long hex or base64-like tokens (heuristic)
        let hex64 = Regex::new(r"\b[0-9a-fA-F]{40,64}\b").unwrap();
        s = hex64.replace_all(&s, "[REDACTED_TOKEN]").into_owned();
        let long_token = Regex::new(r"\b[A-Za-z0-9_\-]{40,}\b").unwrap();
        s = long_token.replace_all(&s, "[REDACTED_TOKEN]").into_owned();
        s
    }

    // apply preview bounding
    const MAX_PREVIEW: usize = 1024;
    let (sp_owned, spt) = match stdout_preview {
        Some(s) => {
            let san = sanitize_preview(s);
            if san.len() > MAX_PREVIEW {
                (Some(san[..MAX_PREVIEW].to_string()), 1)
            } else {
                (Some(san), 0)
            }
        }
        None => (None, 0),
    };
    let (ep_owned, ept) = match stderr_preview {
        Some(s) => {
            let san = sanitize_preview(s);
            if san.len() > MAX_PREVIEW {
                (Some(san[..MAX_PREVIEW].to_string()), 1)
            } else {
                (Some(san), 0)
            }
        }
        None => (None, 0),
    };

    tx.execute("INSERT INTO receipts (id, execution_id, summary, stdout_preview, stderr_preview, stdout_truncated, stderr_truncated, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![receipt_id, execution_id, "", sp_owned, ep_owned, spt, ept, now])?;

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

        // session must exist per FK; create minimal session
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
        // insert session with duplicate id
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

        // insert multiple actions with same created_at
        for i in 0..5 {
            let id = format!("a-{}", i);
            conn.execute("INSERT INTO actions(id, session_id, kind, state, created_at) VALUES (?1,?2,?3,?4,?5)", rusqlite::params![id, "s-pag", "k", "Created", 1000]).unwrap();
        }

        // pagination by created_seq stable ordering
        let mut stmt = conn
            .prepare("SELECT id FROM actions WHERE session_id = ?1 ORDER BY created_seq LIMIT 2")
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params!["s-pag"], |r| r.get::<_, String>(0))
            .unwrap();
        let ids: Vec<String> = rows.map(|r| r.unwrap()).collect();
        assert_eq!(ids.len(), 2);
        // next page
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
        // include a bearer token in stdout
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
        // JWT without Bearer
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
    fn session_action_foreign_key() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = crate::db::init_db(path).expect("init db");

        // create session
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

        // deleting session should cascade to actions
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

        // prepare session & action
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

        // start execution
        let exec_id = start_execution(&conn, "act-r").unwrap();

        // simulate restart by calling recover_running
        let changed = recover_running(&mut conn).unwrap();
        assert!(changed >= 1);

        // check status
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
