use crate::redaction;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("artifact not found: {0}")]
    ArtifactNotFound(String),
    #[error("verification run not found: {0}")]
    VerificationRunNotFound(String),
    #[error("verification run has no receipt yet: {0}")]
    RunNotFinished(String),
    #[error(
        "artifact {artifact_id} belongs to receipt {artifact_receipt}, not verification receipt {run_receipt}"
    )]
    ArtifactReceiptMismatch {
        artifact_id: String,
        artifact_receipt: String,
        run_receipt: String,
    },
    #[error("artifact store key is invalid")]
    InvalidStoreKey,
    #[error("artifact integrity check failed: {0}")]
    Integrity(String),
}

pub type Result<T> = std::result::Result<T, EvidenceError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub created_seq: i64,
    pub id: String,
    pub receipt_id: String,
    pub store_key: String,
    pub checksum: String,
    pub size: i64,
    pub media_type: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactRole {
    Before,
    After,
    Verification,
    Diagnostic,
    Attachment,
}

impl std::fmt::Display for ArtifactRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactRole::Before => write!(f, "Before"),
            ArtifactRole::After => write!(f, "After"),
            ArtifactRole::Verification => write!(f, "Verification"),
            ArtifactRole::Diagnostic => write!(f, "Diagnostic"),
            ArtifactRole::Attachment => write!(f, "Attachment"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactLink {
    pub id: String,
    pub artifact_id: String,
    pub run_id: Option<String>,
    pub role: ArtifactRole,
    pub created_at: i64,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn parse_role(value: &str) -> Result<ArtifactRole> {
    match value {
        "Before" => Ok(ArtifactRole::Before),
        "After" => Ok(ArtifactRole::After),
        "Verification" => Ok(ArtifactRole::Verification),
        "Diagnostic" => Ok(ArtifactRole::Diagnostic),
        "Attachment" => Ok(ArtifactRole::Attachment),
        _ => Err(EvidenceError::Integrity(format!(
            "unknown artifact role {value}"
        ))),
    }
}

fn is_symlink_or_reparse(path: &Path) -> std::io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    let is_symlink = metadata.file_type().is_symlink();

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        let is_reparse = (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
        Ok(is_symlink || is_reparse)
    }

    #[cfg(not(windows))]
    {
        Ok(is_symlink)
    }
}

fn verify_existing_artifact(
    path: &Path,
    expected_checksum: &str,
    expected_size: usize,
) -> std::io::Result<()> {
    if is_symlink_or_reparse(path)? {
        return Err(std::io::Error::other(
            "artifact final path is a symlink or reparse point",
        ));
    }

    let bytes = fs::read(path)?;
    if bytes.len() != expected_size {
        return Err(std::io::Error::other(
            "artifact store integrity mismatch: existing size differs",
        ));
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected_checksum {
        return Err(std::io::Error::other(
            "artifact store integrity mismatch: existing checksum differs",
        ));
    }
    Ok(())
}

/// Store bytes by content hash. This function only manages the content store;
/// use `persist_text_artifact` for command/log text so redaction happens before
/// bytes reach disk.
pub fn store_artifact(storage_dir: &Path, data: &[u8]) -> std::io::Result<(String, PathBuf)> {
    fs::create_dir_all(storage_dir)?;
    let base = storage_dir.canonicalize()?;

    let checksum = hex::encode(Sha256::digest(data));
    let prefix = checksum
        .get(0..2)
        .ok_or_else(|| std::io::Error::other("checksum too short"))?;
    let dir = storage_dir.join(prefix);

    if dir.exists() && is_symlink_or_reparse(&dir)? {
        return Err(std::io::Error::other(
            "artifact storage prefix is a symlink or reparse point",
        ));
    }

    fs::create_dir_all(&dir)?;
    let dir_canon = dir.canonicalize()?;
    if !dir_canon.starts_with(&base) {
        return Err(std::io::Error::other(
            "artifact dir canonicalization outside storage root",
        ));
    }

    let final_path = dir.join(&checksum);
    if final_path.exists() {
        verify_existing_artifact(&final_path, &checksum, data.len())?;
        return Ok((checksum, final_path));
    }

    let temp_path = dir.join(format!("{}.{}.tmp", checksum, Uuid::new_v4()));
    if let Err(error) = fs::write(&temp_path, data) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    let current_dir_canon = dir.canonicalize()?;
    if current_dir_canon != dir_canon || !current_dir_canon.starts_with(&base) {
        let _ = fs::remove_file(&temp_path);
        return Err(std::io::Error::other(
            "artifact directory changed or escaped storage root",
        ));
    }

    match fs::rename(&temp_path, &final_path) {
        Ok(()) => {}
        Err(_) if final_path.exists() => {
            let _ = fs::remove_file(&temp_path);
            verify_existing_artifact(&final_path, &checksum, data.len())?;
        }
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    }

    let final_canon = final_path.canonicalize()?;
    if !final_canon.starts_with(&base) {
        let _ = fs::remove_file(&final_path);
        return Err(std::io::Error::other(
            "artifact stored outside storage root",
        ));
    }
    verify_existing_artifact(&final_path, &checksum, data.len())?;
    Ok((checksum, final_path))
}

/// Ensure a candidate existing path is inside the content store.
pub fn path_within_storage(storage_dir: &Path, path: &Path) -> bool {
    match (path.canonicalize(), storage_dir.canonicalize()) {
        (Ok(candidate), Ok(base)) => candidate.starts_with(base),
        _ => false,
    }
}

fn validate_store_key(key: &str) -> Result<PathBuf> {
    let path = Path::new(key);
    if path.is_absolute() || path.as_os_str().is_empty() {
        return Err(EvidenceError::InvalidStoreKey);
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(EvidenceError::InvalidStoreKey);
        }
    }
    Ok(path.to_path_buf())
}

fn artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        created_seq: row.get(0)?,
        id: row.get(1)?,
        receipt_id: row.get(2)?,
        store_key: row.get(3)?,
        checksum: row.get(4)?,
        size: row.get(5)?,
        media_type: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn persist_artifact_bytes(
    conn: &Connection,
    storage_dir: &Path,
    receipt_id: &str,
    data: &[u8],
    media_type: Option<&str>,
) -> Result<ArtifactRecord> {
    let (checksum, final_path) = store_artifact(storage_dir, data)?;
    let relative = final_path
        .strip_prefix(storage_dir)
        .map_err(|_| EvidenceError::InvalidStoreKey)?;
    let store_key = relative
        .to_str()
        .ok_or(EvidenceError::InvalidStoreKey)?
        .replace('\\', "/");
    validate_store_key(&store_key)?;

    let id = Uuid::new_v4().to_string();
    let created_at = unix_time_secs()?;
    conn.execute(
        "INSERT INTO artifacts(id, receipt_id, store_key, checksum, size, media_type, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![
            id,
            receipt_id,
            store_key,
            checksum,
            data.len() as i64,
            media_type,
            created_at
        ],
    )?;
    get_artifact(conn, &id)?.ok_or(EvidenceError::ArtifactNotFound(id))
}

