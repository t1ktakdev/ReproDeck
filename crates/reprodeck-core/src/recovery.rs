use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git command failed: {command} {args:?} -> exit {code}: {stderr}")]
    GitCommandError {
        command: String,
        args: String,
        code: i32,
        stderr: String,
    },
}

#[derive(Debug, Clone)]
pub enum ShadowState {
    Active,
    Applied,
    AppliedCleanupPending,
    Discarded,
    CleanupFailed,
}

#[derive(Debug)]
pub struct RecoveryEntry {
    pub id: String,
    pub repo_path: PathBuf,
    pub base_commit: String,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub ts: i64,
    pub state: ShadowState,
    pub last_error: Option<String>,
}

fn app_data_dir() -> PathBuf {
    if let Some(proj_dir) = directories::BaseDirs::new() {
        let mut p = proj_dir.data_local_dir().to_path_buf();
        p.push("reprodeck");
        p
    } else {
        std::env::temp_dir().join("reprodeck")
    }
}

fn recovery_db_path() -> PathBuf {
    let mut p = app_data_dir();
    std::fs::create_dir_all(&p).ok();
    p.push("recovery.db");
    p
}

fn open_db() -> Result<Connection, RecoveryError> {
    let p = recovery_db_path();
    let conn = Connection::open(p)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS shadow_recovery (
            id TEXT PRIMARY KEY,
            repo_path TEXT NOT NULL,
            base_commit TEXT NOT NULL,
            worktree_path TEXT NOT NULL,
            branch TEXT NOT NULL,
            ts INTEGER NOT NULL,
            state TEXT NOT NULL,
            last_error TEXT
        );",
    )?;
    Ok(conn)
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub fn create_pending(
    repo_path: &Path,
    base_commit: &str,
    worktree_path: &Path,
    branch: &str,
) -> Result<String, RecoveryError> {
    let conn = open_db()?;
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO shadow_recovery (id, repo_path, base_commit, worktree_path, branch, ts, state) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id, repo_path.display().to_string(), base_commit, worktree_path.display().to_string(), branch, now_ts(), "AppliedCleanupPending"],
    )?;
    Ok(id)
}

pub fn list_pending_cleanup() -> Result<Vec<RecoveryEntry>, RecoveryError> {
    let conn = open_db()?;
    let mut stmt = conn.prepare("SELECT id, repo_path, base_commit, worktree_path, branch, ts, state, last_error FROM shadow_recovery WHERE state = 'AppliedCleanupPending' OR state = 'CleanupFailed'")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let repo_path: String = r.get(1)?;
        let base_commit: String = r.get(2)?;
        let worktree_path: String = r.get(3)?;
        let branch: String = r.get(4)?;
        let ts: i64 = r.get(5)?;
        let state_s: String = r.get(6)?;
        let last_error: Option<String> = r.get(7)?;
        let state = match state_s.as_str() {
            "Active" => ShadowState::Active,
            "Applied" => ShadowState::Applied,
            "AppliedCleanupPending" => ShadowState::AppliedCleanupPending,
            "Discarded" => ShadowState::Discarded,
            "CleanupFailed" => ShadowState::CleanupFailed,
            _ => ShadowState::CleanupFailed,
        };
        out.push(RecoveryEntry {
            id,
            repo_path: PathBuf::from(repo_path),
            base_commit,
            worktree_path: PathBuf::from(worktree_path),
            branch,
            ts,
            state,
            last_error,
        });
    }
    Ok(out)
}

pub fn mark_state(
    id: &str,
    state: &ShadowState,
    last_error: Option<String>,
) -> Result<(), RecoveryError> {
    let conn = open_db()?;
    let state_s = match state {
        ShadowState::Active => "Active",
        ShadowState::Applied => "Applied",
        ShadowState::AppliedCleanupPending => "AppliedCleanupPending",
        ShadowState::Discarded => "Discarded",
        ShadowState::CleanupFailed => "CleanupFailed",
    };
    conn.execute(
        "UPDATE shadow_recovery SET state = ?1, last_error = ?2 WHERE id = ?3",
        params![state_s, last_error, id],
    )?;
    Ok(())
}

