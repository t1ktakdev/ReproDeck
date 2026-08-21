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
        // migration 1: reprodeck_meta and repositories table
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
        // migration 2: sessions, shadow_workspaces, command_executions, timeline_events, evidence, outcome_criteria
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
        // migration 3: actions, executions, receipts, artifacts
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

        CREATE INDEX IF NOT EXISTS idx_artifacts_receipt ON artifacts(receipt_id);
        "
    ),
    (
        "4",
        "-- migration 4: outcome verification tables and evidence_links
        CREATE TABLE IF NOT EXISTS outcome_contracts (
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
        );"
    ),
    (
        "5",
        "CREATE INDEX IF NOT EXISTS idx_repositories_path ON repositories(path);

        CREATE TABLE IF NOT EXISTS environment_snapshots (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            captured_at INTEGER NOT NULL,
            os TEXT NOT NULL,
            arch TEXT NOT NULL,
            git_version TEXT,
            runtimes_json TEXT NOT NULL,
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_environment_session ON environment_snapshots(session_id);

        CREATE TABLE IF NOT EXISTS reproduction_steps (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            ordering INTEGER NOT NULL DEFAULT 0,
            executable TEXT NOT NULL,
            args_json TEXT NOT NULL,
            expected_exit_code INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_reproduction_steps_session ON reproduction_steps(session_id, ordering);

        CREATE TABLE IF NOT EXISTS reproduction_runs (
            id TEXT PRIMARY KEY,
            step_id TEXT NOT NULL,
            phase TEXT NOT NULL CHECK(phase IN ('Before','After')),
            action_id TEXT NOT NULL,
            receipt_id TEXT,
            exit_code INTEGER,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(step_id) REFERENCES reproduction_steps(id) ON DELETE CASCADE,
            FOREIGN KEY(action_id) REFERENCES actions(id) ON DELETE CASCADE,
            FOREIGN KEY(receipt_id) REFERENCES receipts(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_reproduction_runs_step ON reproduction_runs(step_id, phase, created_at);
        "
    ),
    (
        "6",
        "ALTER TABLE reproduction_steps ADD COLUMN active_cycle INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE reproduction_runs ADD COLUMN cycle INTEGER NOT NULL DEFAULT 1;

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS evidence_items (
            created_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            session_id TEXT NOT NULL,
            action_id TEXT,
            receipt_id TEXT,
            kind TEXT NOT NULL,
            source TEXT NOT NULL,
            summary TEXT NOT NULL,
            artifact_id TEXT,
            checksum TEXT,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY(action_id) REFERENCES actions(id) ON DELETE SET NULL,
            FOREIGN KEY(receipt_id) REFERENCES receipts(id) ON DELETE SET NULL,
            FOREIGN KEY(artifact_id) REFERENCES artifacts(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_evidence_items_session ON evidence_items(session_id, created_seq);

        CREATE TABLE IF NOT EXISTS imported_capsules (
            id TEXT PRIMARY KEY,
            source_path TEXT NOT NULL,
            stored_path TEXT NOT NULL,
            session_id TEXT,
            title TEXT,
            format_version INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            imported_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_imported_capsules_imported_at ON imported_capsules(imported_at DESC);
        "
    ),
    (
        "7",
        "CREATE TABLE IF NOT EXISTS project_profiles (
            root_path TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL,
            profile_json TEXT NOT NULL,
            analyzed_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_project_profiles_analyzed_at ON project_profiles(analyzed_at DESC);
        "
    ),
    (
        "8",
        "CREATE TABLE IF NOT EXISTS project_health_runs (
            id TEXT PRIMARY KEY,
            root_path TEXT NOT NULL,
            base_commit TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER NOT NULL,
            status TEXT NOT NULL,
            report_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_project_health_runs_root_finished ON project_health_runs(root_path, finished_at DESC);

        CREATE TABLE IF NOT EXISTS project_problems (
            id TEXT PRIMARY KEY,
            problem_key TEXT UNIQUE NOT NULL,
            root_path TEXT NOT NULL,
            status TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            command_id TEXT NOT NULL,
            health_run_id TEXT NOT NULL,
            check_run_id TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            first_seen_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            cleared_at INTEGER,
            occurrences INTEGER NOT NULL DEFAULT 1,
            FOREIGN KEY(health_run_id) REFERENCES project_health_runs(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_project_problems_root_last_seen ON project_problems(root_path, last_seen_at DESC);
        CREATE INDEX IF NOT EXISTS idx_project_problems_command ON project_problems(root_path, command_id);
        "
    ),
    (
        "9",
        "CREATE TABLE IF NOT EXISTS investigation_cases (
            id TEXT PRIMARY KEY,
            root_path TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            project_relative_path TEXT NOT NULL,
            project_name TEXT NOT NULL,
            health_run_id TEXT NOT NULL,
            cluster_id TEXT NOT NULL,
            base_commit TEXT NOT NULL,
            state TEXT NOT NULL,
            case_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(health_run_id, cluster_id),
            FOREIGN KEY(health_run_id) REFERENCES project_health_runs(id) ON DELETE RESTRICT
        );
        CREATE INDEX IF NOT EXISTS idx_investigation_cases_root_updated ON investigation_cases(root_path, updated_at DESC);

        CREATE TABLE IF NOT EXISTS investigation_workspaces (
            case_id TEXT PRIMARY KEY,
            repo_root TEXT NOT NULL,
            project_relative_path TEXT NOT NULL,
            base_commit TEXT NOT NULL,
            branch TEXT NOT NULL,
            worktree_path TEXT NOT NULL,
            original_head TEXT NOT NULL,
            original_branch TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY(case_id) REFERENCES investigation_cases(id) ON DELETE CASCADE
        );
        "
    ),
    (
        "10",
        "CREATE TABLE IF NOT EXISTS verification_handoffs (
            session_id TEXT PRIMARY KEY,
            investigation_case_id TEXT NOT NULL,
            hypothesis_id TEXT NOT NULL,
            experiment_id TEXT NOT NULL,
            source_commit TEXT NOT NULL,
            patch_sha256 TEXT NOT NULL,
            patch_size INTEGER NOT NULL,
            patch_bytes BLOB NOT NULL,
            files_json TEXT NOT NULL,
            activated_at INTEGER,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS verification_proofs (
            session_id TEXT PRIMARY KEY,
            step_id TEXT NOT NULL,
            cycle INTEGER NOT NULL,
            source_commit TEXT NOT NULL,
            source_state_sha256 TEXT NOT NULL,
            shadow_commit TEXT NOT NULL,
            patch_sha256 TEXT NOT NULL,
            patch_size INTEGER NOT NULL,
            files_json TEXT NOT NULL,
            criterion_sha256 TEXT NOT NULL,
            command_sha256 TEXT NOT NULL,
            after_run_id TEXT NOT NULL,
            verified_at INTEGER NOT NULL,
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY(step_id) REFERENCES reproduction_steps(id) ON DELETE CASCADE,
            FOREIGN KEY(after_run_id) REFERENCES reproduction_runs(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS regression_checks (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            stable_id TEXT NOT NULL,
            title TEXT NOT NULL,
            executable TEXT NOT NULL,
            args_json TEXT NOT NULL,
            expected_exit_code INTEGER NOT NULL DEFAULT 0,
            level TEXT NOT NULL CHECK(level IN ('Required','Recommended','Optional')),
            status TEXT NOT NULL DEFAULT 'Pending',
            receipt_id TEXT,
            verified_patch_sha256 TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(session_id, stable_id),
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY(receipt_id) REFERENCES receipts(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_regression_checks_session ON regression_checks(session_id, level, created_at);"
    ),
];

fn current_migration_version() -> i64 {
    MIGRATIONS.len() as i64
}

fn get_db_schema_version(conn: &Connection) -> MResult<i64> {
    let val: Result<String, rusqlite::Error> = conn.query_row(
        "SELECT value FROM reprodeck_meta WHERE key = 'schema_version'",
        [],
        |r| r.get(0),
    );

    match val {
        Ok(v) => v
            .parse::<i64>()
            .map_err(|_| MigrationError::MigrationFailed("schema_version corrupted".to_string())),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(MigrationError::Db(e)),
    }
}

#[allow(dead_code)]
fn set_db_schema_version(conn: &Connection, v: i64) -> MResult<()> {
    conn.execute(
        "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        params![v.to_string()],
    )?;
    Ok(())
}

/// Apply pending migrations in order. Transaction-safe per migration using BEGIN/COMMIT inside SQL.
fn apply_migrations(conn: &mut Connection) -> MResult<()> {
    let current = get_db_schema_version(conn)?;
    let latest = current_migration_version();
    if current > latest {
        return Err(MigrationError::UnknownSchemaVersion(current));
    }

    for i in (current as usize + 1)..=MIGRATIONS.len() {
        let (ver, sql) = MIGRATIONS
            .get(i - 1)
            .ok_or_else(|| MigrationError::MigrationFailed(format!("missing migration {i}")))?;
        let parsed_version = ver.parse::<i64>().map_err(|_| {
            MigrationError::MigrationFailed(format!("invalid migration version {ver}"))
        })?;
        // execute migration inside a Rust-owned transaction
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![parsed_version.to_string()],
        )?;
        tx.commit()?;
    }

    Ok(())
}

/// Initialise or open the SQLite database at `path` and ensure required schema via migrations.
pub fn init_db(path: &Path) -> MResult<Connection> {
    let mut conn = Connection::open(path)?;

    // pragmas and connection-level settings
    // enable foreign keys
    conn.pragma_update(None, "foreign_keys", true)?;
    // journal mode and synchronous for WAL durability/performance tradeoff
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    // busy timeout (ms)
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;

    // ensure reprodeck_meta exists so we can store schema_version
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
        let path = tmp.path();

        let conn = init_db(path).expect("init db");

        let ver = get_db_schema_version(&conn).unwrap();
        assert_eq!(ver, current_migration_version());
    }

    #[test]
    fn pragmas_and_foreign_keys_enabled() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = init_db(path).expect("init db");
        // check foreign_keys pragma
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        // check journal_mode (string)
        let jm: String = conn
            .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
            .unwrap();
        assert!(jm.eq_ignore_ascii_case("wal"));
    }

    #[test]
    fn upgrade_from_previous_schema() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        // create DB with only migration 1 applied
        let mut conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        // apply migration 1 inside transaction
        let tx = conn.transaction().unwrap();
        tx.execute_batch(MIGRATIONS[0].1).unwrap();
        tx.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params!["1"],
        )
        .unwrap();
        tx.commit().unwrap();

        // now init_db should apply migration 2
        let conn2 = init_db(path).expect("migrate");
        let ver = get_db_schema_version(&conn2).unwrap();
        assert_eq!(ver, current_migration_version());
    }

    #[test]
    fn init_db_idempotent() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let _ = init_db(path).unwrap();
        let _ = init_db(path).unwrap();
    }

    #[test]
    fn unsupported_newer_schema_is_rejected() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)",
            params!["999"],
        )
        .unwrap();
        let res = init_db(path);
        assert!(matches!(res, Err(MigrationError::UnknownSchemaVersion(_))));
    }

    #[test]
    fn corrupted_schema_version_returns_error() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)",
            params!["not-a-number"],
        )
        .unwrap();
        let res = init_db(path);
        assert!(matches!(res, Err(MigrationError::MigrationFailed(_))));
    }

    #[test]
    fn failing_migration_rolls_back() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS reprodeck_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1)",
            params!["1"],
        )
        .unwrap();

        // apply a failing migration inside a transaction and ensure rollback
        let mut conn2 = Connection::open(path).unwrap();
        let tx = conn2.transaction().unwrap();
        // a migration that creates a table then fails
        let res = tx.execute_batch("CREATE TABLE test_temp(id INTEGER);; INVALID SQL;");
        assert!(res.is_err());
        drop(tx);
        // ensure schema_version still 1
        let ver = get_db_schema_version(&conn2).unwrap();
        assert_eq!(ver, 1);
    }
}
