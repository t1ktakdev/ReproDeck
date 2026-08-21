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
    NotFound(String),
    #[error("invalid artifact store key")]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceKind {
    CommandFailure,
    CommandSuccess,
    StdoutFragment,
    StderrFragment,
    ChangedFile,
    GitDiff,
    EnvironmentValue,
    TestResult,
    StackTrace,
    UserNote,
}

impl EvidenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CommandFailure => "command_failure",
            Self::CommandSuccess => "command_success",
            Self::StdoutFragment => "stdout_fragment",
            Self::StderrFragment => "stderr_fragment",
            Self::ChangedFile => "changed_file",
            Self::GitDiff => "git_diff",
            Self::EnvironmentValue => "environment_value",
            Self::TestResult => "test_result",
            Self::StackTrace => "stack_trace",
            Self::UserNote => "user_note",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceItem {
    pub created_seq: i64,
    pub id: String,
    pub session_id: String,
    pub action_id: Option<String>,
    pub receipt_id: Option<String>,
    pub kind: String,
    pub source: String,
    pub summary: String,
    pub artifact_id: Option<String>,
    pub checksum: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewEvidenceItem<'a> {
    pub session_id: &'a str,
    pub action_id: Option<&'a str>,
    pub receipt_id: Option<&'a str>,
    pub kind: EvidenceKind,
    pub source: &'a str,
    pub summary: &'a str,
    pub artifact: Option<&'a ArtifactRecord>,
}

pub fn create_evidence_item(conn: &Connection, input: NewEvidenceItem<'_>) -> Result<EvidenceItem> {
    let id = Uuid::new_v4().to_string();
    let now = unix_time_secs()?;
    let sanitized_summary = redaction::redact_text(input.summary);
    conn.execute(
        "INSERT INTO evidence_items(id,session_id,action_id,receipt_id,kind,source,summary,artifact_id,checksum,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![
            id,
            input.session_id,
            input.action_id,
            input.receipt_id,
            input.kind.as_str(),
            input.source,
            sanitized_summary,
            input.artifact.map(|value| value.id.as_str()),
            input.artifact.map(|value| value.checksum.as_str()),
            now
        ],
    )?;
    get_evidence_item(conn, &id)?.ok_or_else(|| EvidenceError::NotFound(id))
}

pub fn get_evidence_item(conn: &Connection, id: &str) -> Result<Option<EvidenceItem>> {
    Ok(conn.query_row(
        "SELECT created_seq,id,session_id,action_id,receipt_id,kind,source,summary,artifact_id,checksum,created_at FROM evidence_items WHERE id=?1",
        rusqlite::params![id],
        |row| Ok(EvidenceItem {
            created_seq: row.get(0)?, id: row.get(1)?, session_id: row.get(2)?, action_id: row.get(3)?, receipt_id: row.get(4)?,
            kind: row.get(5)?, source: row.get(6)?, summary: row.get(7)?, artifact_id: row.get(8)?, checksum: row.get(9)?, created_at: row.get(10)?,
        }),
    ).optional()?)
}

