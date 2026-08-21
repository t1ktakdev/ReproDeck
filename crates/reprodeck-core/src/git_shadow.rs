use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum GitShadowError {
    #[error("git failed: {command}: {stderr}")]
    GitFailed { command: String, stderr: String },
    #[error("git output is not valid UTF-8: {0}")]
    GitOutputNotUtf8(String),
    #[error("repository has no commits: {0}")]
    UnbornRepository(String),
    #[error("shadow patch cannot be applied safely: {0}")]
    PatchApplyFailed(String),
    #[error("submodule/gitlink changes are not supported")]
    SubmoduleNotSupported,
    #[error("symbolic-link changes are not supported by Apply")]
    SymlinkNotSupported,
    #[error("unsafe patch target path: {0}")]
    UnsafePatchPath(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("apply succeeded but shadow cleanup failed; recovery reference: {0}")]
    AppliedCleanupPending(String),
}

pub type Result<T> = std::result::Result<T, GitShadowError>;

fn git_output(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(GitShadowError::GitFailed {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

fn git_text(cwd: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(git_output(cwd, args)?)
        .map(|s| s.trim().to_owned())
        .map_err(|_| GitShadowError::GitOutputNotUtf8(format!("git {}", args.join(" "))))
}

fn git_with_stdin(cwd: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(input)?;
    }
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(GitShadowError::GitFailed {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

fn contains_mode(patch: &[u8], mode: &[u8]) -> bool {
    patch.windows(mode.len()).any(|window| window == mode)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Shadow {
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub original_head: String,
    pub original_branch: String,
}

impl Shadow {
    pub fn create(repo: &Path, base_commit: Option<&str>) -> Result<Self> {
        let repo_root = PathBuf::from(git_text(repo, &["rev-parse", "--show-toplevel"])?);
        let original_head = git_text(&repo_root, &["rev-parse", "--verify", "HEAD"])
            .map_err(|_| GitShadowError::UnbornRepository(repo_root.display().to_string()))?;
        let original_branch = git_text(&repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        let base = base_commit.unwrap_or(&original_head).to_owned();
        // Validate base before creating resources.
        let _ = git_text(
            &repo_root,
            &["rev-parse", "--verify", &format!("{}^{{commit}}", base)],
        )?;

        let worktree = std::env::temp_dir().join(format!("reprodeck-shadow-{}", Uuid::new_v4()));
        let branch = format!("reprodeck-shadow-{}", Uuid::new_v4());
        let out = Command::new("git")
            .current_dir(&repo_root)
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg(&branch)
            .arg(&worktree)
            .arg(&base)
            .output()?;
        if !out.status.success() {
            let _ = fs::remove_dir_all(&worktree);
            return Err(GitShadowError::GitFailed {
                command: "git worktree add".into(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }

        Ok(Self {
            repo: repo_root,
            worktree,
            branch,
            base_commit: base,
            original_head,
            original_branch,
        })
    }

    pub fn commit_all(&self, message: &str) -> Result<String> {
        git_output(&self.worktree, &["add", "-A"])?;
        let out = Command::new("git")
            .current_dir(&self.worktree)
            .args([
                "-c",
                "user.name=ReproDeck",
                "-c",
                "user.email=local@reprodeck.invalid",
                "commit",
                "--no-gpg-sign",
                "-m",
                message,
            ])
            .output()?;
        if !out.status.success() {
            return Err(GitShadowError::GitFailed {
                command: "git commit".into(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }
        git_text(
            &self.repo,
            &["rev-parse", &format!("refs/heads/{}", self.branch)],
        )
    }

    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        Ok(!git_output(&self.worktree, &["status", "--porcelain=v1", "-z"])?.is_empty())
    }

    pub fn branch_head(&self) -> Result<String> {
        git_text(
            &self.repo,
            &["rev-parse", &format!("refs/heads/{}", self.branch)],
        )
    }

    pub fn source_head(&self) -> Result<String> {
        git_text(&self.repo, &["rev-parse", "HEAD"])
    }

    pub fn source_state_bytes(&self) -> Result<Vec<u8>> {
        git_output(&self.repo, &["status", "--porcelain=v1", "-z"])
    }

    pub fn diff_name_status_bytes(&self) -> Result<Vec<u8>> {
        git_output(
            &self.repo,
            &[
                "diff",
                "-z",
                "--name-status",
                "--find-renames",
                &format!("{}..{}", self.base_commit, self.branch),
            ],
        )
    }

    /// Human-readable diff summary; never used for applying changes.
    pub fn diff_name_status(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.diff_name_status_bytes()?).into_owned())
    }

    pub fn prepare_patch_bytes(&self) -> Result<Vec<u8>> {
        self.validate_patch_targets()?;
        let patch = git_output(
            &self.repo,
            &[
                "diff",
                "--binary",
                "--full-index",
                "--find-renames",
                &format!("{}..{}", self.base_commit, self.branch),
            ],
        )?;
        if contains_mode(&patch, b"mode 160000") {
            return Err(GitShadowError::SubmoduleNotSupported);
        }
        if contains_mode(&patch, b"mode 120000") {
            return Err(GitShadowError::SymlinkNotSupported);
        }
        Ok(patch)
    }

    fn validate_patch_targets(&self) -> Result<()> {
        let bytes = self.diff_name_status_bytes()?;
        let parts = bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let root = self.repo.canonicalize()?;
        let mut index = 0usize;
        while index < parts.len() {
            let status = std::str::from_utf8(parts[index]).map_err(|_| {
                GitShadowError::UnsafePatchPath("non-UTF-8 Git status record".into())
            })?;
            index += 1;
            let path_count = if status.starts_with('R') || status.starts_with('C') {
                2
            } else {
                1
            };
            for _ in 0..path_count {
                let raw = parts.get(index).ok_or_else(|| {
                    GitShadowError::UnsafePatchPath("incomplete Git path record".into())
                })?;
                index += 1;
                let value = std::str::from_utf8(raw)
                    .map_err(|_| GitShadowError::UnsafePatchPath("non-UTF-8 patch path".into()))?;
                let relative = Path::new(value);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(GitShadowError::UnsafePatchPath(value.to_owned()));
                }
                let mut current = root.clone();
                for component in relative.components() {
                    let std::path::Component::Normal(component) = component else {
                        return Err(GitShadowError::UnsafePatchPath(value.to_owned()));
                    };
                    current.push(component);
                    let Ok(metadata) = fs::symlink_metadata(&current) else {
                        continue;
                    };
                    let is_reparse = {
                        #[cfg(windows)]
                        {
                            use std::os::windows::fs::MetadataExt;
                            metadata.file_attributes() & 0x400 != 0
                        }
                        #[cfg(not(windows))]
                        {
                            false
                        }
                    };
                    if metadata.file_type().is_symlink() || is_reparse {
                        return Err(GitShadowError::UnsafePatchPath(value.to_owned()));
                    }
                    if metadata.is_dir() && !current.canonicalize()?.starts_with(&root) {
                        return Err(GitShadowError::UnsafePatchPath(value.to_owned()));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn prepare_patch(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.prepare_patch_bytes()?).into_owned())
    }

    pub fn check_patch_against_worktree(&self, patch: &[u8]) -> Result<()> {
        git_with_stdin(
            &self.worktree,
            &["apply", "--check", "--binary", "--whitespace=nowarn", "-"],
            patch,
        )?;
        Ok(())
    }

    pub fn apply_patch_to_worktree(&self, patch: &[u8]) -> Result<()> {
        self.check_patch_against_worktree(patch)?;
        git_with_stdin(
            &self.worktree,
            &["apply", "--binary", "--whitespace=nowarn", "-"],
            patch,
        )?;
        Ok(())
    }

    /// Apply the shadow diff to the original working tree without touching the
    /// index and without creating a commit. `git apply` is deliberately used as
    /// the mutation primitive: by default it applies atomically and refuses the
    /// whole patch when any hunk cannot be applied. We never use `--reject` or
    /// `--unsafe-paths`.
    pub fn apply(self) -> Result<()> {
        self.apply_with_expected_identity(None, None)
    }

    /// Apply only when the exact binary patch still has the previously verified
    /// identity. This comparison happens inside the mutation primitive so the
    /// caller cannot accidentally verify one diff and apply another one.
    pub fn apply_verified(
        self,
        expected_patch_sha256: &str,
        expected_source_state_sha256: &str,
    ) -> Result<()> {
        self.apply_with_expected_identity(
            Some(expected_patch_sha256),
            Some(expected_source_state_sha256),
        )
    }

    fn apply_with_expected_identity(
        self,
        expected_patch_sha256: Option<&str>,
        expected_source_state_sha256: Option<&str>,
    ) -> Result<()> {
        if !self.repo.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "original repository no longer exists",
            )
            .into());
        }
        let current_head = git_text(&self.repo, &["rev-parse", "HEAD"])?;
        if current_head != self.original_head {
            return Err(GitShadowError::PatchApplyFailed(
                "original HEAD moved after the shadow workspace was created".into(),
            ));
        }
        if let Some(expected) = expected_source_state_sha256 {
            let actual = hex::encode(Sha256::digest(self.source_state_bytes()?));
            if actual != expected {
                return Err(GitShadowError::PatchApplyFailed(
                    "the source working tree or index changed after verification".into(),
                ));
            }
        }

        let patch = self.prepare_patch_bytes()?;
        if let Some(expected) = expected_patch_sha256 {
            let actual = hex::encode(Sha256::digest(&patch));
            if actual != expected {
                return Err(GitShadowError::PatchApplyFailed(
                    "the current patch bytes do not match the verified patch identity".into(),
                ));
            }
        }
        if patch.is_empty() {
            self.discard()?;
            return Ok(());
        }

        // Preflight against the current working tree. This catches user edits
        // that overlap with the shadow patch and prevents silent overwrite.
        if let Err(error) = git_with_stdin(
            &self.repo,
            &["apply", "--check", "--binary", "--whitespace=nowarn", "-"],
            &patch,
        ) {
            return Err(GitShadowError::PatchApplyFailed(error.to_string()));
        }

        // Default git-apply behaviour is atomic when --reject is not used.
        git_with_stdin(
            &self.repo,
            &["apply", "--binary", "--whitespace=nowarn", "-"],
            &patch,
        )
        .map_err(|error| GitShadowError::PatchApplyFailed(error.to_string()))?;

        let after_head = git_text(&self.repo, &["rev-parse", "HEAD"])?;
        if after_head != self.original_head {
            return Err(GitShadowError::PatchApplyFailed(
                "unexpected HEAD change after apply".into(),
            ));
        }

        if let Err(cleanup_error) = self.discard() {
            let recovery_reference = match crate::recovery::create_pending(
                &self.repo,
                &self.base_commit,
                &self.worktree,
                &self.branch,
            ) {
                Ok(id) => {
                    let _ = crate::recovery::mark_state(
                        &id,
                        &crate::recovery::ShadowState::AppliedCleanupPending,
                        Some(cleanup_error.to_string()),
                    );
                    id
                }
                Err(storage_error) => {
                    let marker = std::env::temp_dir()
                        .join(format!("reprodeck-recovery-{}.txt", Uuid::new_v4()));
                    let message = format!(
                        "Apply succeeded; cleanup is pending.\nrepo={}\nworktree={}\nbranch={}\nerror={}\nstorage_error={}\n",
                        self.repo.display(), self.worktree.display(), self.branch, cleanup_error, storage_error
                    );
                    fs::write(&marker, message)?;
                    marker.display().to_string()
                }
            };
            return Err(GitShadowError::AppliedCleanupPending(recovery_reference));
        }
        Ok(())
    }

    pub fn discard(&self) -> Result<()> {
        if self.worktree.exists() {
            let out = Command::new("git")
                .current_dir(&self.repo)
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(&self.worktree)
                .output()?;
            if !out.status.success() && self.worktree.exists() {
                return Err(GitShadowError::GitFailed {
                    command: "git worktree remove --force".into(),
                    stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
                });
            }
        }
        let _ = Command::new("git")
            .current_dir(&self.repo)
            .args(["worktree", "prune"])
            .output();
        let reference = format!("refs/heads/{}", self.branch);
        let exists = Command::new("git")
            .current_dir(&self.repo)
            .args(["show-ref", "--verify", "--quiet", &reference])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if exists {
            git_output(&self.repo, &["branch", "-D", &self.branch])?;
        }
        if self.worktree.exists() {
            fs::remove_dir_all(&self.worktree)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo(path: &Path) {
        git_output(path, &["init"]).unwrap();
        git_output(path, &["config", "user.name", "Tester"]).unwrap();
        git_output(path, &["config", "user.email", "tester@example.invalid"]).unwrap();
        // Test repositories must not inherit the host's global core.autocrlf.
        // Otherwise Windows can rewrite LF fixtures to CRLF during worktree
        // checkout/apply and turn a Git-safety test into a machine-config test.
        git_output(path, &["config", "core.autocrlf", "false"]).unwrap();
        fs::write(path.join("a.txt"), "one\n").unwrap();
        git_output(path, &["add", "-A"]).unwrap();
        git_output(path, &["commit", "-m", "initial"]).unwrap();
    }

    #[test]
    fn original_is_untouched_until_apply() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let shadow = Shadow::create(dir.path(), None).unwrap();
        fs::write(shadow.worktree.join("a.txt"), "two\n").unwrap();
        shadow.commit_all("change").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "one\n"
        );
        shadow.apply().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "two\n"
        );
    }

    #[test]
    fn conflicting_local_edit_blocks_apply() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let shadow = Shadow::create(dir.path(), None).unwrap();
        fs::write(shadow.worktree.join("a.txt"), "shadow\n").unwrap();
        shadow.commit_all("change").unwrap();
        fs::write(dir.path().join("a.txt"), "local\n").unwrap();
        assert!(shadow.apply().is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "local\n"
        );
    }

    #[test]
    fn index_is_not_touched_by_apply() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("staged.txt"), "staged\n").unwrap();
        git_output(dir.path(), &["add", "staged.txt"]).unwrap();
        let before = git_output(dir.path(), &["ls-files", "-s"]).unwrap();
        let shadow = Shadow::create(dir.path(), None).unwrap();
        fs::write(shadow.worktree.join("a.txt"), "two\n").unwrap();
        shadow.commit_all("change").unwrap();
        shadow.apply().unwrap();
        assert_eq!(before, git_output(dir.path(), &["ls-files", "-s"]).unwrap());
    }
}
