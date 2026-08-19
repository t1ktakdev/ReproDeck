use git2::{Repository, Status, StatusOptions};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("repository path is not valid UTF-8")]
    NonUtf8Path,
    #[error("repository has no commit at HEAD")]
    UnbornRepository,
    #[error("session not found: {0}")]
    SessionNotFound(String),
}

pub type Result<T> = std::result::Result<T, RepositoryError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryInfo {
    pub id: Option<String>,
    pub path: String,
    pub head_commit: String,
    pub branch: String,
    pub is_dirty: bool,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

pub fn inspect_repository(path: &Path) -> Result<RepositoryInfo> {
    let repository = Repository::discover(path)?;
    let root = repository
        .workdir()
        .ok_or_else(|| git2::Error::from_str("bare repositories are not supported"))?
        .canonicalize()?;
    let root = root.to_str().ok_or(RepositoryError::NonUtf8Path)?.to_owned();
    let head = repository.head().map_err(|error| {
        if error.code() == git2::ErrorCode::UnbornBranch {
            RepositoryError::UnbornRepository
        } else {
            RepositoryError::Git(error)
        }
    })?;
    let head_commit = head
        .target()
        .ok_or(RepositoryError::UnbornRepository)?
        .to_string();
    let branch = if head.is_branch() {
        head.shorthand().unwrap_or("HEAD").to_owned()
    } else {
        "HEAD".to_owned()
    };
    drop(head);

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let statuses = repository.statuses(Some(&mut options))?;
    let is_dirty = statuses.iter().any(|entry| entry.status() != Status::CURRENT);

    Ok(RepositoryInfo {
        id: None,
        path: root,
        head_commit,
        branch,
        is_dirty,
    })
}

pub fn attach_repository_to_session(
    conn: &mut Connection,
    session_id: &str,
    path: &Path,
) -> Result<RepositoryInfo> {
    let mut info = inspect_repository(path)?;
    let session_exists = conn
        .query_row(
            "SELECT 1 FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !session_exists {
        return Err(RepositoryError::SessionNotFound(session_id.to_owned()));
    }

    let tx = conn.transaction()?;
    let existing_id: Option<String> = tx
        .query_row(
            "SELECT id FROM repositories WHERE path = ?1 ORDER BY rowid ASC LIMIT 1",
            rusqlite::params![info.path],
            |row| row.get(0),
        )
        .optional()?;
    let repository_id = existing_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    tx.execute(
        "INSERT INTO repositories(id, path, head_commit) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET path = excluded.path, head_commit = excluded.head_commit",
        rusqlite::params![repository_id, info.path, info.head_commit],
    )?;
    tx.execute(
        "UPDATE sessions SET repo_id = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![repository_id, unix_time_secs()?, session_id],
    )?;
    tx.commit()?;

    info.id = Some(repository_id);
    Ok(info)
}

pub fn get_session_repository(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<RepositoryInfo>> {
    let stored: Option<(String, String)> = conn
        .query_row(
            "SELECT r.id, r.path FROM sessions s JOIN repositories r ON r.id = s.repo_id WHERE s.id = ?1",
            rusqlite::params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((id, path)) = stored else {
        return Ok(None);
    };
    let mut info = inspect_repository(Path::new(&path))?;
    info.id = Some(id);
    Ok(Some(info))
}
