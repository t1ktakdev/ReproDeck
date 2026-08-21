use crate::git_shadow::{GitShadowError, Shadow};
use crate::repository::{self, RepositoryError};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShadowSessionError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Shadow(#[from] GitShadowError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Timeline(#[from] crate::timeline::TimelineError),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("session has no attached repository: {0}")]
    RepositoryNotAttached(String),
    #[error("shadow workspace not found for session: {0}")]
    ShadowNotFound(String),
    #[error("shadow workspace record is stale or unsafe to resume")]
    StaleShadow,
    #[error("shadow workspace has no changes to checkpoint")]
    NoChanges,
    #[error("checkpoint the workspace before verification; uncommitted changes are not proof")]
    UncommittedChanges,
    #[error("the source repository HEAD moved after the workspace was created")]
    SourceCommitChanged,
    #[error("the current patch is empty")]
    EmptyPatch,
    #[error("changes were applied, but the local shadow record could not be cleared")]
    AppliedStateCleanupFailed,
    #[error("workspace was discarded, but the local shadow record could not be cleared")]
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
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShadowDiff {
    pub patch: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchIdentity {
    pub source_commit: String,
    pub source_state_sha256: String,
    pub shadow_commit: String,
    pub patch_sha256: String,
    pub patch_size: u64,
    pub files: Vec<String>,
}

fn git_output(cwd: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
}

fn shadow_dirty(path: &Path) -> bool {
    git_output(path, &["status", "--porcelain=v1", "-z"])
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

fn row(conn: &Connection, session_id: &str) -> Result<Option<ShadowWorkspaceRecord>> {
    let raw: Option<(String,String,String,String,String)> = conn.query_row(
        "SELECT sw.repo_id,r.path,sw.base_commit,sw.branch,sw.worktree_path FROM shadow_workspaces sw JOIN repositories r ON r.id=sw.repo_id WHERE sw.id=?1",
        rusqlite::params![session_id],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional()?;
    let Some((repo_id, repo_path, base_commit, branch, worktree_path)) = raw else {
        return Ok(None);
    };
    let worktree = PathBuf::from(&worktree_path);
    let repo_path_ref = Path::new(&repo_path);

    // Temporary directories can disappear after a reboot or cleanup tool while
    // the shadow branch is still intact. Rebuild that worktree from the recorded
    // branch instead of treating the session as lost. No original working-tree
    // files are changed by this recovery.
    if !worktree.exists() {
        let _ = std::process::Command::new("git")
            .current_dir(repo_path_ref)
            .args(["worktree", "prune"])
            .output();
        let restored = std::process::Command::new("git")
            .current_dir(repo_path_ref)
            .arg("worktree")
            .arg("add")
            .arg(&worktree)
            .arg(&branch)
            .output();
        if !restored
            .as_ref()
            .is_ok_and(|output| output.status.success())
            || !worktree.exists()
        {
            return Err(ShadowSessionError::StaleShadow);
        }
    }
    let original_branch = repository::inspect_repository(repo_path_ref)?.branch;
    Ok(Some(ShadowWorkspaceRecord {
        session_id: session_id.to_owned(),
        repo_id,
        repo_path,
        base_commit,
        branch,
        worktree_path,
        original_branch,
        dirty: shadow_dirty(&worktree),
    }))
}

pub fn get_session_shadow(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<ShadowWorkspaceRecord>> {
    row(conn, session_id)
}

pub fn create_session_shadow(conn: &Connection, session_id: &str) -> Result<ShadowWorkspaceRecord> {
    if let Some(existing) = row(conn, session_id)? {
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
        "INSERT INTO shadow_workspaces(id,repo_id,base_commit,branch,worktree_path) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![session_id,repo_id,shadow.base_commit,shadow.branch,shadow.worktree.to_string_lossy().into_owned()],
    );
    if let Err(error) = insert {
        let _ = shadow.discard();
        return Err(error.into());
    }
    row(conn, session_id)?.ok_or(ShadowSessionError::StaleShadow)
}

fn restored(record: &ShadowWorkspaceRecord) -> Shadow {
    Shadow {
        repo: PathBuf::from(&record.repo_path),
        worktree: PathBuf::from(&record.worktree_path),
        branch: record.branch.clone(),
        base_commit: record.base_commit.clone(),
        original_head: record.base_commit.clone(),
        original_branch: record.original_branch.clone(),
    }
}

pub fn finalize_session_shadow(conn: &Connection, session_id: &str) -> Result<String> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restored(&record);
    if !shadow.has_uncommitted_changes()? {
        return Err(ShadowSessionError::NoChanges);
    }
    Ok(shadow.commit_all("ReproDeck shadow checkpoint")?)
}

pub fn session_shadow_diff(conn: &Connection, session_id: &str) -> Result<ShadowDiff> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restored(&record);
    let mut files = Vec::new();
    let bytes = shadow.diff_name_status_bytes()?;
    let parts: Vec<&[u8]> = bytes.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    let mut index = 0usize;
    while index < parts.len() {
        let status = String::from_utf8_lossy(parts[index]);
        index += 1;
        let path_count = if status.starts_with('R') || status.starts_with('C') {
            2
        } else {
            1
        };
        for _ in 0..path_count {
            if index < parts.len() {
                files.push(String::from_utf8_lossy(parts[index]).into_owned());
                index += 1;
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(ShadowDiff {
        patch: shadow.prepare_patch()?,
        files,
    })
}

pub fn current_patch_identity(conn: &Connection, session_id: &str) -> Result<PatchIdentity> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restored(&record);
    if shadow.has_uncommitted_changes()? {
        return Err(ShadowSessionError::UncommittedChanges);
    }
    if shadow.source_head()? != record.base_commit {
        return Err(ShadowSessionError::SourceCommitChanged);
    }
    let patch = shadow.prepare_patch_bytes()?;
    if patch.is_empty() {
        return Err(ShadowSessionError::EmptyPatch);
    }
    let diff = session_shadow_diff(conn, session_id)?;
    Ok(PatchIdentity {
        source_commit: record.base_commit,
        source_state_sha256: hex::encode(Sha256::digest(shadow.source_state_bytes()?)),
        shadow_commit: shadow.branch_head()?,
        patch_sha256: hex::encode(Sha256::digest(&patch)),
        patch_size: patch.len() as u64,
        files: diff.files,
    })
}

pub fn check_patch_against_session(
    conn: &Connection,
    session_id: &str,
    patch: &[u8],
) -> Result<()> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restored(&record);
    if shadow.has_uncommitted_changes()? || !shadow.prepare_patch_bytes()?.is_empty() {
        return Err(ShadowSessionError::UncommittedChanges);
    }
    shadow.check_patch_against_worktree(patch)?;
    Ok(())
}

pub fn apply_patch_and_checkpoint(
    conn: &Connection,
    session_id: &str,
    patch: &[u8],
) -> Result<PatchIdentity> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restored(&record);
    if shadow.has_uncommitted_changes()? || !shadow.prepare_patch_bytes()?.is_empty() {
        return Err(ShadowSessionError::UncommittedChanges);
    }
    shadow.apply_patch_to_worktree(patch)?;
    shadow.commit_all("ReproDeck transferred investigation patch")?;
    current_patch_identity(conn, session_id)
}

pub fn apply_session_shadow(conn: &Connection, session_id: &str) -> Result<()> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    restored(&record).apply()?;
    conn.execute(
        "DELETE FROM shadow_workspaces WHERE id=?1",
        rusqlite::params![session_id],
    )
    .map_err(|_| ShadowSessionError::AppliedStateCleanupFailed)?;
    Ok(())
}

pub fn apply_verified_session_shadow(
    conn: &Connection,
    session_id: &str,
    expected_patch_sha256: &str,
    expected_source_state_sha256: &str,
) -> Result<()> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    let shadow = restored(&record);
    if shadow.has_uncommitted_changes()? {
        return Err(ShadowSessionError::UncommittedChanges);
    }
    shadow.apply_verified(expected_patch_sha256, expected_source_state_sha256)?;
    conn.execute(
        "DELETE FROM shadow_workspaces WHERE id=?1",
        rusqlite::params![session_id],
    )
    .map_err(|_| ShadowSessionError::AppliedStateCleanupFailed)?;
    Ok(())
}

pub fn discard_session_shadow(conn: &Connection, session_id: &str) -> Result<()> {
    let record = row(conn, session_id)?
        .ok_or_else(|| ShadowSessionError::ShadowNotFound(session_id.to_owned()))?;
    restored(&record).discard()?;
    conn.execute(
        "DELETE FROM shadow_workspaces WHERE id=?1",
        rusqlite::params![session_id],
    )
    .map_err(|_| ShadowSessionError::DiscardedStateCleanupFailed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::init_db, state_machine, workflow};
    use tempfile::{tempdir, NamedTempFile};

    fn git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn setup() -> (
        tempfile::TempDir,
        NamedTempFile,
        Connection,
        ShadowWorkspaceRecord,
    ) {
        let repo_dir = tempdir().unwrap();
        git(repo_dir.path(), &["init"]);
        git(repo_dir.path(), &["config", "user.name", "ReproDeck Tests"]);
        git(
            repo_dir.path(),
            &["config", "user.email", "tests@reprodeck.invalid"],
        );
        // Keep fixture bytes deterministic across developer machines.
        git(repo_dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(repo_dir.path().join("tracked.txt"), "base\n").unwrap();
        git(repo_dir.path(), &["add", "tracked.txt"]);
        git(repo_dir.path(), &["commit", "-m", "initial"]);

        let db_file = NamedTempFile::new().unwrap();
        let mut conn = init_db(db_file.path()).unwrap();
        workflow::create_bug_session(&conn, "session", &workflow::SessionMeta::default()).unwrap();
        repository::attach_repository_to_session(&mut conn, "session", repo_dir.path()).unwrap();
        state_machine::transition_session(&conn, "session", state_machine::SessionState::Preparing)
            .unwrap();
        state_machine::transition_session(
            &conn,
            "session",
            state_machine::SessionState::CreatingWorkspace,
        )
        .unwrap();
        let record = create_session_shadow(&conn, "session").unwrap();
        state_machine::transition_session(&conn, "session", state_machine::SessionState::Ready)
            .unwrap();
        (repo_dir, db_file, conn, record)
    }

    #[test]
    fn missing_temporary_worktree_is_recreated_from_shadow_branch() {
        let (_repo_dir, _db_file, conn, record) = setup();
        std::fs::remove_dir_all(&record.worktree_path).unwrap();
        assert!(!Path::new(&record.worktree_path).exists());

        let recovered = get_session_shadow(&conn, "session").unwrap().unwrap();
        assert_eq!(recovered.branch, record.branch);
        assert!(Path::new(&recovered.worktree_path).exists());
        assert_eq!(
            std::fs::read_to_string(Path::new(&recovered.worktree_path).join("tracked.txt"))
                .unwrap(),
            "base\n"
        );

        discard_session_shadow(&conn, "session").unwrap();
        assert!(get_session_shadow(&conn, "session").unwrap().is_none());
    }

    #[test]
    fn original_remains_unchanged_while_shadow_is_dirty() {
        let (repo_dir, _db_file, conn, record) = setup();
        std::fs::write(
            Path::new(&record.worktree_path).join("tracked.txt"),
            "changed\n",
        )
        .unwrap();
        let refreshed = get_session_shadow(&conn, "session").unwrap().unwrap();
        assert!(refreshed.dirty);
        assert_eq!(
            std::fs::read_to_string(repo_dir.path().join("tracked.txt")).unwrap(),
            "base\n"
        );
        discard_session_shadow(&conn, "session").unwrap();
    }
}
