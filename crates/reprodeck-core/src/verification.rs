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
}

type Result<T> = std::result::Result<T, VerificationError>;

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

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

#[derive(Debug, Serialize, Deserialize, Clone)]
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

fn parse_run_status(value: &str) -> Option<RunStatus> {
    match value {
        "Pending" => Some(RunStatus::Pending),
        "Running" => Some(RunStatus::Running),
        "Passed" => Some(RunStatus::Passed),
        "Failed" => Some(RunStatus::Failed),
        "Error" => Some(RunStatus::Error),
        "Interrupted" => Some(RunStatus::Interrupted),
        _ => None,
    }
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
    let mut stmt = conn.prepare("SELECT id, session_id, title, description, state, version, created_at, updated_at FROM outcome_contracts WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(r) = rows.next()? {
        Ok(Some(OutcomeContract {
            id: r.get(0)?,
            session_id: r.get(1)?,
            title: r.get(2)?,
            description: r.get(3)?,
            state: r.get(4)?,
            version: r.get(5)?,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        }))
    } else {
        Ok(None)
    }
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

pub fn list_verification_checks(
    conn: &Connection,
    contract_id: &str,
) -> Result<Vec<VerificationCheck>> {
    let mut stmt = conn.prepare(
        "SELECT id, contract_id, stable_id, description, command_ref, expected_condition, required, ordering FROM verification_checks WHERE contract_id = ?1 ORDER BY ordering, id",
    )?;
    let rows = stmt.query_map(rusqlite::params![contract_id], |r| {
        Ok(VerificationCheck {
            id: r.get(0)?,
            contract_id: r.get(1)?,
            stable_id: r.get(2)?,
            description: r.get(3)?,
            command_ref: r.get(4)?,
            expected_condition: r.get(5)?,
            required: r.get(6)?,
            ordering: r.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(VerificationError::Db)
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
                |r| r.get(0),
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
        |r| r.get(0),
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
        state: "Created".to_string(),
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

/// Start a contract-level verification run. Prefer `start_verification_check_run`
/// for contracts that contain explicit checks.
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

fn execution_id_for_run(conn: &Connection, run_id: &str) -> Result<String> {
    conn.query_row(
        "SELECT id FROM executions WHERE action_id = ?1 ORDER BY created_seq DESC LIMIT 1",
        rusqlite::params![run_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| VerificationError::RunNotFound(run_id.to_owned()))
}

fn validate_finish_status(status: RunStatus) -> Result<()> {
    match status {
        RunStatus::Passed | RunStatus::Failed | RunStatus::Error | RunStatus::Interrupted => Ok(()),
        RunStatus::Pending | RunStatus::Running => {
            Err(VerificationError::InvalidFinishStatus(status))
        }
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
            |r| r.get(0),
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
    Ok(())
}

/// Finish a verification run and its underlying Timeline execution in one DB
/// transaction. The returned receipt is the actual Timeline receipt.
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

/// Attach an already-created receipt to a verification run. The receipt must
/// belong to the Timeline execution created for this run.
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
            |r| r.get(0),
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

pub fn recover_running_verifications(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE verification_runs SET status = 'Interrupted', finished_at = COALESCE(finished_at, started_at) WHERE status = 'Running'",
        [],
    )?;
    tx.execute(
        "UPDATE executions SET status = 'Interrupted' WHERE status = 'Running' AND finished_at IS NULL",
        [],
    )?;
    tx.commit()?;
    Ok(changed)
}

fn latest_status(
    conn: &Connection,
    contract_id: &str,
    check_id: &str,
    phase: RunPhase,
) -> Result<Option<RunStatus>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT status FROM verification_runs WHERE contract_id = ?1 AND check_id = ?2 AND phase = ?3 ORDER BY started_at DESC, id DESC LIMIT 1",
            rusqlite::params![contract_id, check_id, phase.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.as_deref().and_then(parse_run_status))
}

fn evaluate_check(before: Option<RunStatus>, after: Option<RunStatus>) -> OutcomeState {
    match before {
        Some(RunStatus::Passed) => OutcomeState::ReproductionNotProven,
        Some(RunStatus::Failed) => match after {
            Some(RunStatus::Passed) => OutcomeState::VerifiedFix,
            Some(RunStatus::Failed) => OutcomeState::NotFixed,
            _ => OutcomeState::Inconclusive,
        },
        Some(
            RunStatus::Pending | RunStatus::Running | RunStatus::Error | RunStatus::Interrupted,
        )
        | None => OutcomeState::Inconclusive,
    }
}

pub fn evaluate_outcome_state(conn: &Connection, contract_id: &str) -> Result<OutcomeState> {
    let checks = list_verification_checks(conn, contract_id)?;
    let required: Vec<_> = checks.into_iter().filter(|check| check.required).collect();

    if required.is_empty() {
        return Ok(OutcomeState::Inconclusive);
    }

    let mut saw_reproduction_not_proven = false;
    let mut saw_inconclusive = false;
    for check in required {
        let before = latest_status(conn, contract_id, &check.id, RunPhase::Before)?;
        let after = latest_status(conn, contract_id, &check.id, RunPhase::After)?;
        match evaluate_check(before, after) {
            OutcomeState::NotFixed => return Ok(OutcomeState::NotFixed),
            OutcomeState::ReproductionNotProven => saw_reproduction_not_proven = true,
            OutcomeState::Inconclusive => saw_inconclusive = true,
            OutcomeState::VerifiedFix => {}
        }
    }

    if saw_reproduction_not_proven {
        Ok(OutcomeState::ReproductionNotProven)
    } else if saw_inconclusive {
        Ok(OutcomeState::Inconclusive)
    } else {
        Ok(OutcomeState::VerifiedFix)
    }
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
        let conn = init_db(tmp.path()).expect("init db");
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
    fn create_and_query_contract() {
        let (_tmp, conn, contract, _check) = setup();
        let got = get_outcome_contract(&conn, &contract.id)
            .expect("get")
            .expect("found");
        assert_eq!(got.title, "T");
    }

    #[test]
    fn start_and_finish_run_lifecycle_uses_real_receipt() {
        let (_tmp, mut conn, contract, check) = setup();
        let run_id =
            start_verification_check_run(&mut conn, &contract.id, &check.id, RunPhase::Before)
                .expect("start");

        let (status, receipt_at_start): (String, Option<String>) = conn
            .query_row(
                "SELECT status, receipt_id FROM verification_runs WHERE id = ?1",
                rusqlite::params![&run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "Running");
        assert!(receipt_at_start.is_none());

        let receipt = finish_verification_run_with_output(
            &mut conn,
            &run_id,
            RunStatus::Failed,
            Some("failure"),
            None,
        )
        .unwrap();

        let (run_status, stored_receipt): (String, Option<String>) = conn
            .query_row(
                "SELECT status, receipt_id FROM verification_runs WHERE id = ?1",
                rusqlite::params![&run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(run_status, "Failed");
        assert_eq!(stored_receipt.as_deref(), Some(receipt.as_str()));

        let execution_status: String = conn
            .query_row(
                "SELECT status FROM executions WHERE action_id = ?1",
                rusqlite::params![&run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(execution_status, "Failed");
    }

    #[test]
    fn before_failed_after_passed_is_verified_fix() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::Before,
            RunStatus::Failed,
        );
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::After,
            RunStatus::Passed,
        );
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::VerifiedFix
        );
    }

    #[test]
    fn before_passed_means_reproduction_not_proven() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::Before,
            RunStatus::Passed,
        );
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::After,
            RunStatus::Passed,
        );
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::ReproductionNotProven
        );
    }

    #[test]
    fn before_failed_after_failed_is_not_fixed() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::Before,
            RunStatus::Failed,
        );
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::After,
            RunStatus::Failed,
        );
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::NotFixed
        );
    }

    #[test]
    fn error_or_interruption_is_inconclusive() {
        let (_tmp, mut conn, contract, check) = setup();
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::Before,
            RunStatus::Error,
        );
        complete(
            &mut conn,
            &contract,
            &check,
            RunPhase::After,
            RunStatus::Passed,
        );
        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::Inconclusive
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

        complete(
            &mut conn,
            &contract,
            &check_a,
            RunPhase::Before,
            RunStatus::Failed,
        );
        complete(
            &mut conn,
            &contract,
            &check_b,
            RunPhase::After,
            RunStatus::Passed,
        );

        assert_eq!(
            evaluate_outcome_state(&conn, &contract.id).unwrap(),
            OutcomeState::Inconclusive
        );
    }

    #[test]
    fn recovery_interrupts_verification_and_timeline_execution() {
        let (_tmp, mut conn, contract, check) = setup();
        let run =
            start_verification_check_run(&mut conn, &contract.id, &check.id, RunPhase::Before)
                .unwrap();
        assert_eq!(recover_running_verifications(&mut conn).unwrap(), 1);

        let run_status: String = conn
            .query_row(
                "SELECT status FROM verification_runs WHERE id = ?1",
                rusqlite::params![&run],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(run_status, "Interrupted");

        let execution_status: String = conn
            .query_row(
                "SELECT status FROM executions WHERE action_id = ?1",
                rusqlite::params![&run],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(execution_status, "Interrupted");
    }
}