/// Persist text evidence after central secret redaction. This is the preferred
/// API for stdout, stderr, logs, command output and generated diagnostic text.
pub fn persist_text_artifact(
    conn: &Connection,
    storage_dir: &Path,
    receipt_id: &str,
    text: &str,
    media_type: Option<&str>,
) -> Result<ArtifactRecord> {
    let redacted = redaction::redact_text(text);
    persist_artifact_bytes(
        conn,
        storage_dir,
        receipt_id,
        redacted.as_bytes(),
        media_type.or(Some("text/plain; charset=utf-8")),
    )
}

/// Persist a binary/user attachment. Callers must only use this for content
/// where text secret redaction is not applicable (for example an image).
pub fn persist_binary_attachment(
    conn: &Connection,
    storage_dir: &Path,
    receipt_id: &str,
    data: &[u8],
    media_type: Option<&str>,
) -> Result<ArtifactRecord> {
    persist_artifact_bytes(conn, storage_dir, receipt_id, data, media_type)
}

pub fn get_artifact(conn: &Connection, artifact_id: &str) -> Result<Option<ArtifactRecord>> {
    Ok(conn
        .query_row(
            "SELECT created_seq, id, receipt_id, store_key, checksum, size, media_type, created_at FROM artifacts WHERE id = ?1",
            rusqlite::params![artifact_id],
            artifact_from_row,
        )
        .optional()?)
}

