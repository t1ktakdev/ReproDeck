use crate::redaction;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("GitHub CLI (gh) was not found. Install it and restart ReproDeck.")]
    NotInstalled,
    #[error("GitHub CLI is not authenticated. Run `gh auth login` first.")]
    NotAuthenticated,
    #[error("explicit confirmation is required for this network action")]
    ConfirmationRequired,
    #[error("invalid GitHub request: {0}")]
    InvalidRequest(String),
    #[error("the repository has uncommitted changes. Commit the applied fix yourself before creating a pull request.")]
    WorkingTreeDirty,
    #[error("GitHub CLI failed: {0}")]
    CommandFailed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, GitHubError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubStatus {
    pub installed: bool,
    pub authenticated: bool,
    pub version: Option<String>,
    pub account_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubCreatedItem {
    pub url: String,
    pub kind: String,
}

fn run(repo: Option<&Path>, args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("gh");
    command.args(args);
    if let Some(repo) = repo {
        command.current_dir(repo);
    }
    command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GitHubError::NotInstalled
        } else {
            GitHubError::Io(error)
        }
    })
}

pub fn status() -> GitHubStatus {
    let version = run(None, &["--version"])
        .ok()
        .and_then(|output| {
            output.status.success().then(|| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
        })
        .filter(|value| !value.is_empty());
    if version.is_none() {
        return GitHubStatus {
            installed: false,
            authenticated: false,
            version: None,
            account_hint: None,
        };
    }
    let auth = run(None, &["auth", "status", "--active"]).ok();
    let authenticated = auth.as_ref().is_some_and(|output| output.status.success());
    let account_hint = auth.and_then(|output| {
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        combined
            .lines()
            .find(|line| line.to_ascii_lowercase().contains("account"))
            .map(|line| redaction::redact_text(line.trim()))
    });
    GitHubStatus {
        installed: true,
        authenticated,
        version,
        account_hint,
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = redaction::redact_text(&String::from_utf8_lossy(&output.stderr));
        Err(GitHubError::InvalidRequest(if stderr.trim().is_empty() {
            "Git repository check failed".into()
        } else {
            stderr.trim().into()
        }))
    }
}

fn current_branch(repo: &Path) -> Result<String> {
    let output = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if branch.is_empty() {
        return Err(GitHubError::InvalidRequest(
            "a draft pull request requires a checked-out branch (detached HEAD is not supported)"
                .into(),
        ));
    }
    Ok(branch)
}

fn require_clean_worktree(repo: &Path) -> Result<()> {
    let output = git(
        repo,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?;
    if output.stdout.is_empty() {
        Ok(())
    } else {
        Err(GitHubError::WorkingTreeDirty)
    }
}

fn require_ready() -> Result<()> {
    let state = status();
    if !state.installed {
        return Err(GitHubError::NotInstalled);
    }
    if !state.authenticated {
        return Err(GitHubError::NotAuthenticated);
    }
    Ok(())
}

pub fn create_issue(
    repo: &Path,
    title: &str,
    body: &str,
    confirmed: bool,
) -> Result<GitHubCreatedItem> {
    if !confirmed {
        return Err(GitHubError::ConfirmationRequired);
    }
    if title.trim().is_empty() {
        return Err(GitHubError::InvalidRequest(
            "issue title cannot be empty".into(),
        ));
    }
    require_ready()?;
    let output = run(
        Some(repo),
        &["issue", "create", "--title", title.trim(), "--body", body],
    )?;
    if !output.status.success() {
        return Err(GitHubError::CommandFailed(redaction::redact_text(
            &String::from_utf8_lossy(&output.stderr),
        )));
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !url.starts_with("https://") {
        return Err(GitHubError::CommandFailed(
            "gh did not return an issue URL".into(),
        ));
    }
    Ok(GitHubCreatedItem {
        url,
        kind: "issue".into(),
    })
}

pub fn create_draft_pr(
    repo: &Path,
    title: &str,
    body: &str,
    confirmed: bool,
) -> Result<GitHubCreatedItem> {
    if !confirmed {
        return Err(GitHubError::ConfirmationRequired);
    }
    if title.trim().is_empty() {
        return Err(GitHubError::InvalidRequest(
            "pull request title cannot be empty".into(),
        ));
    }
    require_ready()?;
    require_clean_worktree(repo)?;
    let branch = current_branch(repo)?;
    // Supplying --head tells gh to use this existing branch and avoids its interactive
    // fork/push flow. ReproDeck never publishes commits on the user's behalf.
    let output = run(
        Some(repo),
        &[
            "pr",
            "create",
            "--draft",
            "--head",
            &branch,
            "--title",
            title.trim(),
            "--body",
            body,
        ],
    )?;
    if !output.status.success() {
        return Err(GitHubError::CommandFailed(redaction::redact_text(
            &String::from_utf8_lossy(&output.stderr),
        )));
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !url.starts_with("https://") {
        return Err(GitHubError::CommandFailed(
            "gh did not return a pull request URL".into(),
        ));
    }
    Ok(GitHubCreatedItem {
        url,
        kind: "draft_pr".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        for args in [
            vec!["init"],
            vec!["config", "user.name", "ReproDeck Test"],
            vec!["config", "user.email", "reprodeck@example.invalid"],
            vec!["config", "core.autocrlf", "false"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("git");
            assert!(output.status.success());
        }
        fs::write(dir.path().join("tracked.txt"), "base\n").expect("write fixture");
        assert!(Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .status()
            .expect("git add")
            .success());
        assert!(Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .status()
            .expect("git commit")
            .success());
        dir
    }

    #[test]
    fn draft_pr_preflight_requires_clean_worktree() {
        let dir = init_repo();
        assert!(require_clean_worktree(dir.path()).is_ok());
        fs::write(dir.path().join("tracked.txt"), "changed\n").expect("modify fixture");
        assert!(matches!(
            require_clean_worktree(dir.path()),
            Err(GitHubError::WorkingTreeDirty)
        ));
    }

    #[test]
    fn current_branch_is_resolved_without_network() {
        let dir = init_repo();
        let branch = current_branch(dir.path()).expect("branch");
        assert!(!branch.is_empty());
    }
}
