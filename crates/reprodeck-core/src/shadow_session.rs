use crate::git_shadow::{GitShadowError, Shadow};
use crate::repository::{self, RepositoryError};
use crate::timeline::TimelineError;
use git2::{Delta, DiffFindOptions, DiffOptions, IndexAddOption, Repository, Signature};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(not(test))]
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShadowSessionError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    Shadow(#[from] GitShadowError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("session has no attached repository: {0}")]
    RepositoryNotAttached(String),
    #[error("shadow workspace not found for session: {0}")]
    ShadowNotFound(String),
    #[error("shadow workspace record is stale or unsafe to resume")]
    StaleShadow,
    #[error("shadow workspace has no changes to finalize")]
    NoChanges,
    #[error("shadow workspace was applied, but its database record could not be removed")]
    AppliedStateCleanupFailed,
    #[error("shadow workspace was discarded, but its database record could not be removed")]
    DiscardedStateCleanupFailed,
}

pub type Result<T> = std::result::Result<T, ShadowSessionError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShadowWorkspaceRecord {
    pub session_id: String,
    pub repo_id: String,
    pub repo_path: String,
    pub base_commit: String,
    pub branch: String,
    pub worktree_path: String,
    pub original_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShadowChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShadowChange {
    pub kind: ShadowChangeKind,
    pub path: String,
    pub old_path: Option<String>,
}

fn path_for_display(path: Option<&Path>) -> String {
    path.map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn record_from_row(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<ShadowWorkspaceRecord>> {
    let row: Option<(String, String, String, String, String)> = conn
        .query_row(
            "SELECT sw.repo_id, r.path, sw.base_commit, sw.branch, sw.worktree_path
             FROM shadow_workspaces sw
             JOIN repositories r ON r.id = sw.repo_id
             WHERE sw.id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((repo_id, repo_path, base_commit, branch, worktree_path)) = row else {
        return Ok(None);
    };
    let original_branch = repository::inspect_repository(Path::new(&repo_path))?.branch;
    Ok(Some(ShadowWorkspaceRecord {
        session_id: session_id.to_owned(),
        repo_id,
        repo_path,
        base_commit,
        branch,
        worktree_path,
        original_branch,
    }))
}

pub fn get_session_shadow(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<ShadowWorkspaceRecord>> {
    record_from_row(conn, session_id)
}

pub fn create_session_shadow(
    conn: &Connection,
    session_id: &str,
) -> Result<ShadowWorkspaceRecord> {
    if let Some(existing) = get_session_shadow(conn, session_id)? {
        return Ok(existing);
    }

    let session = crate::timeline::get_session_record(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::SessionNotFound(session_id.to_owned()))?;
    let repo_id = session
        .repo_id
        .ok_or_else(|| ShadowSessionError::RepositoryNotAttached(session_id.to_owned()))?;
    let attached = repository::get_session_repository(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::RepositoryNotAttached(session_id.to_owned()))?;
    let shadow = Shadow::create(Path::new(&attached.path), None)?;

    let insert = conn.execute(
        "INSERT INTO shadow_workspaces(id, repo_id, base_commit, branch, worktree_path)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            session_id,
            repo_id,
            shadow.base_commit,
            shadow.branch,
            shadow.worktree.to_string_lossy().into_owned()
        ],
    );
    if let Err(error) = insert {
        let _ = shadow.discard();
        return Err(error.into());
    }

    Ok(ShadowWorkspaceRecord {
        session_id: session_id.to_owned(),
        repo_id,
        repo_path: shadow.repo.to_string_lossy().into_owned(),
        base_commit: shadow.base_commit.clone(),
        branch: shadow.branch.clone(),
        worktree_path: shadow.worktree.to_string_lossy().into_owned(),
        original_branch: shadow.original_branch.clone(),
    })
}

fn open_main_repository(record: &ShadowWorkspaceRecord) -> Result<Repository> {
    let worktree_path = Path::new(&record.worktree_path);
    if !worktree_path.exists() {
        return Err(ShadowSessionError::StaleShadow);
    }
    let repository = Repository::open(&record.repo_path)?;
    if repository
        .find_reference(&format!("refs/heads/{}", record.branch))
        .is_err()
    {
        return Err(ShadowSessionError::StaleShadow);
    }
    Ok(repository)
}

pub fn list_session_shadow_changes(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ShadowChange>> {
    let record = get_session_shadow(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let repository = open_main_repository(&record)?;
    let base = repository.find_commit(git2::Oid::from_str(&record.base_commit)?)?;
    let target_oid = repository.refname_to_id(&format!("refs/heads/{}", record.branch))?;
    let target = repository.find_commit(target_oid)?;
    let base_tree = base.tree()?;
    let target_tree = target.tree()?;
    let mut options = DiffOptions::new();
    options.include_typechange(true);
    let mut diff = repository.diff_tree_to_tree(
        Some(&base_tree),
        Some(&target_tree),
        Some(&mut options),
    )?;
    let mut find = DiffFindOptions::new();
    find.renames(true).copies(true);
    diff.find_similar(Some(&mut find))?;

    let mut changes = Vec::new();
    for delta in diff.deltas() {
        let old_path = delta.old_file().path();
        let new_path = delta.new_file().path();
        let change = match delta.status() {
            Delta::Added => ShadowChange {
                kind: ShadowChangeKind::Added,
                path: path_for_display(new_path),
                old_path: None,
            },
            Delta::Modified => ShadowChange {
                kind: ShadowChangeKind::Modified,
                path: path_for_display(new_path),
                old_path: None,
            },
            Delta::Deleted => ShadowChange {
                kind: ShadowChangeKind::Deleted,
                path: path_for_display(old_path),
                old_path: None,
            },
            Delta::Renamed => ShadowChange {
                kind: ShadowChangeKind::Renamed,
                path: path_for_display(new_path),
                old_path: Some(path_for_display(old_path)),
            },
            Delta::Copied => ShadowChange {
                kind: ShadowChangeKind::Copied,
                path: path_for_display(new_path),
                old_path: Some(path_for_display(old_path)),
            },
            Delta::Typechange => ShadowChange {
                kind: ShadowChangeKind::TypeChanged,
                path: path_for_display(new_path),
                old_path: Some(path_for_display(old_path)),
            },
            Delta::Unmodified
            | Delta::Ignored
            | Delta::Untracked
            | Delta::Unreadable
            | Delta::Conflicted => continue,
        };
        changes.push(change);
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(changes)
}

pub fn finalize_session_shadow(conn: &Connection, session_id: &str) -> Result<String> {
    let record = get_session_shadow(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    open_main_repository(&record)?;
    let repository = Repository::open(&record.worktree_path)?;
    let mut index = repository.index()?;
    index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"], None)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repository.find_tree(tree_id)?;
    let parent = repository.head()?.peel_to_commit()?;
    if parent.tree_id() == tree_id {
        return Err(ShadowSessionError::NoChanges);
    }
    let signature = Signature::now("ReproDeck", "local@reprodeck.invalid")?;
    let commit = repository.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "ReproDeck shadow checkpoint",
        &tree,
        &[&parent],
    )?;
    Ok(commit.to_string())
}

#[cfg(not(test))]
fn restore_shadow(record: &ShadowWorkspaceRecord) -> Shadow {
    Shadow {
        repo: PathBuf::from(&record.repo_path),
        worktree: PathBuf::from(&record.worktree_path),
        branch: record.branch.clone(),
        base_commit: record.base_commit.clone(),
        original_head: record.base_commit.clone(),
        original_branch: record.original_branch.clone(),
    }
}

#[cfg(not(test))]
pub fn apply_session_shadow(conn: &Connection, session_id: &str) -> Result<()> {
    let record = get_session_shadow(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    open_main_repository(&record)?;
    let shadow = restore_shadow(&record);
    shadow.apply()?;
    conn.execute(
        "DELETE FROM shadow_workspaces WHERE id = ?1",
        rusqlite::params![session_id],
    )
    .map_err(|_| ShadowSessionError::AppliedStateCleanupFailed)?;
    Ok(())
}

#[cfg(not(test))]
pub fn discard_session_shadow(conn: &Connection, session_id: &str) -> Result<()> {
    let record = get_session_shadow(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restore_shadow(&record);
    shadow.discard()?;
    conn.execute(
        "DELETE FROM shadow_workspaces WHERE id = ?1",
        rusqlite::params![session_id],
    )
    .map_err(|_| ShadowSessionError::DiscardedStateCleanupFailed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::init_db, timeline};
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
        let signature = Signature::now("ReproDeck Tests", "tests@reprodeck.local").unwrap();
        repository
            .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
    }

    fn setup() -> (tempfile::TempDir, NamedTempFile, Connection) {
        let directory = tempdir().unwrap();
        init_repo(directory.path());
        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        timeline::create_session(&conn, "session", "Active", None).unwrap();
        repository::attach_repository_to_session(&mut conn, "session", directory.path()).unwrap();
        (directory, db_file, conn)
    }

    #[test]
    fn create_shadow_keeps_original_worktree_unchanged() {
        let (directory, _db_file, conn) = setup();
        let record = create_session_shadow(&conn, "session").unwrap();
        assert!(Path::new(&record.worktree_path).exists());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("tracked.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(
            get_session_shadow(&conn, "session").unwrap(),
            Some(record)
        );
    }

    #[test]
    fn create_shadow_is_idempotent_for_active_session() {
        let (_directory, _db_file, conn) = setup();
        let first = create_session_shadow(&conn, "session").unwrap();
        let second = create_session_shadow(&conn, "session").unwrap();
        assert_eq!(first, second);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM shadow_workspaces", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn finalize_and_list_changes_use_shadow_branch_only() {
        let (directory, _db_file, conn) = setup();
        let record = create_session_shadow(&conn, "session").unwrap();
        std::fs::write(
            Path::new(&record.worktree_path).join("tracked.txt"),
            "fixed\n",
        )
        .unwrap();
        let commit = finalize_session_shadow(&conn, "session").unwrap();
        assert!(!commit.is_empty());
        let changes = list_session_shadow_changes(&conn, "session").unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ShadowChangeKind::Modified);
        assert_eq!(changes[0].path, "tracked.txt");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("tracked.txt")).unwrap(),
            "base\n"
        );
    }

    #[test]
    fn session_without_repository_cannot_create_shadow() {
        let db_file = NamedTempFile::new().unwrap();
        let conn = init_db(db_file.path()).unwrap();
        timeline::create_session(&conn, "session", "Active", None).unwrap();
        assert!(matches!(
            create_session_shadow(&conn, "session"),
            Err(ShadowSessionError::RepositoryNotAttached(_))
        ));
    }

    #[test]
    fn missing_session_is_rejected() {
        let db_file = NamedTempFile::new().unwrap();
        let conn = init_db(db_file.path()).unwrap();
        assert!(matches!(
            create_session_shadow(&conn, "missing"),
            Err(ShadowSessionError::SessionNotFound(_))
        ));
    }

    #[test]
    fn finalize_without_changes_is_rejected() {
        let (_directory, _db_file, conn) = setup();
        create_session_shadow(&conn, "session").unwrap();
        assert!(matches!(
            finalize_session_shadow(&conn, "session"),
            Err(ShadowSessionError::NoChanges)
        ));
    }
}