pub fn list_evidence_items(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Vec<EvidenceItem>> {
    let limit = limit.clamp(1, 1000) as i64;
    let mut stmt = conn.prepare(
        "SELECT created_seq,id,session_id,action_id,receipt_id,kind,source,summary,artifact_id,checksum,created_at FROM evidence_items WHERE session_id=?1 ORDER BY created_seq DESC LIMIT ?2"
    )?;
    let items = stmt
        .query_map(rusqlite::params![session_id, limit], |row| {
            Ok(EvidenceItem {
                created_seq: row.get(0)?,
                id: row.get(1)?,
                session_id: row.get(2)?,
                action_id: row.get(3)?,
                receipt_id: row.get(4)?,
                kind: row.get(5)?,
                source: row.get(6)?,
                summary: row.get(7)?,
                artifact_id: row.get(8)?,
                checksum: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(items)
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn is_symlink_or_reparse(path: &Path) -> std::io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    let is_symlink = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        Ok(is_symlink || (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
    }
    #[cfg(not(windows))]
    {
        Ok(is_symlink)
    }
}

fn verify_existing(path: &Path, checksum: &str, data_len: usize) -> std::io::Result<()> {
    if is_symlink_or_reparse(path)? {
        return Err(std::io::Error::other(
            "artifact path is a symlink or reparse point",
        ));
    }
    let bytes = fs::read(path)?;
    if bytes.len() != data_len || hex::encode(Sha256::digest(&bytes)) != checksum {
        return Err(std::io::Error::other(
            "existing artifact content does not match its content hash",
        ));
    }
    Ok(())
}

pub fn store_artifact(storage_dir: &Path, data: &[u8]) -> std::io::Result<(String, PathBuf)> {
    fs::create_dir_all(storage_dir)?;
    let base = storage_dir.canonicalize()?;
    let checksum = hex::encode(Sha256::digest(data));
    let prefix = checksum
        .get(0..2)
        .ok_or_else(|| std::io::Error::other("checksum too short"))?;
    let dir = storage_dir.join(prefix);
    if dir.exists() && is_symlink_or_reparse(&dir)? {
        return Err(std::io::Error::other("artifact prefix is unsafe"));
    }
    fs::create_dir_all(&dir)?;
    let dir_canon = dir.canonicalize()?;
    if !dir_canon.starts_with(&base) {
        return Err(std::io::Error::other("artifact directory escaped store"));
    }
    let final_path = dir.join(&checksum);
    if final_path.exists() {
        verify_existing(&final_path, &checksum, data.len())?;
        return Ok((checksum, final_path));
    }
    let temp = dir.join(format!("{}.{}.tmp", checksum, Uuid::new_v4()));
    fs::write(&temp, data)?;
    match fs::rename(&temp, &final_path) {
        Ok(()) => {}
        Err(_) if final_path.exists() => {
            let _ = fs::remove_file(&temp);
            verify_existing(&final_path, &checksum, data.len())?;
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    }
    let final_canon = final_path.canonicalize()?;
    if !final_canon.starts_with(&base) {
        let _ = fs::remove_file(&final_path);
        return Err(std::io::Error::other("artifact escaped store"));
    }
    verify_existing(&final_path, &checksum, data.len())?;
    Ok((checksum, final_path))
}

fn validate_store_key(key: &str) -> Result<PathBuf> {
    let path = Path::new(key);
    if path.is_absolute() || path.as_os_str().is_empty() {
        return Err(EvidenceError::InvalidStoreKey);
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EvidenceError::InvalidStoreKey);
    }
    Ok(path.to_path_buf())
}

fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
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

fn persist_bytes(
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
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO artifacts(id,receipt_id,store_key,checksum,size,media_type,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![id,receipt_id,store_key,checksum,data.len() as i64,media_type,now],
    )?;
    get_artifact(conn, &id)?.ok_or(EvidenceError::NotFound(id))
}

pub fn persist_text_artifact(
    conn: &Connection,
    storage_dir: &Path,
    receipt_id: &str,
    text: &str,
    media_type: Option<&str>,
) -> Result<ArtifactRecord> {
    let redacted = redaction::redact_text(text);
    persist_bytes(
        conn,
        storage_dir,
        receipt_id,
        redacted.as_bytes(),
        media_type.or(Some("text/plain; charset=utf-8")),
    )
}

pub fn persist_binary_attachment(
    conn: &Connection,
    storage_dir: &Path,
    receipt_id: &str,
    data: &[u8],
    media_type: Option<&str>,
) -> Result<ArtifactRecord> {
    persist_bytes(conn, storage_dir, receipt_id, data, media_type)
}

pub fn get_artifact(conn: &Connection, id: &str) -> Result<Option<ArtifactRecord>> {
    Ok(conn.query_row(
        "SELECT created_seq,id,receipt_id,store_key,checksum,size,media_type,created_at FROM artifacts WHERE id=?1",
        rusqlite::params![id], record_from_row,
    ).optional()?)
}

pub fn list_artifacts_for_receipt(
    conn: &Connection,
    receipt_id: &str,
) -> Result<Vec<ArtifactRecord>> {
    let mut stmt = conn.prepare("SELECT created_seq,id,receipt_id,store_key,checksum,size,media_type,created_at FROM artifacts WHERE receipt_id=?1 ORDER BY created_seq ASC")?;
    let records = stmt
        .query_map(rusqlite::params![receipt_id], record_from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn read_artifact(conn: &Connection, storage_dir: &Path, id: &str) -> Result<Vec<u8>> {
    let artifact = get_artifact(conn, id)?.ok_or_else(|| EvidenceError::NotFound(id.to_owned()))?;
    let relative = validate_store_key(&artifact.store_key)?;
    let base = storage_dir.canonicalize()?;
    let path = storage_dir.join(relative);
    let candidate = path.canonicalize()?;
    if !candidate.starts_with(base) || is_symlink_or_reparse(&candidate)? {
        return Err(EvidenceError::InvalidStoreKey);
    }
    let bytes = fs::read(candidate)?;
    if bytes.len() as i64 != artifact.size
        || hex::encode(Sha256::digest(&bytes)) != artifact.checksum
    {
        return Err(EvidenceError::Integrity(
            "stored bytes no longer match metadata".into(),
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn content_store_is_idempotent() {
        let dir = tempdir().unwrap();
        let a = store_artifact(dir.path(), b"same").unwrap();
        let b = store_artifact(dir.path(), b"same").unwrap();
        assert_eq!(a, b);
    }
    #[test]
    fn invalid_store_keys_are_rejected() {
        assert!(validate_store_key("../outside").is_err());
        assert!(validate_store_key("/absolute").is_err());
    }
}
