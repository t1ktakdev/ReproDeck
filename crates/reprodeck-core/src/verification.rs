use crate::timeline;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct OutcomeContract {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub id: String,
    pub contract_id: String,
    pub stable_id: String,
    pub description: String,
    pub command_ref: Option<String>,
    pub expected_condition: Option<String>,
    pub required: bool,
    pub ordering: i64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum RunPhase {
    Before,
    After,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum RunStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Error,
    Interrupted,
}

impl Display for RunPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunPhase::Before => write!(f, "Before"),
            RunPhase::After => write!(f, "After"),
        }
    }
}

impl Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunStatus::Pending => write!(f, "Pending"),
            RunStatus::Running => write!(f, "Running"),
            RunStatus::Passed => write!(f, "Passed"),
            RunStatus::Failed => write!(f, "Failed"),
            RunStatus::Error => write!(f, "Error"),
            RunStatus::Interrupted => write!(f, "Interrupted"),
        }
    }
}

pub fn create_outcome_contract(
    conn: &Connection,
    session_id: &str,
    title: &str,
    description: Option<&str>,
) -> Result<OutcomeContract, rusqlite::Error> {
    let id = Uuid::new_v4().to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO outcome_contracts (id, session_id, title, description, state, version, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![id, session_id, title, description, "Draft", 1, now],
    )?;
    Ok(OutcomeContract {
        id,
        session_id: session_id.to_string(),
        title: title.to_string(),
        description: description.map(|s| s.to_string()),
        state: "Draft".to_string(),
        version: 1,
        created_at: now,
        updated_at: None,
    })
}

pub fn get_outcome_contract(
    conn: &Connection,
    id: &str,
) -> Result<Option<OutcomeContract>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT id, session_id, title, description, state, version, created_at, updated_at FROM outcome_contracts WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(r) = rows.next()? {
        let c = OutcomeContract {
            id: r.get(0)?,
            session_id: r.get(1)?,
            title: r.get(2)?,
            description: r.get(3)?,
            state: r.get(4)?,
            version: r.get(5)?,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        };
        Ok(Some(c))
    } else {
        Ok(None)
    }
}

/// Start a verification run. This creates a Timeline Action and starts an execution; it records a verification_runs row with status Running.
pub fn start_verification_run(
    conn: &Connection,
    contract_id: &str,
    phase: RunPhase,
) -> Result<String, timeline::TimelineError> {
    let run_id = Uuid::new_v4().to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // find session_id for contract to populate action
    let session_id: String = conn.query_row(
        "SELECT session_id FROM outcome_contracts WHERE id = ?1",
        rusqlite::params![contract_id],
        |r| r.get(0),
    )?;

    // create action
    let action = timeline::Action {
        id: run_id.clone(),
        session_id: session_id.clone(),
        parent_id: None,
        kind: "verification:run".to_string(),
        meta: Some(format!(
            "{{\"contract_id\":\"{}\",\"phase\":\"{}\"}}",
            contract_id, phase,
        )),
        state: "Created".to_string(),
        created_at: now,
    };
    timeline::create_action(conn, &action)?;

    // start execution
    let exec_id = timeline::start_execution(conn, &action.id)?;

    // insert verification_runs
    conn.execute(
        "INSERT INTO verification_runs (id, contract_id, phase, status, started_at, receipt_id) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![run_id, contract_id, phase.to_string(), RunStatus::Running.to_string(), now, exec_id],
    )?;

    Ok(run_id)
}

/// Finish a verification run by updating its status and recording receipt_id (receipt_id is expected to be created by timeline.finish_execution and returned by it).
pub fn finish_verification_run(
    conn: &mut Connection,
    run_id: &str,
    status: RunStatus,
    receipt_id: &str,
) -> Result<(), rusqlite::Error> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    conn.execute(
        "UPDATE verification_runs SET status = ?1, finished_at = ?2, receipt_id = ?3 WHERE id = ?4",
        rusqlite::params![status.to_string(), now, receipt_id, run_id],
    )?;
    Ok(())
}

pub fn evaluate_outcome(conn: &Connection, contract_id: &str) -> Result<String, rusqlite::Error> {
    // naive evaluation for initial implementation:
    // if any Before run exists with status Failed -> before_failed
    // if any After run exists with status Passed and before_failed -> Verified Fix
    let before_failed: Option<String> = conn.query_row(
        "SELECT id FROM verification_runs WHERE contract_id = ?1 AND phase = 'Before' AND status = 'Failed' LIMIT 1",
        rusqlite::params![contract_id],
        |r| r.get(0),
    ).optional()?;

    if before_failed.is_none() {
        return Ok("BeforePassedOrNotObserved".to_string());
    }

    let after_passed: Option<String> = conn.query_row(
        "SELECT id FROM verification_runs WHERE contract_id = ?1 AND phase = 'After' AND status = 'Passed' LIMIT 1",
        rusqlite::params![contract_id],
        |r| r.get(0),
    ).optional()?;

    if after_passed.is_some() {
        Ok("VerifiedFix".to_string())
    } else {
        Ok("NotFixed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::NamedTempFile;

    #[test]
    fn create_and_query_contract() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = init_db(path).expect("init db");

        // create minimal session required by FK
        conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["s-test","r",1,1,"Active"]).unwrap();

        let c = create_outcome_contract(&conn, "s-test", "T", Some("d")).expect("create");
        let got = get_outcome_contract(&conn, &c.id)
            .expect("get")
            .expect("found");
        assert_eq!(got.title, "T");
    }

    #[test]
    fn start_and_finish_run_lifecycle() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let mut conn = init_db(path).expect("init db");

        conn.execute("INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)", rusqlite::params!["s-r","r",1,1,"Active"]).unwrap();
        let c = create_outcome_contract(&conn, "s-r", "title", None).unwrap();

        let run_id = start_verification_run(&conn, &c.id, RunPhase::Before).expect("start");
        // there should be a verification_runs row
        let status: String = conn
            .query_row(
                "SELECT status FROM verification_runs WHERE id = ?1",
                rusqlite::params![&run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "Running");

        // simulate finishing by calling finish_verification_run
        finish_verification_run(&mut conn, &run_id, RunStatus::Passed, "receipt-x")
            .expect("finish");
        let status2: String = conn
            .query_row(
                "SELECT status FROM verification_runs WHERE id = ?1",
                rusqlite::params![&run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status2, "Passed");
    }
}
