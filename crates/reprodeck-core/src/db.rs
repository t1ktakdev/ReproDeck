use rusqlite::{params, Connection};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unknown or too-new schema version: {0}")]
    UnknownSchemaVersion(i64),
    #[error("migration failed: {0}")]
    MigrationFailed(String),
}

type MResult<T> = std::result::Result<T, MigrationError>;

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "1",
        "CREATE TABLE IF NOT EXISTS reprodeck_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS repositories (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL,
            head_commit TEXT
        );",
    ),
    (
        "2",
        "CREATE TABLE IF NOT EXISTS sessions (
            created_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            repo_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER,
            state TEXT NOT NULL DEFAULT 'Active',
            meta TEXT
        );

        CREATE TABLE IF NOT EXISTS shadow_workspaces (
            id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            base_commit TEXT NOT NULL,
            branch TEXT NOT NULL,
            worktree_path TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS command_executions (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            command TEXT NOT NULL,
            exit_code INTEGER,
            stdout BLOB,
            stderr BLOB,
            started_at INTEGER NOT NULL,
            finished_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS timeline_events (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            ts INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT
        );

        CREATE TABLE IF NOT EXISTS evidence (
            id TEXT PRIMARY KEY,
            timeline_id TEXT NOT NULL,
            path TEXT NOT NULL,
            checksum TEXT
        );

        CREATE TABLE IF NOT EXISTS outcome_criteria (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            spec TEXT NOT NULL
        );
        INSERT OR IGNORE INTO reprodeck_meta (key, value) VALUES ('schema_version', '1');",
    ),
    (
        "3",
        "CREATE INDEX IF NOT EXISTS idx_sessions_id ON sessions(id);

        CREATE TABLE IF NOT EXISTS actions (
            created_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            session_id TEXT NOT NULL,
            parent_id TEXT,
            kind TEXT NOT NULL,
            meta TEXT,
            state TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE ON UPDATE NO ACTION
        );

        CREATE INDEX IF NOT EXISTS idx_actions_session ON actions(session_id);

        CREATE TABLE IF NOT EXISTS executions (
            created_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            action_id TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            duration_ms INTEGER,
            FOREIGN KEY(action_id) REFERENCES actions(id) ON DELETE CASCADE ON UPDATE NO ACTION
        );

        CREATE INDEX IF NOT EXISTS idx_executions_action ON executions(action_id);

        CREATE TABLE IF NOT EXISTS receipts (
            created_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            execution_id TEXT NOT NULL,
            summary TEXT,
            stdout_preview TEXT,
            stderr_preview TEXT,
            stdout_truncated INTEGER DEFAULT 0,
            stderr_truncated INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(execution_id) REFERENCES executions(id) ON DELETE CASCADE ON UPDATE NO ACTION
        );

        CREATE INDEX IF NOT EXISTS idx_receipts_execution ON receipts(execution_id);

        CREATE TABLE IF NOT EXISTS artifacts (
            created_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            receipt_id TEXT NOT NULL,
            store_key TEXT NOT NULL,
            checksum TEXT NOT NULL,
            size INTEGER NOT NULL,
            media_type TEXT,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(receipt_id) REFERENCES receipts(id) ON DELETE CASCADE ON UPDATE NO ACTION
        );

        CREATE INDEX IF NOT EXISTS idx_artifacts_receipt ON artifacts(receipt_id);",
    ),
    (
        "4",
        "CREATE TABLE IF NOT EXISTS outcome_contracts (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            state TEXT NOT NULL DEFAULT 'Draft',
            version INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS verification_checks (
            id TEXT PRIMARY KEY,
            contract_id TEXT NOT NULL,
            stable_id TEXT NOT NULL,
            description TEXT NOT NULL,
            command_ref TEXT,
            expected_condition TEXT,
            required INTEGER DEFAULT 1,
            ordering INTEGER DEFAULT 0,
            FOREIGN KEY(contract_id) REFERENCES outcome_contracts(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_checks_contract ON verification_checks(contract_id);

        CREATE TABLE IF NOT EXISTS verification_runs (
            id TEXT PRIMARY KEY,
            contract_id TEXT NOT NULL,
            check_id TEXT,
            phase TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at INTEGER,
            finished_at INTEGER,
            duration_ms INTEGER,
            receipt_id TEXT,
            FOREIGN KEY(contract_id) REFERENCES outcome_contracts(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_runs_contract ON verification_runs(contract_id);

        CREATE TABLE IF NOT EXISTS outcome_results (
            id TEXT PRIMARY KEY,
            contract_id TEXT NOT NULL,
            overall_state TEXT NOT NULL,
            before_summary TEXT,
            after_summary TEXT,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(contract_id) REFERENCES outcome_contracts(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS evidence_links (
            id TEXT PRIMARY KEY,
            evidence_id TEXT NOT NULL,
            run_id TEXT,
            role TEXT NOT NULL,
            FOREIGN KEY(evidence_id) REFERENCES evidence(id) ON DELETE CASCADE,
            FOREIGN KEY(run_id) REFERENCES verification_runs(id) ON DELETE CASCADE
        );",
    ),
    (
        "5",
        "-- Tighten outcome/evidence integrity without destructively rebuilding v4 tables.
        CREATE UNIQUE INDEX IF NOT EXISTS idx_checks_contract_stable
            ON verification_checks(contract_id, stable_id);
        CREATE INDEX IF NOT EXISTS idx_runs_check ON verification_runs(check_id);
        CREATE INDEX IF NOT EXISTS idx_runs_receipt ON verification_runs(receipt_id);

        CREATE TABLE IF NOT EXISTS artifact_links (
            id TEXT PRIMARY KEY,
            artifact_id TEXT NOT NULL,
            run_id TEXT,
            role TEXT NOT NULL CHECK(role IN ('Before','After','Verification','Diagnostic','Attachment')),
            created_at INTEGER NOT NULL,
            FOREIGN KEY(artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE,
            FOREIGN KEY(run_id) REFERENCES verification_runs(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_artifact_links_artifact ON artifact_links(artifact_id);
        CREATE INDEX IF NOT EXISTS idx_artifact_links_run ON artifact_links(run_id);

        CREATE TRIGGER IF NOT EXISTS trg_outcome_contract_session_insert
        BEFORE INSERT ON outcome_contracts
        WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.session_id)
        BEGIN
            SELECT RAISE(ABORT, 'outcome contract session does not exist');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_outcome_contract_session_update
        BEFORE UPDATE OF session_id ON outcome_contracts
        WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.session_id)
        BEGIN
            SELECT RAISE(ABORT, 'outcome contract session does not exist');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_sessions_delete_outcome_contracts
        AFTER DELETE ON sessions
        BEGIN
            DELETE FROM outcome_contracts WHERE session_id = OLD.id;
        END;

        CREATE TRIGGER IF NOT EXISTS trg_verification_run_check_insert
        BEFORE INSERT ON verification_runs
        WHEN NEW.check_id IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM verification_checks
            WHERE id = NEW.check_id AND contract_id = NEW.contract_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'verification check does not belong to contract');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_verification_run_check_update
        BEFORE UPDATE OF check_id, contract_id ON verification_runs
        WHEN NEW.check_id IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM verification_checks
            WHERE id = NEW.check_id AND contract_id = NEW.contract_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'verification check does not belong to contract');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_verification_run_receipt_insert
        BEFORE INSERT ON verification_runs
        WHEN NEW.receipt_id IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM receipts WHERE id = NEW.receipt_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'verification receipt does not exist');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_verification_run_receipt_update
        BEFORE UPDATE OF receipt_id ON verification_runs
        WHEN NEW.receipt_id IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM receipts WHERE id = NEW.receipt_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'verification receipt does not exist');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_verification_run_state_insert
        BEFORE INSERT ON verification_runs
        WHEN NEW.phase NOT IN ('Before','After') OR NEW.status NOT IN ('Pending','Running','Passed','Failed','Error','Interrupted')
        BEGIN
            SELECT RAISE(ABORT, 'invalid verification phase or status');
        END;

        CREATE TRIGGER IF NOT EXISTS trg_verification_run_state_update
        BEFORE UPDATE OF phase, status ON verification_runs
        WHEN NEW.phase NOT IN ('Before','After') OR NEW.status NOT IN ('Pending','Running','Passed','Failed','Error','Interrupted')
        BEGIN
            SELECT RAISE(ABORT, 'invalid verification phase or status');
        END;",
    ),
];

fn current_migration_version() -> i64 {
    MIGRATIONS.len() as i64
}

fn get_db_schema_version(conn: &Connection) -> MResult<i64> {
    let value: Result<String, rusqlite::Error> = conn.query_row(
        "SELECT value FROM reprodeck_meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    );

    match value {
        Ok(value) => value
            .parse::<i64>()
            .map_err(|_| MigrationError::MigrationFailed("schema_version corrupted".to_string())),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(error) => Err(MigrationError::Db(error)),
    }
}

#[allow(dead_code)]
fn set_db_schema_version(conn: &Connection, version: i64) -> MResult<()> {
    conn.execute(
        "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        params![version.to_string()],
    )?;
    Ok(())
}

fn apply_migrations(conn: &mut Connection) -> MResult<()> {
    let current = get_db_schema_version(conn)?;
    let latest = current_migration_version();
    if current > latest {
        return Err(MigrationError::UnknownSchemaVersion(current));
    }

    for index in (current as usize + 1)..=MIGRATIONS.len() {
        let (version, sql) = MIGRATIONS.get(index - 1).expect("migration index is valid");
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![version.parse::<i64>().expect("static migration version").to_string()],
        )?;
        tx.commit()?;
    }

    Ok(())
}

pub fn init_db(path: &Path) -> MResult<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    apply_migrations(&mut conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn fresh_db_applies_all_migrations() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        assert_eq!(
            get_db_schema_version(&conn).unwrap(),
            current_migration_version()
        );
    }

    #[test]
    fn pragmas_and_foreign_keys_enabled() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .unwrap();
        assert!(journal_mode.eq_ignore_ascii_case("wal"));
    }

    #[test]
    fn upgrade_from_previous_schema() {
        let tmp = NamedTempFile::new().unwrap();
        let mut conn = Connection::open(tmp.path()).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute_batch(MIGRATIONS[0].1).unwrap();
        tx.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params!["1"],
        )
        .unwrap();
        tx.commit().unwrap();

        let conn = init_db(tmp.path()).unwrap();
        assert_eq!(
            get_db_schema_version(&conn).unwrap(),
            current_migration_version()
        );
    }

    #[test]
    fn init_db_idempotent() {
        let tmp = NamedTempFile::new().unwrap();
        init_db(tmp.path()).unwrap();
        init_db(tmp.path()).unwrap();
    }

    #[test]
    fn unsupported_newer_schema_is_rejected() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)",
            params!["999"],
        )
        .unwrap();
        assert!(matches!(
            init_db(tmp.path()),
            Err(MigrationError::UnknownSchemaVersion(_))
        ));
    }

    #[test]
    fn corrupted_schema_version_returns_error() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)",
            params!["not-a-number"],
        )
        .unwrap();
        assert!(matches!(
            init_db(tmp.path()),
            Err(MigrationError::MigrationFailed(_))
        ));
    }

    #[test]
    fn failing_migration_rolls_back() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)",
            params!["1"],
        )
        .unwrap();
        let mut conn = Connection::open(tmp.path()).unwrap();
        let tx = conn.transaction().unwrap();
        assert!(tx
            .execute_batch("CREATE TABLE test_temp(id INTEGER); INVALID SQL;")
            .is_err());
        drop(tx);
        assert_eq!(get_db_schema_version(&conn).unwrap(), 1);
    }

    fn seed_session_and_contract(conn: &Connection) {
        conn.execute(
            "INSERT INTO sessions(id, repo_id, created_at, updated_at, state) VALUES ('s1','r',1,1,'Active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outcome_contracts(id, session_id, title, state, version, created_at) VALUES ('c1','s1','contract','Draft',1,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outcome_contracts(id, session_id, title, state, version, created_at) VALUES ('c2','s1','contract2','Draft',1,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verification_checks(id, contract_id, stable_id, description, required, ordering) VALUES ('check1','c1','stable','check',1,0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn outcome_contract_requires_existing_session() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        let result = conn.execute(
            "INSERT INTO outcome_contracts(id, session_id, title, state, version, created_at) VALUES ('bad','missing','x','Draft',1,1)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn verification_check_must_belong_to_run_contract() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        seed_session_and_contract(&conn);
        let result = conn.execute(
            "INSERT INTO verification_runs(id, contract_id, check_id, phase, status, started_at) VALUES ('run','c2','check1','Before','Running',1)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn verification_state_values_are_enforced() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        seed_session_and_contract(&conn);
        let result = conn.execute(
            "INSERT INTO verification_runs(id, contract_id, check_id, phase, status, started_at) VALUES ('run','c1','check1','Maybe','Magic',1)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn deleting_session_cascades_outcome_contracts_via_trigger() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = init_db(tmp.path()).unwrap();
        seed_session_and_contract(&conn);
        conn.execute("DELETE FROM sessions WHERE id = 's1'", [])
            .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM outcome_contracts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
