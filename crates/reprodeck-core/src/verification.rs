use crate::timeline;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum VerificationError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Timeline(#[from] timeline::TimelineError),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("verification run not found: {0}")]
    RunNotFound(String),
    #[error("verification check not found or does not belong to contract: {0}")]
    CheckNotFound(String),
    #[error("verification run cannot finish from status {0}")]
    InvalidFinishStatus(RunStatus),
    #[error("receipt {receipt_id} does not belong to verification run {run_id}")]
    ReceiptMismatch { run_id: String, receipt_id: String },
    #[error("invalid persisted verification value for {field}: {value}")]
    InvalidPersistedState { field: &'static str, value: String },
}

type Result<T> = std::result::Result<T, VerificationError>;

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum OutcomeState {
    VerifiedFix,
    ReproductionNotProven,
    NotFixed,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationRun {
    pub id: String,
    pub contract_id: String,
    pub check_id: Option<String>,
    pub phase: RunPhase,
    pub status: RunStatus,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub receipt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationCheckSummary {
    pub check: VerificationCheck,
    pub before: Option<RunStatus>,
    pub after: Option<RunStatus>,
    pub outcome: OutcomeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeSummary {
    pub contract_id: String,
    pub overall: OutcomeState,
    pub checks: Vec<VerificationCheckSummary>,
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

impl Display for OutcomeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutcomeState::VerifiedFix => write!(f, "VerifiedFix"),
            OutcomeState::ReproductionNotProven => write!(f, "ReproductionNotProven"),
            OutcomeState::NotFixed => write!(f, "NotFixed"),
            OutcomeState::Inconclusive => write!(f, "Inconclusive"),
        }
    }
}

fn parse_run_phase(value: &str) -> Result<RunPhase> {
    match value {
        "Before" => Ok(RunPhase::Before),
        "After" => Ok(RunPhase::After),
        _ => Err(VerificationError::InvalidPersistedState {
            field: "phase",
            value: value.to_owned(),
        }),
    }
}

fn parse_run_status(value: &str) -> Result<RunStatus> {
    match value {
        "Pending" => Ok(RunStatus::Pending),
        "Running" => Ok(RunStatus::Running),
        "Passed" => Ok(RunStatus::Passed),
        "Failed" => Ok(RunStatus::Failed),
        "Error" => Ok(RunStatus::Error),
        "Interrupted" => Ok(RunStatus::Interrupted),
        _ => Err(VerificationError::InvalidPersistedState {
            field: "status",
            value: value.to_owned(),
        }),
    }
}

fn contract_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutcomeContract> {
    Ok(OutcomeContract {
        id: row.get(0)?,
        session_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        state: row.get(4)?,
        version: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn check_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationCheck> {
    Ok(VerificationCheck {
        id: row.get(0)?,
        contract_id: row.get(1)?,
        stable_id: row.get(2)?,
        description: row.get(3)?,
        command_ref: row.get(4)?,
        expected_condition: row.get(5)?,
        required: row.get::<_, i64>(6)? != 0,
        ordering: row.get(7)?,
    })
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, Option<String>, String, String, Option<i64>, Option<i64>, Option<i64>, Option<String>)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn decode_run(
    raw: (String, String, Option<String>, String, String, Option<i64>, Option<i64>, Option<i64>, Option<String>),
) -> Result<VerificationRun> {
    Ok(VerificationRun {
        id: raw.0,
        contract_id: raw.1,
        check_id: raw.2,
        phase: parse_run_phase(&raw.3)?,
        status: parse_run_status(&raw.4)?,
        started_at: raw.5,
        finished_at: raw.6,
        duration_ms: raw.7,
        receipt_id: raw.8,
    })
}

pub fn create_outcome_contract(
    conn: &Connection,
    session_id: &str,
    title: &str,
    description: Option<&str>,
) -> Result<OutcomeContract> {
    let id = Uuid::new_v4().to_string();
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO outcome_contracts (id, session_id, title, description, state, version, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![id, session_id, title, description, "Draft", 1, now],
    )?;
    Ok(OutcomeContract {
        id,
        session_id: session_id.to_string(),
        title: title.to_string(),
        description: description.map(str::to_owned),
        state: "Draft".to_string(),
        version: 1,
        created_at: now,
        updated_at: None,
    })
}

pub fn get_outcome_contract(conn: &Connection, id: &str) -> Result<Option<OutcomeContract>> {
    Ok(conn
        .query_row(
            "SELECT id, session_id, title, description, state, version, created_at, updated_at FROM outcome_contracts WHERE id = ?1",
            rusqlite::params![id],
            contract_from_row,
        )
        .optional()?)
}

pub fn list_outcome_contracts(
    conn: &Connection,
    session_id: Option<&str>,
) -> Result<Vec<OutcomeContract>> {
    let sql = match session_id {
        Some(_) => {
            "SELECT id, session_id, title, description, state, version, created_at, updated_at FROM outcome_contracts WHERE session_id = ?1 ORDER BY rowid DESC"
        }
        None => {
            "SELECT id, session_id, title, description, state, version, created_at, updated_at FROM outcome_contracts ORDER BY rowid DESC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let values = if let Some(session_id) = session_id {
        stmt.query_map(rusqlite::params![session_id], contract_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], contract_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    Ok(values)
}

pub fn add_verification_check(
    conn: &Connection,
    contract_id: &str,
    stable_id: &str,
    description: &str,
    command_ref: Option<&str>,
    expected_condition: Option<&str>,
    required: bool,
    ordering: i64,
) -> Result<VerificationCheck> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO verification_checks (id, contract_id, stable_id, description, command_ref, expected_condition, required, ordering) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![id, contract_id, stable_id, description, command_ref, expected_condition, required, ordering],
    )?;
    Ok(VerificationCheck {
        id,
        contract_id: contract_id.to_owned(),
        stable_id: stable_id.to_owned(),
        description: description.to_owned(),
        command_ref: command_ref.map(str::to_owned),
        expected_condition: expected_condition.map(str::to_owned),
        required,
        ordering,
    })
}

pub fn update_verification_check(
    conn: &Connection,
    check: &VerificationCheck,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE verification_checks SET stable_id = ?1, description = ?2, command_ref = ?3, expected_condition = ?4, required = ?5, ordering = ?6 WHERE id = ?7 AND contract_id = ?8",
        rusqlite::params![
            check.stable_id,
            check.description,
            check.command_ref,
            check.expected_condition,
            check.required,
            check.ordering,
            check.id,
            check.contract_id
        ],
    )?;
    if changed != 1 {
        return Err(VerificationError::CheckNotFound(check.id.clone()));
    }
    Ok(())
}

pub fn list_verification_checks(
    conn: &Connection,
    contract_id: &str,
) -> Result<Vec<VerificationCheck>> {
    let mut stmt = conn.prepare(
        "SELECT id, contract_id, stable_id, description, command_ref, expected_condition, required, ordering FROM verification_checks WHERE contract_id = ?1 ORDER BY ordering ASC, rowid ASC",
    )?;
    Ok(stmt
        .query_map(rusqlite::params![contract_id], check_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn start_run(
    conn: &mut Connection,
    contract_id: &str,
    check_id: Option<&str>,
    phase: RunPhase,
) -> Result<String> {
    let tx = conn.transaction()?;

    if let Some(check_id) = check_id {
        let exists: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM verification_checks WHERE id = ?1 AND contract_id = ?2",
                rusqlite::params![check_id, contract_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(VerificationError::CheckNotFound(check_id.to_owned()));
        }
    }

    let run_id = Uuid::new_v4().to_string();
    let now = unix_time_secs()?;
    let session_id: String = tx.query_row(
        "SELECT session_id FROM outcome_contracts WHERE id = ?1",
        rusqlite::params![contract_id],
        |row| row.get(0),
    )?;

    let meta = serde_json::to_string(&serde_json::json!({
        "contract_id": contract_id,
        "check_id": check_id,
        "phase": phase.to_string(),
    }))?;
    let action = timeline::Action {
        id: run_id.clone(),
        session_id,
        parent_id: None,
        kind: "verification:run".to_string(),
        meta: Some(meta),
        state: "Running".to_string(),
        created_at: now,
    };
    timeline::create_action(&tx, &action)?;
    timeline::start_execution(&tx, &action.id)?;

    tx.execute(
        "INSERT INTO verification_runs (id, contract_id, check_id, phase, status, started_at, receipt_id) VALUES (?1,?2,?3,?4,?5,?6,NULL)",
        rusqlite::params![run_id, contract_id, check_id, phase.to_string(), RunStatus::Running.to_string(), now],
    )?;
    tx.commit()?;
    Ok(run_id)
}

pub fn start_verification_run(
    conn: &mut Connection,
    contract_id: &str,
    phase: RunPhase,
) -> Result<String> {
    start_run(conn, contract_id, None, phase)
}

pub fn start_verification_check_run(
    conn: &mut Connection,
    contract_id: &str,
    check_id: &str,
    phase: RunPhase,
) -> Result<String> {
    start_run(conn, contract_id, Some(check_id), phase)
}

pub fn get_verification_run(
    conn: &Connection,
    run_id: &str,
) -> Result<Option<VerificationRun>> {
    let raw = conn
        .query_row(
            "SELECT id, contract_id, check_id, phase, status, started_at, finished_at, duration_ms, receipt_id FROM verification_runs WHERE id = ?1",
            rusqlite::params![run_id],
            run_from_row,
        )
        .optional()?;
    raw.map(decode_run).transpose()
}

pub fn list_verification_runs(
    conn: &Connection,
    contract_id: &str,
) -> Result<Vec<VerificationRun>> {
    let mut stmt = conn.prepare(
        "SELECT id, contract_id, check_id, phase, status, started_at, finished_at, duration_ms, receipt_id FROM verification_runs WHERE contract_id = ?1 ORDER BY rowid DESC",
    )?;
    let raws = stmt
        .query_map(rusqlite::params![contract_id], run_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    raws.into_iter().map(decode_run).collect()
}

fn execution_id_for_run(conn: &Connection, run_id: &str) -> Result<String> {
    conn.query_row(
        "SELECT id FROM executions WHERE action_id = ?1 ORDER BY created_seq DESC LIMIT 1",
        rusqlite::params![run_id],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| VerificationError::RunNotFound(run_id.to_owned()))
}

fn validate_finish_status(status: RunStatus) -> Result<()> {
    match status {
        RunStatus::Passed | RunStatus::Failed | RunStatus::Error | RunStatus::Interrupted => Ok(()),
        RunStatus::Pending | RunStatus::Running => Err(VerificationError::InvalidFinishStatus(status)),
    }
}

fn timeline_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Passed => "Succeeded",
        RunStatus::Failed | RunStatus::Error => "Failed",
        RunStatus::Interrupted => "Interrupted",
        RunStatus::Pending => "Pending",
        RunStatus::Running => "Running",
    }
}

fn update_finished_run(
    conn: &Connection,
    run_id: &str,
    status: RunStatus,
    receipt_id: &str,
    now: i64,
) -> Result<()> {
    let started_at: i64 = conn
        .query_row(
            "SELECT started_at FROM verification_runs WHERE id = ?1 AND status = 'Running'",
            rusqlite::params![run_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| VerificationError::RunNotFound(run_id.to_owned()))?;
    let duration_ms = now.saturating_sub(started_at).saturating_mul(1000);
    let changed = conn.execute(
        "UPDATE verification_runs SET status = ?1, finished_at = ?2, duration_ms = ?3, receipt_id = ?4 WHERE id = ?5 AND status = 'Running'",
        rusqlite::params![status.to_string(), now, duration_ms, receipt_id, run_id],
    )?;
    if changed != 1 {
        return Err(VerificationError::RunNotFound(run_id.to_owned()));
    }
    conn.execute(
        "UPDATE actions SET state = ?1 WHERE id = ?2",
        rusqlite::params![status.to_string(), run_id],
    )?;
    Ok(())
}

pub fn finish_verification_run_with_output(
    conn: &mut Connection,
    run_id: &str,
    status: RunStatus,
    stdout_preview: Option<&str>,
    stderr_preview: Option<&str>,
) -> Result<String> {
    validate_finish_status(status)?;
    let tx = conn.transaction()?;
    let execution_id = execution_id_for_run(&tx, run_id)?;
    let receipt_id = timeline::finish_execution_in_transaction(
        &tx,
        &execution_id,
        timeline_status(status),
        stdout_preview,
        stderr_preview,
    )?;
    update_finished_run(&tx, run_id, status, &receipt_id, unix_time_secs()?)?;
    tx.commit()?;
    Ok(receipt_id)
}

pub fn finish_verification_run(
    conn: &mut Connection,
    run_id: &str,
    status: RunStatus,
    receipt_id: &str,
) -> Result<()> {
    validate_finish_status(status)?;
    let tx = conn.transaction()?;
    let execution_id = execution_id_for_run(&tx, run_id)?;
    let matching_receipt: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM receipts WHERE id = ?1 AND execution_id = ?2",
            rusqlite::params![receipt_id, execution_id],
            |row| row.get(0),
        )
        .optional()?;
    if matching_receipt.is_none() {
        return Err(VerificationError::ReceiptMismatch {
            run_id: run_id.to_owned(),
            receipt_id: receipt_id.to_owned(),
        });
    }
    update_finished_run(&tx, run_id, status, receipt_id, unix_time_secs()?)?;
    tx.commit()?;
    Ok(())
}

pub fn interrupt_running_verifications(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE executions SET status = 'Interrupted', finished_at = COALESCE(finished_at, started_at), duration_ms = COALESCE(duration_ms, 0) WHERE status = 'Running' AND action_id IN (SELECT id FROM verification_runs WHERE status = 'Running')",
        [],
    )?;
    tx.execute(
        "UPDATE actions SET state = 'Interrupted' WHERE id IN (SELECT id FROM verification_runs WHERE status = 'Running')",
        [],
    )?;
    let changed = tx.execute(
        "UPDATE verification_runs SET status = 'Interrupted', finished_at = COALESCE(finished_at, started_at), duration_ms = COALESCE(duration_ms, 0) WHERE status = 'Running'",
        [],
    )?;
    tx.commit()?;
    Ok(changed)
}

pub fn recover_running_verifications(conn: &mut Connection) -> Result<usize> {
    interrupt_running_verifications(conn)
}

fn latest_status(
    conn: &Connection,
    contract_id: &str,
    check_id: &str,
    phase: RunPhase,
) -> Result<Option<RunStatus>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT status FROM verification_runs WHERE contract_id = ?1 AND check_id = ?2 AND phase = ?3 ORDER BY rowid DESC LIMIT 1",
            rusqlite::params![contract_id, check_id, phase.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    value.map(|value| parse_run_status(&value)).transpose()
}

fn evaluate_check(before: Option<RunStatus>, after: Option<RunStatus>) -> OutcomeState {
    match before {
        Some(RunStatus::Passed) => OutcomeState::ReproductionNotProven,
        Some(RunStatus::Failed) => match after {
            Some(RunStatus::Passed) => OutcomeState::VerifiedFix,
            Some(RunStatus::Failed) => OutcomeState::NotFixed,
            _ => OutcomeState::Inconclusive,
        },
        Some(RunStatus::Pending | RunStatus::Running | RunStatus::Error | RunStatus::Interrupted)
        | None => OutcomeState::Inconclusive,
    }
}

pub fn get_outcome_summary(conn: &Connection, contract_id: &str) -> Result<OutcomeSummary> {
    let checks = list_verification_checks(conn, contract_id)?;
    let mut summaries = Vec::with_capacity(checks.len());
    for check in checks {
        let before = latest_status(conn, contract_id, &check.id, RunPhase::Before)?;
        let after = latest_status(conn, contract_id, &check.id, RunPhase::After)?;
        let outcome = evaluate_check(before, after);
        summaries.push(VerificationCheckSummary {
            check,
            before,
            after,
            outcome,
        });
    }

    let required: Vec<&VerificationCheckSummary> =
        summaries.iter().filter(|item| item.check.required).collect();
    let overall = if required.is_empty() {
        OutcomeState::Inconclusive
    } else if required
        .iter()
        .any(|item| item.outcome == OutcomeState::NotFixed)
    {
        OutcomeState::NotFixed
    } else if required
        .iter()
        .any(|item| item.outcome == OutcomeState::ReproductionNotProven)
    {
        OutcomeState::ReproductionNotProven
    } else if required
        .iter()
        .any(|item| item.outcome == OutcomeState::Inconclusive)
    {
        OutcomeState::Inconclusive
    } else {
        OutcomeState::VerifiedFix
    };

    Ok(OutcomeSummary {
        contract_id: contract_id.to_owned(),
        overall,
        checks: summaries,
    })
}

pub fn evaluate_outcome_state(conn: &Connection, contract_id: &str) -> Result<OutcomeState> {
    Ok(get_outcome_summary(conn, contract_id)?.overall)
}

pub fn evaluate_outcome(conn: &Connection, contract_id: &str) -> Result<String> {
    Ok(evaluate_outcome_state(conn, contract_id)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::NamedTempFile;

    fn setup() -> (NamedTempFile, Connection, OutcomeContract, VerificationCheck) {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        conn.execute(
            "INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params!["s-test", "r", 1, 1, "Active"],
        )
        .unwrap();
        let contract = create_outcome_contract(&conn, "s-test", "T", Some("d")).unwrap();
        let check = add_verification_check(
            &conn,
            &contract.id,
            "check-1",
            "Regression test",
            Some("cargo test"),
            Some("exit 0"),
            true,
            0,
        )
        .unwrap();
        (tmp, conn, contract, check)
    }

    fn complete(
        conn: &mut Connection,
        contract: &OutcomeContract,
        check: &VerificationCheck,
        phase: RunPhase,
        status: RunStatus,
    ) -> String {
        let run = start_verification_check_run(conn, &contract.id, &check.id, phase).unwrap();
        finish_verification_run_with_output(conn, &run, status, Some("verification output"), None)
            .unwrap();
        run
    }

    #[test]
    fn create_query_and_list_contract() {
        let (_tmp, conn, contract, _check) = setup();
        let got = get_outcome_contract(&conn, &contract.id).unwrap().unwrap();
        assert_eq!(got.title, "T");
        let contracts = list_outcome_contracts(&conn, Some("s-test")).unwrap();
        assert_eq!(contracts, vec![contract]);
        assert!(list_outcome_contracts(&conn, Some("other")).unwrap().is_empty());
    }

    #[test]
    fn check_update_and_ordering_are_deterministic() {
        let (_tmp, conn, contract, mut check) = setup();
        let second = add_verification_check(
            &conn,
            &contract.id,
            "check-2",
            "Second",
            None,
            None,
            false,
            10,
        )
        .unwrap();
        check.description = "Updated regression".to_string();
        check.ordering = 20;
        update_verification_check(&conn, &check).unwrap();
        let checks = list_verification_checks(&conn, &contract.id).unwrap();
        assert_eq!(checks[0].id, second.id);
        assert_eq!(checks[1].description, "Updated regression");
    }

    #[test]
    fn start_and_finish_run_lifecycle_uses_real_receipt() {
        let (_tmp, mut conn, contract, check) = setup();
        let run_id = start_verification_check_run(
            &mut conn,
            &contract.id,
            &check.id,
            RunPhase::Before,
        )
        .unwrap();
        let running = get_verification_run(&conn, &run_id).unwrap().unwrap();
        assert_eq!(running.status, RunStatus::Running);
        assert!(running.receipt_id.is_none());

        let receipt = finish_verification_run_with_output(
            &mut conn,
            &run_id,
            RunStatus::Failed,
            Some("failure"),
            None,
        )
        .unwrap();
        let finished = get_verification_run(&conn, &run_id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Failed);
        assert_eq!(finished.receipt_id.as_deref(), Some(receipt.as_str()));
        assert_eq!(
            timeline::get_execution(
                &conn,
                &conn.query_row(
                    "SELECT id FROM executions WHERE action_id = ?1",
                    rusqlite::params![&run_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            )
            .unwrap()
            .unwrap()
            .status,
            "Failed"
        );
    }

    #[test]
    fn before_failed_after_passed_is_verified_fix() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(&mut conn, &contract, &check, RunPhase::Before, RunStatus::Failed);
        complete(&mut conn, &contract, &check, RunPhase::After, RunStatus::Passed);
        let summary = get_outcome_summary(&conn, &contract.id).unwrap();
        assert_eq!(summary.overall, OutcomeState::VerifiedFix);
        assert_eq!(summary.checks[0].before, Some(RunStatus::Failed));
        assert_eq!(summary.checks[0].after, Some(RunStatus::Passed));
    }

    #[test]
    fn before_passed_means_reproduction_not_proven() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(&mut conn, &contract, &check, RunPhase::Before, RunStatus::Passed);
        complete(&mut conn, &contract, &check, RunPhase::After, RunStatus::Passed);
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::ReproductionNotProven
        );
    }

    #[test]
    fn before_failed_after_failed_is_not_fixed() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(&mut conn, &contract, &check, RunPhase::Before, RunStatus::Failed);
        complete(&mut conn, &contract, &check, RunPhase::After, RunStatus::Failed);
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::NotFixed
        );
    }

    #[test]
    fn error_or_interruption_is_inconclusive() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(&mut conn, &contract, &check, RunPhase::Before, RunStatus::Error);
        complete(&mut conn, &contract, &check, RunPhase::After, RunStatus::Passed);
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::Inconclusive
        );
    }

    #[test]
    fn optional_failure_does_not_block_required_verified_fix() {
        let (_tmp, mut conn, contract, required) = setup();
        let optional = add_verification_check(
            &conn,
            &contract.id,
            "optional",
            "Optional diagnostic",
            None,
            None,
            false,
            1,
        )
        .unwrap();
        complete(&mut conn, &contract, &required, RunPhase::Before, RunStatus::Failed);
        complete(&mut conn, &contract, &required, RunPhase::After, RunStatus::Passed);
        complete(&mut conn, &contract, &optional, RunPhase::Before, RunStatus::Failed);
        complete(&mut conn, &contract, &optional, RunPhase::After, RunStatus::Failed);
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::VerifiedFix
        );
    }

    #[test]
    fn different_checks_cannot_prove_each_other() {
        let (_tmp, mut conn, contract, check_a) = setup();
        let check_b = add_verification_check(
            &conn,
            &contract.id,
            "check-2",
            "Second regression check",
            None,
            None,
            true,
            1,
        )
        .unwrap();
        complete(&mut conn, &contract, &check_a, RunPhase::Before, RunStatus::Failed);
        complete(&mut conn, &contract, &check_b, RunPhase::After, RunStatus::Passed);
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::Inconclusive
        );
    }

    #[test]
    fn latest_run_wins_deterministically() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(&mut conn, &contract, &check, RunPhase::Before, RunStatus::Failed);
        complete(&mut conn, &contract, &check, RunPhase::After, RunStatus::Failed);
        complete(&mut conn, &contract, &check, RunPhase::After, RunStatus::Passed);
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::VerifiedFix
        );
        let runs = list_verification_runs(&conn, &contract.id).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].status, RunStatus::Passed);
    }

    #[test]
    fn recovery_interrupts_only_verification_timeline_executions() {
        let (_tmp, mut conn, contract, check) = setup();
        let run = start_verification_check_run(
            &mut conn,
            &contract.id,
            &check.id,
            RunPhase::Before,
        )
        .unwrap();

        let unrelated_action = timeline::Action {
            id: "unrelated-action".to_string(),
            session_id: "s-test".to_string(),
            parent_id: None,
            kind: "command".to_string(),
            meta: None,
            state: "Running".to_string(),
            created_at: 1,
        };
        timeline::create_action(&conn, &unrelated_action).unwrap();
        let unrelated_execution = timeline::start_execution(&conn, &unrelated_action.id).unwrap();

        assert_eq!(interrupt_running_verifications(&mut conn).unwrap(), 1);
        assert_eq!(
            get_verification_run(&conn, &run).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
        assert_eq!(
            timeline::get_execution(&conn, &unrelated_execution)
                .unwrap()
                .unwrap()
                .status,
            "Running"
        );
    }
}
