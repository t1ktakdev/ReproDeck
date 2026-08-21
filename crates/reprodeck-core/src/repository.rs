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
    pub changed_files: Vec<String>,
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
    let root_text = root
        .to_str()
        .ok_or(RepositoryError::NonUtf8Path)?
        .to_owned();
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
    let mut changed_files = Vec::new();
    let mut is_dirty = false;
    for entry in statuses.iter() {
        if entry.status() == Status::CURRENT {
            continue;
        }
        is_dirty = true;
        if let Some(path) = entry.path() {
            changed_files.push(path.to_owned());
        }
    }
    changed_files.sort();
    changed_files.dedup();

    Ok(RepositoryInfo {
        id: None,
        path: root_text,
        head_commit,
        branch,
        is_dirty,
        changed_files,
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
            "SELECT 1 FROM sessions WHERE id=?1",
            rusqlite::params![session_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !session_exists {
        return Err(RepositoryError::SessionNotFound(session_id.to_owned()));
    }

    let tx = conn.transaction()?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM repositories WHERE path=?1 ORDER BY rowid ASC LIMIT 1",
            rusqlite::params![info.path],
            |row| row.get(0),
        )
        .optional()?;
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    tx.execute(
        "INSERT INTO repositories(id,path,head_commit) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET path=excluded.path, head_commit=excluded.head_commit",
        rusqlite::params![id, info.path, info.head_commit],
    )?;
    tx.execute(
        "UPDATE sessions SET repo_id=?1, updated_at=?2 WHERE id=?3",
        rusqlite::params![id, unix_time_secs()?, session_id],
    )?;
    tx.commit()?;
    info.id = Some(id);
    Ok(info)
}

pub fn get_session_repository(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<RepositoryInfo>> {
    let stored: Option<(String,String)> = conn.query_row(
        "SELECT r.id,r.path FROM sessions s JOIN repositories r ON r.id=s.repo_id WHERE s.id=?1",
        rusqlite::params![session_id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let Some((id, path)) = stored else {
        return Ok(None);
    };
    let mut info = inspect_repository(Path::new(&path))?;
    info.id = Some(id);
    Ok(Some(info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::init_db, timeline};
    use git2::{IndexAddOption, Signature};
    use tempfile::{tempdir, NamedTempFile};

    fn init_repo(path: &Path) {
        let repository = Repository::init(path).unwrap();
        std::fs::write(path.join("tracked.txt"), "base\n").unwrap();
        let mut index = repository.index().unwrap();
        index
            .add_all(["tracked.txt"], IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let sig = Signature::now("Tests", "tests@reprodeck.local").unwrap();
        repository
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
    }

    #[test]
    fn attach_and_reload() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let db = NamedTempFile::new().unwrap();
        let mut conn = init_db(db.path()).unwrap();
        timeline::create_session(&conn, "s", "Draft", None).unwrap();
        let attached = attach_repository_to_session(&mut conn, "s", dir.path()).unwrap();
        let loaded = get_session_repository(&conn, "s").unwrap().unwrap();
        assert_eq!(attached.path, loaded.path);
        assert_eq!(attached.id, loaded.id);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredRepository {
    pub id: String,
    pub path: String,
    pub stored_head_commit: Option<String>,
    pub current: Option<RepositoryInfo>,
    pub accessible: bool,
}

pub fn list_repositories(conn: &Connection) -> Result<Vec<StoredRepository>> {
    let mut stmt =
        conn.prepare("SELECT id,path,head_commit FROM repositories ORDER BY rowid DESC LIMIT 500")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(
            |(id, path, stored_head_commit)| match inspect_repository(Path::new(&path)) {
                Ok(mut info) => {
                    info.id = Some(id.clone());
                    StoredRepository {
                        id,
                        path,
                        stored_head_commit,
                        current: Some(info),
                        accessible: true,
                    }
                }
                Err(_) => StoredRepository {
                    id,
                    path,
                    stored_head_commit,
                    current: None,
                    accessible: false,
                },
            },
        )
        .collect())
}