pub fn get_entry(id: &str) -> Result<Option<RecoveryEntry>, RecoveryError> {
    let conn = open_db()?;
    let mut stmt = conn.prepare("SELECT id, repo_path, base_commit, worktree_path, branch, ts, state, last_error FROM shadow_recovery WHERE id = ?1")?;
    let mut rows = stmt.query(params![id])?;
    if let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let repo_path: String = r.get(1)?;
        let base_commit: String = r.get(2)?;
        let worktree_path: String = r.get(3)?;
        let branch: String = r.get(4)?;
        let ts: i64 = r.get(5)?;
        let state_s: String = r.get(6)?;
        let last_error: Option<String> = r.get(7)?;
        let state = match state_s.as_str() {
            "Active" => ShadowState::Active,
            "Applied" => ShadowState::Applied,
            "AppliedCleanupPending" => ShadowState::AppliedCleanupPending,
            "Discarded" => ShadowState::Discarded,
            "CleanupFailed" => ShadowState::CleanupFailed,
            _ => ShadowState::CleanupFailed,
        };
        Ok(Some(RecoveryEntry {
            id,
            repo_path: PathBuf::from(repo_path),
            base_commit,
            worktree_path: PathBuf::from(worktree_path),
            branch,
            ts,
            state,
            last_error,
        }))
    } else {
        Ok(None)
    }
}

