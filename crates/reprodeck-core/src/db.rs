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
            id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            started_at INTEGER NOT NULL
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
        let (ver, sql) = MIGRATIONS.get(i - 1).unwrap();
        // execute migration inside a Rust-owned transaction
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO reprodeck_meta(key,value) VALUES('schema_version',?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![ver.parse::<i64>().unwrap().to_string()],
        )?;
        tx.commit()?;
    }

    Ok(())
}

/// Initialise or open the SQLite database at `path` and ensure required schema via migrations.
pub fn init_db(path: &Path) -> MResult<Connection> {
    let mut conn = Connection::open(path)?;

    // pragmas
    conn.pragma_update(None, "foreign_keys", true)?;

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