pub fn list_artifacts_for_receipt(
    conn: &Connection,
    receipt_id: &str,
) -> Result<Vec<ArtifactRecord>> {
    let mut stmt = conn.prepare(
        "SELECT created_seq, id, receipt_id, store_key, checksum, size, media_type, created_at FROM artifacts WHERE receipt_id = ?1 ORDER BY created_seq ASC",
    )?;
    Ok(stmt
        .query_map(rusqlite::params![receipt_id], artifact_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Read an artifact only by its database identity. No arbitrary filesystem path
/// from the frontend is accepted here.
pub fn read_artifact(conn: &Connection, storage_dir: &Path, artifact_id: &str) -> Result<Vec<u8>> {
    let artifact = get_artifact(conn, artifact_id)?
        .ok_or_else(|| EvidenceError::ArtifactNotFound(artifact_id.to_owned()))?;
    let relative = validate_store_key(&artifact.store_key)?;
    let path = storage_dir.join(relative);
    if !path_within_storage(storage_dir, &path) {
        return Err(EvidenceError::InvalidStoreKey);
    }
    let bytes = fs::read(&path)?;
    if bytes.len() as i64 != artifact.size {
        return Err(EvidenceError::Integrity("stored size differs".to_string()));
    }
    let checksum = hex::encode(Sha256::digest(&bytes));
    if checksum != artifact.checksum {
        return Err(EvidenceError::Integrity(
            "stored checksum differs".to_string(),
        ));
    }
    Ok(bytes)
}

fn verification_receipt(conn: &Connection, run_id: &str) -> Result<String> {
    let receipt = conn
        .query_row(
            "SELECT receipt_id FROM verification_runs WHERE id = ?1",
            rusqlite::params![run_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .ok_or_else(|| EvidenceError::VerificationRunNotFound(run_id.to_owned()))?;
    receipt.ok_or_else(|| EvidenceError::RunNotFinished(run_id.to_owned()))
}

/// Link an artifact to a verification run only when the artifact was produced by
/// the very same receipt that completed that run. This prevents a valid artifact
/// from another command/run being relabelled as BEFORE/AFTER proof.
pub fn link_artifact(
    conn: &Connection,
    artifact_id: &str,
    run_id: Option<&str>,
    role: ArtifactRole,
) -> Result<ArtifactLink> {
    let artifact = get_artifact(conn, artifact_id)?
        .ok_or_else(|| EvidenceError::ArtifactNotFound(artifact_id.to_owned()))?;

    if let Some(run_id) = run_id {
        let run_receipt = verification_receipt(conn, run_id)?;
        if artifact.receipt_id != run_receipt {
            return Err(EvidenceError::ArtifactReceiptMismatch {
                artifact_id: artifact_id.to_owned(),
                artifact_receipt: artifact.receipt_id,
                run_receipt,
            });
        }
    }

    let id = Uuid::new_v4().to_string();
    let created_at = unix_time_secs()?;
    conn.execute(
        "INSERT INTO artifact_links(id, artifact_id, run_id, role, created_at) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![id, artifact_id, run_id, role.to_string(), created_at],
    )?;
    Ok(ArtifactLink {
        id,
        artifact_id: artifact_id.to_owned(),
        run_id: run_id.map(str::to_owned),
        role,
        created_at,
    })
}

pub fn list_artifact_links_for_run(conn: &Connection, run_id: &str) -> Result<Vec<ArtifactLink>> {
    let mut stmt = conn.prepare(
        "SELECT id, artifact_id, run_id, role, created_at FROM artifact_links WHERE run_id = ?1 ORDER BY rowid ASC",
    )?;
    let raw = stmt
        .query_map(rusqlite::params![run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    raw.into_iter()
        .map(|(id, artifact_id, run_id, role, created_at)| {
            Ok(ArtifactLink {
                id,
                artifact_id,
                run_id,
                role: parse_role(&role)?,
                created_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::init_db, timeline, verification};
    use tempfile::{tempdir, NamedTempFile};

    fn ensure_session(conn: &Connection) {
        if timeline::get_session(conn, "session").unwrap().is_none() {
            timeline::create_session(conn, "session", "Active", None).unwrap();
        }
    }

    fn receipt(conn: &mut Connection, suffix: &str) -> String {
        ensure_session(conn);
        let action = timeline::Action {
            id: format!("action-{suffix}"),
            session_id: "session".to_string(),
            parent_id: None,
            kind: "command".to_string(),
            meta: None,
            state: "Running".to_string(),
            created_at: 1,
        };
        timeline::create_action(conn, &action).unwrap();
        let execution = timeline::start_execution(conn, &action.id).unwrap();
        timeline::finish_execution(conn, &execution, "Succeeded", None, None).unwrap()
    }

    fn verification_run_with_receipt(conn: &mut Connection) -> (String, String) {
        ensure_session(conn);
        let contract =
            verification::create_outcome_contract(conn, "session", "Outcome", None).unwrap();
        let check = verification::add_verification_check(
            conn,
            &contract.id,
            "check",
            "Check",
            None,
            Some("exit 0"),
            true,
            0,
        )
        .unwrap();
        let run = verification::start_verification_check_run(
            conn,
            &contract.id,
            &check.id,
            verification::RunPhase::Before,
        )
        .unwrap();
        let receipt = verification::finish_verification_run_with_output(
            conn,
            &run,
            verification::RunStatus::Failed,
            Some("evidence"),
            None,
        )
        .unwrap();
        (run, receipt)
    }

    #[test]
    fn store_and_check_artifact() {
        let dir = tempdir().unwrap();
        let (checksum, path) = store_artifact(dir.path(), b"hello world").unwrap();
        assert!(path.exists());
        assert_eq!(checksum.len(), 64);
        assert!(path.starts_with(dir.path()));
        assert_eq!(fs::read(path).unwrap(), b"hello world");
    }

    #[test]
    fn duplicate_artifact_idempotent() {
        let dir = tempdir().unwrap();
        let data = b"same content";
        let (first_checksum, first_path) = store_artifact(dir.path(), data).unwrap();
        let (second_checksum, second_path) = store_artifact(dir.path(), data).unwrap();
        assert_eq!(first_checksum, second_checksum);
        assert_eq!(first_path, second_path);
        assert_eq!(fs::read(first_path).unwrap(), data);
    }

    #[test]
    fn existing_corrupt_content_is_rejected() {
        let dir = tempdir().unwrap();
        let data = b"expected content";
        let checksum = hex::encode(Sha256::digest(data));
        let prefix_dir = dir.path().join(&checksum[0..2]);
        fs::create_dir_all(&prefix_dir).unwrap();
        fs::write(prefix_dir.join(&checksum), b"corrupt").unwrap();
        assert!(store_artifact(dir.path(), data).is_err());
    }

    #[test]
    fn path_within_storage_detects_outside() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("foo");
        fs::write(&outside_file, b"x").unwrap();
        assert!(!path_within_storage(dir.path(), &outside_file));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_prefix_prevented() {
        use std::os::unix::fs as unixfs;
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let data = b"symlink test";
        let checksum = hex::encode(Sha256::digest(data));
        let prefix_path = dir.path().join(&checksum[0..2]);
        unixfs::symlink(outside.path(), &prefix_path).unwrap();
        assert!(store_artifact(dir.path(), data).is_err());
        assert!(!outside.path().join(&checksum).exists());
    }

    #[test]
    fn text_is_redacted_before_artifact_storage() {
        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        let receipt_id = receipt(&mut conn, "redaction");
        let storage = tempdir().unwrap();
        let secret = "password=hunter2 Bearer secret-token";
        let artifact = persist_text_artifact(
            &conn,
            storage.path(),
            &receipt_id,
            secret,
            Some("text/plain"),
        )
        .unwrap();
        let bytes = read_artifact(&conn, storage.path(), &artifact.id).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("secret-token"));
        assert!(text.contains("REDACTED"));
    }

    #[test]
    fn artifact_read_uses_database_identity_and_verifies_integrity() {
        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        let receipt_id = receipt(&mut conn, "integrity");
        let storage = tempdir().unwrap();
        let artifact = persist_binary_attachment(
            &conn,
            storage.path(),
            &receipt_id,
            b"binary\0data",
            Some("application/octet-stream"),
        )
        .unwrap();
        assert_eq!(
            read_artifact(&conn, storage.path(), &artifact.id).unwrap(),
            b"binary\0data"
        );
        let path = storage.path().join(&artifact.store_key);
        fs::write(path, b"tampered").unwrap();
        assert!(matches!(
            read_artifact(&conn, storage.path(), &artifact.id),
            Err(EvidenceError::Integrity(_))
        ));
    }

    #[test]
    fn artifact_links_require_same_verification_receipt() {
        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        let (run, run_receipt) = verification_run_with_receipt(&mut conn);
        let storage = tempdir().unwrap();
        let artifact = persist_text_artifact(
            &conn,
            storage.path(),
            &run_receipt,
            "evidence",
            None,
        )
        .unwrap();
        link_artifact(&conn, &artifact.id, Some(&run), ArtifactRole::Before).unwrap();
        let links = list_artifact_links_for_run(&conn, &run).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].role, ArtifactRole::Before);
        assert_eq!(links[0].artifact_id, artifact.id);
    }

    #[test]
    fn artifact_from_another_receipt_cannot_be_relabelled_as_run_proof() {
        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        let unrelated_receipt = receipt(&mut conn, "unrelated");
        let (run, _run_receipt) = verification_run_with_receipt(&mut conn);
        let storage = tempdir().unwrap();
        let artifact = persist_text_artifact(
            &conn,
            storage.path(),
            &unrelated_receipt,
            "unrelated evidence",
            None,
        )
        .unwrap();
        assert!(matches!(
            link_artifact(&conn, &artifact.id, Some(&run), ArtifactRole::Before),
            Err(EvidenceError::ArtifactReceiptMismatch { .. })
        ));
        assert!(list_artifact_links_for_run(&conn, &run).unwrap().is_empty());
    }

    #[test]
    fn unfinished_run_cannot_receive_proof_artifact() {
        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        ensure_session(&conn);
        let unrelated_receipt = receipt(&mut conn, "unfinished");
        let contract =
            verification::create_outcome_contract(&conn, "session", "Outcome", None).unwrap();
        let check = verification::add_verification_check(
            &conn,
            &contract.id,
            "unfinished-check",
            "Check",
            None,
            Some("exit 0"),
            true,
            0,
        )
        .unwrap();
        let run = verification::start_verification_check_run(
            &mut conn,
            &contract.id,
            &check.id,
            verification::RunPhase::Before,
        )
        .unwrap();
        let storage = tempdir().unwrap();
        let artifact = persist_text_artifact(
            &conn,
            storage.path(),
            &unrelated_receipt,
            "evidence",
            None,
        )
        .unwrap();
        assert!(matches!(
            link_artifact(&conn, &artifact.id, Some(&run), ArtifactRole::Before),
            Err(EvidenceError::RunNotFinished(_))
        ));
    }

    #[test]
    fn invalid_store_key_is_rejected_before_read() {
        assert!(validate_store_key("../outside").is_err());
        assert!(validate_store_key("/absolute").is_err());
    }
}