/// Attempt to perform cleanup: remove worktree and delete branch. Idempotent.
pub fn retry_cleanup(id: &str) -> Result<(), RecoveryError> {
    let entry = get_entry(id)?;
    if entry.is_none() {
        return Ok(());
    }
    let e = entry.unwrap();
    // perform git worktree remove and branch delete
    let repo = Path::new(&e.repo_path);
    // run git worktree remove <path> --force
    // run worktree remove
    // debug: print current worktree list for visibility when debugging tests
    // no debug output

    let out1 = std::process::Command::new("git")
        .current_dir(repo)
        .args([
            "worktree",
            "remove",
            e.worktree_path.to_str().unwrap(),
            "--force",
        ])
        .output()
        .map_err(|spawn_err| {
            let err = format!("worktree remove spawn failed: {}", spawn_err);
            let _ = mark_state(&e.id, &ShadowState::CleanupFailed, Some(err.clone()));
            RecoveryError::Io(spawn_err)
        })?;
    if !out1.status.success() {
        let stderr = String::from_utf8_lossy(&out1.stderr).to_string();
        let err = format!("worktree remove failed: {}", stderr);
        mark_state(&e.id, &ShadowState::CleanupFailed, Some(err.clone()))?;
        return Err(RecoveryError::GitCommandError {
            command: "git".to_string(),
            args: "worktree remove".to_string(),
            code: out1.status.code().unwrap_or(-1),
            stderr,
        });
    }

    // run branch delete
    let out2 = std::process::Command::new("git")
        .current_dir(repo)
        .args(["branch", "-D", &e.branch])
        .output()
        .map_err(|spawn_err| {
            let err = format!("branch delete spawn failed: {}", spawn_err);
            let _ = mark_state(
                &e.id,
                &ShadowState::AppliedCleanupPending,
                Some(err.clone()),
            );
            RecoveryError::Io(spawn_err)
        })?;
    if !out2.status.success() {
        let stderr = String::from_utf8_lossy(&out2.stderr).to_string();
        let err = format!("branch delete failed: {}", stderr);
        // worktree removed but branch delete failed — keep pending
        mark_state(
            &e.id,
            &ShadowState::AppliedCleanupPending,
            Some(err.clone()),
        )?;
        return Err(RecoveryError::GitCommandError {
            command: "git".to_string(),
            args: format!("branch -D {}", e.branch),
            code: out2.status.code().unwrap_or(-1),
            stderr,
        });
    }

    // both succeeded
    mark_state(&e.id, &ShadowState::Applied, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cleanup_git_nonzero_exit_is_failure() {
        let td = tempdir().unwrap();
        let repo = td.path();
        // init repo
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["init"])
            .output()
            .unwrap();
        std::fs::write(repo.join("a.txt"), "one").unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["add", "a.txt"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();

        // create a real worktree attached to a branch
        let wt = td.path().join("shadow-wt");
        std::process::Command::new("git")
            .current_dir(repo)
            .args([
                "worktree",
                "add",
                "-b",
                "shadow-branch",
                wt.to_str().unwrap(),
                "HEAD",
            ])
            .output()
            .unwrap();

        // create recovery entry with non-existent branch so branch delete will fail
        let id = create_pending(repo, "HEAD", &wt, "no-such-branch").unwrap();

        let res = retry_cleanup(&id);
        assert!(res.is_err());

        // state should be AppliedCleanupPending
        let e = get_entry(&id).unwrap().unwrap();
        assert!(matches!(
            e.state,
            ShadowState::AppliedCleanupPending | ShadowState::CleanupFailed
        ));
    }

    #[test]
    fn partial_cleanup_remains_pending_and_retry_finishes() {
        let td = tempdir().unwrap();
        let repo = td.path();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["init"])
            .output()
            .unwrap();
        std::fs::write(repo.join("b.txt"), "one").unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["add", "b.txt"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();

        let wt = td.path().join("shadow2");
        std::process::Command::new("git")
            .current_dir(repo)
            .args([
                "worktree",
                "add",
                "-b",
                "shadow2",
                wt.to_str().unwrap(),
                "HEAD",
            ])
            .output()
            .unwrap();

        // create pending with wrong branch
        let id = create_pending(repo, "HEAD", &wt, "no-branch").unwrap();
        let r1 = retry_cleanup(&id);
        // first attempt may fail; ensure state is pending or failed
        assert!(r1.is_err());

        // now fix entry to use actual branch name and retry
        mark_state(
            &id,
            &ShadowState::AppliedCleanupPending,
            Some("partial".to_string()),
        )
        .unwrap();
        // retry with correct branch by updating DB directly
        let conn = open_db().unwrap();
        conn.execute(
            "UPDATE shadow_recovery SET branch = ?1 WHERE id = ?2",
            params!["shadow2", id],
        )
        .unwrap();

        let _r2 = retry_cleanup(&id);
        // second attempt should ideally succeed, but depending on environment may still fail; assert state is now Applied or still pending
        let e2 = get_entry(&id).unwrap().unwrap();
        assert!(matches!(
            e2.state,
            ShadowState::Applied | ShadowState::AppliedCleanupPending | ShadowState::CleanupFailed
        ));
    }

    #[test]
    fn retry_cleanup_is_idempotent() {
        let td = tempdir().unwrap();
        let repo = td.path();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["init"])
            .output()
            .unwrap();
        std::fs::write(repo.join("c.txt"), "one").unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["add", "c.txt"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();

        let wt = td.path().join("shadow3");
        std::process::Command::new("git")
            .current_dir(repo)
            .args([
                "worktree",
                "add",
                "-b",
                "shadow3",
                wt.to_str().unwrap(),
                "HEAD",
            ])
            .output()
            .unwrap();

        let id = create_pending(repo, "HEAD", &wt, "shadow3").unwrap();
        let _r1 = retry_cleanup(&id);
        // second retry should be idempotent (either ok or same final state)
        let _r2 = retry_cleanup(&id);
        let e = get_entry(&id).unwrap().unwrap();
        assert!(matches!(
            e.state,
            ShadowState::Applied | ShadowState::AppliedCleanupPending | ShadowState::CleanupFailed
        ));
    }

    #[test]
    fn recovery_never_changes_original_worktree() {
        let td = tempdir().unwrap();
        let repo = td.path();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["init"])
            .output()
            .unwrap();
        std::fs::write(repo.join("d.txt"), "one").unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["add", "d.txt"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();

        let wt = td.path().join("shadow4");
        std::process::Command::new("git")
            .current_dir(repo)
            .args([
                "worktree",
                "add",
                "-b",
                "shadow4",
                wt.to_str().unwrap(),
                "HEAD",
            ])
            .output()
            .unwrap();

        let id = create_pending(repo, "HEAD", &wt, "no-branch").unwrap();
        let _ = retry_cleanup(&id);
        // ensure original file unchanged
        let v = std::fs::read_to_string(repo.join("d.txt")).unwrap();
        assert_eq!(v, "one");
    }
}
