use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;
use uuid::Uuid;

// test-only failure injection moved to per-Shadow instance; no global Lazy/AtomicI32 needed

#[derive(Debug, Error)]
pub enum GitShadowError {
    #[error("git failed: {0} -- {1}")]
    GitFailed(String, String),

    #[error("repository has no commits (unborn) for path {0}")]
    UnbornRepository(String),

    #[error("patch could not be applied cleanly: {0}")]
    PatchApplyFailed(String),

    #[error("submodule/gitlink changes are not supported")]
    SubmoduleNotSupported,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("apply succeeded but cleanup failed; pending cleanup marker at {0}")]
    AppliedCleanupPending(PathBuf),
}

type Result<T> = std::result::Result<T, GitShadowError>;

fn run_git(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(GitShadowError::Io)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        // detect unborn repository when asking rev-parse HEAD
        if args == ["rev-parse", "HEAD"] {
            return Err(GitShadowError::UnbornRepository(stderr));
        }
        Err(GitShadowError::GitFailed(args.join(" "), stderr))
    }
}

#[allow(dead_code)]
fn run_git_with_input(cwd: &Path, args: &[&str], input: &str) -> Result<String> {
    let mut cmd = Command::new("git");
    let mut child = cmd
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(GitShadowError::Io)?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(input.as_bytes())
            .map_err(GitShadowError::Io)?;
    }
    let out = child.wait_with_output().map_err(GitShadowError::Io)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(GitShadowError::GitFailed(
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).to_string(),
        ))
    }
}

fn run_git_bytes(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(GitShadowError::Io)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        Err(GitShadowError::GitFailed(args.join(" "), stderr))
    }
}

#[derive(Debug)]
pub struct Shadow {
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub original_head: String,
    pub original_branch: String,
    #[cfg(test)]
    pub apply_fail_after: std::sync::atomic::AtomicI32,
}

impl Shadow {
    /// Create a new shadow worktree based on `base_commit` (or HEAD if None).
    /// The shadow is implemented using `git worktree add -b <branch> <path> <commit>`.
    pub fn create(repo: &Path, base_commit: Option<&str>) -> Result<Self> {
        // Resolve repository root
        let repo_root = PathBuf::from(run_git(repo, &["rev-parse", "--show-toplevel"])?);

        // ensure repository has an initial commit
        if run_git(&repo_root, &["rev-parse", "--verify", "HEAD"]).is_err() {
            return Err(GitShadowError::UnbornRepository(repo.display().to_string()));
        }
        let original_branch = run_git(&repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        let original_head = run_git(&repo_root, &["rev-parse", "HEAD"])?;

        let base = match base_commit {
            Some(b) => b.to_string(),
            None => original_head.clone(),
        };

        // create a temporary directory for the worktree
        let tmp_dir = std::env::temp_dir().join(format!("reprodeck-shadow-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir)?;

        let branch = format!("reprodeck-shadow-{}", Uuid::new_v4());

        // git worktree add -b <branch> <tmp_dir> <base>
        run_git(
            &repo_root,
            &[
                "worktree",
                "add",
                "-b",
                &branch,
                tmp_dir.to_str().unwrap(),
                &base,
            ],
        )?;

        Ok(Shadow {
            repo: repo_root,
            worktree: tmp_dir,
            branch,
            base_commit: base,
            original_head,
            original_branch,
            #[cfg(test)]
            apply_fail_after: std::sync::atomic::AtomicI32::new(-1),
        })
    }

    /// Commit all changes in the shadow worktree with given message
    pub fn commit_all(&self, message: &str) -> Result<String> {
        run_git(&self.worktree, &["add", "-A"])?;
        run_git(&self.worktree, &["commit", "-m", message])?;
        // return new head of shadow branch
        run_git(
            &self.repo,
            &["rev-parse", &format!("refs/heads/{}", self.branch)],
        )
    }

    #[cfg(test)]
    pub fn set_apply_fail_after(&self, v: i32) {
        self.apply_fail_after
            .store(v, std::sync::atomic::Ordering::SeqCst);
    }

    /// Get name-status diff between original head and shadow branch
    pub fn diff_name_status(&self) -> Result<String> {
        // machine-parsable name-status (NUL-delimited)
        run_git(
            &self.repo,
            &[
                "diff",
                "-z",
                "--name-status",
                &format!("{}..{}", self.base_commit, self.branch),
            ],
        )
    }

    /// Prepare the patch (git diff --binary base..branch)
    pub fn prepare_patch(&self) -> Result<String> {
        let patch = run_git(
            &self.repo,
            &[
                "diff",
                "--binary",
                &format!("{}..{}", self.base_commit, self.branch),
            ],
        )?;

        if patch.contains("new mode 160000")
            || patch.contains("old mode 160000")
            || patch.contains("GITLINK")
        {
            return Err(GitShadowError::SubmoduleNotSupported);
        }

        Ok(patch)
    }

    /// Apply the shadow patch into the original working tree WITHOUT committing.
    /// This will:
    /// - verify the repo still exists and HEAD didn't move since creation
    /// - perform a dry-run check that the patch can be applied cleanly
    /// - apply the patch to the working tree (no commit, no index changes)
    ///
    /// If the patch cannot be applied cleanly, returns an error and does not
    /// mutate the original working tree.
    pub fn apply(self) -> Result<()> {
        // ensure original repo still exists
        if !self.repo.exists() {
            return Err(GitShadowError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "original repository no longer exists",
            )));
        }

        // ensure original hasn't moved
        let current_head = run_git(&self.repo, &["rev-parse", "HEAD"])?;
        if current_head != self.original_head {
            return Err(GitShadowError::GitFailed(
                "HEAD moved".into(),
                "original HEAD changed since shadow creation".into(),
            ));
        }

        // Snapshot index (staged entries) so we can restore it after apply.
        // We capture `git ls-files -s` output (mode sha stage\tpath) and convert it to
        // the format expected by `git update-index --index-info` (mode sha\tpath).
        let index_snapshot_bytes = run_git_bytes(&self.repo, &["ls-files", "-s"]).ok();
        let mut index_info_input: Option<String> = None;
        let mut index_snapshot_raw: Option<String> = None;
        if let Some(b) = index_snapshot_bytes {
            let s = String::from_utf8_lossy(&b).to_string();
            index_snapshot_raw = Some(s.clone());
            let mut lines = Vec::new();
            for line in s.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(tabpos) = line.find('\t') {
                    let left = &line[..tabpos];
                    let path = &line[tabpos + 1..];
                    let mut parts = left.split_whitespace();
                    let mode = parts.next().unwrap_or("");
                    let sha = parts.next().unwrap_or("");
                    lines.push(format!("{} {}\t{}", mode, sha, path));
                }
            }
            if !lines.is_empty() {
                index_info_input = Some(lines.join("\n") + "\n");
            }
        }

        // enumerate name-status to understand operations in a machine-safe way (-z output)
        let name_status_z = run_git(
            &self.repo,
            &[
                "diff",
                "-z",
                "--name-status",
                &format!("{}..{}", self.base_commit, self.branch),
            ],
        )?;

        #[derive(Debug, Clone)]
        enum Change {
            Add(PathBuf),
            Modify(PathBuf),
            Delete(PathBuf),
            Rename(PathBuf, PathBuf),
        }

        let mut changes: Vec<Change> = Vec::new();
        // parse NUL-delimited tokens: status\0path\0  or status\0old\0new\0 for renames
        let mut parts: Vec<&str> = name_status_z.split('\u{0}').collect();
        // last element after trailing NUL may be empty; remove it
        if let Some(last) = parts.last() {
            if last.is_empty() {
                parts.pop();
            }
        }
        let mut i = 0usize;
        while i < parts.len() {
            let status = parts[i];
            i += 1;
            match status.chars().next() {
                Some('A') => {
                    if i < parts.len() {
                        changes.push(Change::Add(PathBuf::from(parts[i])));
                    }
                    i += 1;
                }
                Some('M') => {
                    if i < parts.len() {
                        changes.push(Change::Modify(PathBuf::from(parts[i])));
                    }
                    i += 1;
                }
                Some('D') => {
                    if i < parts.len() {
                        changes.push(Change::Delete(PathBuf::from(parts[i])));
                    }
                    i += 1;
                }
                Some('R') => {
                    // rename uses two paths
                    if i + 1 < parts.len() {
                        let old = PathBuf::from(parts[i]);
                        let new = PathBuf::from(parts[i + 1]);
                        changes.push(Change::Rename(old, new));
                    }
                    i += 2;
                }
                _ => {
                    // unknown status; skip one token to avoid infinite loop
                    i += 1;
                }
            }
        }

        // prepare conflict detection: for every changed path, compare working tree content to base_commit content
        for ch in &changes {
            match ch {
                Change::Add(p) | Change::Modify(p) | Change::Delete(p) => {
                    let base_blob = run_git_bytes(
                        &self.repo,
                        &["show", &format!("{}:{}", self.base_commit, p.display())],
                    )
                    .ok();
                    let work_bytes = std::fs::read(self.repo.join(p)).ok();
                    // If working tree differs from base, and shadow modifies it, that's a conflict
                    if let (Some(b), Some(w)) = (base_blob.as_ref(), work_bytes.as_ref()) {
                        if b != w {
                            return Err(GitShadowError::PatchApplyFailed(format!(
                                "conflict on {}",
                                p.display()
                            )));
                        }
                    }
                    if base_blob.is_none() && work_bytes.is_some() && matches!(ch, Change::Add(_)) {
                        // file exists locally but wasn't in base; treat as conflict
                        return Err(GitShadowError::PatchApplyFailed(format!(
                            "conflict on {} (local addition)",
                            p.display()
                        )));
                    }
                }
                Change::Rename(old, _new) => {
                    let base_blob = run_git_bytes(
                        &self.repo,
                        &["show", &format!("{}:{}", self.base_commit, old.display())],
                    )
                    .ok();
                    let work_bytes = std::fs::read(self.repo.join(old)).ok();
                    if let (Some(b), Some(w)) = (base_blob.as_ref(), work_bytes.as_ref()) {
                        if b != w {
                            return Err(GitShadowError::PatchApplyFailed(format!(
                                "conflict on {} for rename",
                                old.display()
                            )));
                        }
                    }
                }
            }
        }

        // Build an ApplyPlan: prefetch blobs and metadata for atomic-like apply
        #[derive(Debug, Clone)]
        enum Op {
            Write {
                path: PathBuf,
                blob: Vec<u8>,
                executable: bool,
            },
            Delete {
                path: PathBuf,
            },
        }

        let mut plan: Vec<Op> = Vec::new();
        for ch in &changes {
            match ch {
                Change::Add(p) | Change::Modify(p) => {
                    ensure_path_within_repo(&self.repo, p)?;
                    let blob = run_git_bytes(
                        &self.repo,
                        &["show", &format!("{}:{}", self.branch, p.display())],
                    )?;
                    let mut executable = false;
                    if let Ok(ls) =
                        run_git(&self.repo, &["ls-tree", &self.branch, &p.to_string_lossy()])
                    {
                        if ls.starts_with("100755") {
                            executable = true;
                        }
                        if ls.starts_with("160000") {
                            return Err(GitShadowError::SubmoduleNotSupported);
                        }
                    }
                    plan.push(Op::Write {
                        path: p.clone(),
                        blob,
                        executable,
                    });
                }
                Change::Delete(p) => {
                    ensure_path_within_repo(&self.repo, p)?;
                    plan.push(Op::Delete { path: p.clone() });
                }
                Change::Rename(old, new) => {
                    if cfg!(windows)
                        && old.to_string_lossy().to_lowercase()
                            == new.to_string_lossy().to_lowercase()
                        && old != new
                    {
                        return Err(GitShadowError::PatchApplyFailed(format!(
                            "case-only rename unsupported on Windows: {} -> {}",
                            old.display(),
                            new.display()
                        )));
                    }
                    ensure_path_within_repo(&self.repo, old)?;
                    ensure_path_within_repo(&self.repo, new)?;
                    // fetch blob for new path from shadow branch
                    let blob = run_git_bytes(
                        &self.repo,
                        &["show", &format!("{}:{}", self.branch, new.display())],
                    )?;
                    let mut executable = false;
                    if let Ok(ls) = run_git(
                        &self.repo,
                        &["ls-tree", &self.branch, &new.to_string_lossy()],
                    ) {
                        if ls.starts_with("100755") {
                            executable = true;
                        }
                        if ls.starts_with("160000") {
                            return Err(GitShadowError::SubmoduleNotSupported);
                        }
                    }
                    plan.push(Op::Write {
                        path: new.clone(),
                        blob,
                        executable,
                    });
                    plan.push(Op::Delete { path: old.clone() });
                }
            }
        }

        // Validate plan against working tree (conflicts)
        for op in &plan {
            match op {
                Op::Write { path, .. } => {
                    let base_blob = run_git_bytes(
                        &self.repo,
                        &["show", &format!("{}:{}", self.base_commit, path.display())],
                    )
                    .ok();
                    let work_bytes = std::fs::read(self.repo.join(path)).ok();
                    if let (Some(b), Some(w)) = (base_blob.as_ref(), work_bytes.as_ref()) {
                        if b != w {
                            return Err(GitShadowError::PatchApplyFailed(format!(
                                "conflict on {}",
                                path.display()
                            )));
                        }
                    }
                    if base_blob.is_none() && work_bytes.is_some() {
                        return Err(GitShadowError::PatchApplyFailed(format!(
                            "conflict on {} (local addition)",
                            path.display()
                        )));
                    }
                }
                Op::Delete { path } => {
                    let target = self.repo.join(path);
                    if target.exists() && target.is_dir() {
                        return Err(GitShadowError::PatchApplyFailed(format!(
                            "delete would remove directory: {}",
                            path.display()
                        )));
                    }
                }
            }
        }

        // Prepare rollback journal (backups) in tempdir
        let journal_dir = std::env::temp_dir().join(format!("reprodeck-apply-{}", Uuid::new_v4()));
        fs::create_dir_all(&journal_dir)?;
        let mut backups: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        let mut applied_ops: Vec<Op> = Vec::new();

        for (idx, op) in plan.into_iter().enumerate() {
            // backup pre-existing file if any
            match &op {
                Op::Write { path, .. } => {
                    let target = self.repo.join(path);
                    if target.exists() {
                        if target.is_file() {
                            let bp = journal_dir.join(format!("backup-{}", idx));
                            fs::copy(&target, &bp)?;
                            backups.push((path.clone(), Some(bp)));
                        } else {
                            return Err(GitShadowError::PatchApplyFailed(format!(
                                "unexpected non-file at {}",
                                path.display()
                            )));
                        }
                    } else {
                        backups.push((path.clone(), None));
                    }
                }
                Op::Delete { path } => {
                    let target = self.repo.join(path);
                    if target.exists() {
                        if target.is_file() {
                            let bp = journal_dir.join(format!("backup-{}", idx));
                            fs::copy(&target, &bp)?;
                            backups.push((path.clone(), Some(bp)));
                        } else {
                            return Err(GitShadowError::PatchApplyFailed(format!(
                                "refuse to remove non-file {}",
                                path.display()
                            )));
                        }
                    } else {
                        backups.push((path.clone(), None));
                    }
                }
            }

            // test injection (per-Shadow, test-only)
            #[cfg(test)]
            {
                let v = self
                    .apply_fail_after
                    .load(std::sync::atomic::Ordering::SeqCst);
                if v >= 0 && (idx as i32) == v {
                    // simulate failure: rollback from backups
                    for (p, b) in backups.iter().rev() {
                        let targ = self.repo.join(p);
                        if let Some(bp) = b {
                            let _ = fs::copy(bp, &targ);
                        } else {
                            let _ = fs::remove_file(&targ);
                        }
                    }
                    let _ = fs::remove_dir_all(&journal_dir);
                    return Err(GitShadowError::PatchApplyFailed(
                        "injected failure".to_string(),
                    ));
                }
            }

            // apply op
            match &op {
                Op::Write {
                    path,
                    blob,
                    executable,
                } => {
                    let target = self.repo.join(path);
                    if let Some(parent) = target.parent() {
                        // parent_rel is the relative path within the repo for validation
                        if let Some(parent_rel) = path.parent() {
                            // ensure parent exists and still within repo before mutation
                            ensure_path_within_repo(&self.repo, parent_rel)?;
                        }
                        fs::create_dir_all(parent)?;
                        // double-check parent is still within repo after creation
                        if let Some(parent_rel) = path.parent() {
                            ensure_path_within_repo(&self.repo, parent_rel)?;
                        }
                    }

                    // Platform-specific safe write:
                    // - On Unix: use openat with O_NOFOLLOW to avoid symlink races
                    // - On other platforms: perform an additional ensure_path_within_repo check and then write
                    #[cfg(unix)]
                    {
                        use libc::{
                            close, fchmod, mode_t, openat, write as libc_write, O_CREAT,
                            O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_WRONLY,
                        };
                        use std::ffi::CString;
                        use std::os::unix::ffi::OsStrExt;

                        let parent = target.parent().expect("parent exists");
                        // open parent dir FD with O_DIRECTORY|O_RDONLY|O_NOFOLLOW
                        let parent_c = CString::new(parent.as_os_str().as_bytes()).unwrap();
                        let dirfd = unsafe {
                            libc::open(parent_c.as_ptr(), libc::O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
                        };
                        if dirfd < 0 {
                            return Err(GitShadowError::Io(std::io::Error::last_os_error()));
                        }

                        let name = target.file_name().unwrap().to_string_lossy();
                        let name_c = CString::new(name.as_bytes()).unwrap();

                        let fd = unsafe {
                            openat(
                                dirfd,
                                name_c.as_ptr(),
                                O_CREAT | O_EXCL | O_WRONLY,
                                0o644 as mode_t,
                            )
                        };
                        if fd < 0 {
                            unsafe { close(dirfd) };
                            return Err(GitShadowError::Io(std::io::Error::last_os_error()));
                        }

                        // write blob fully
                        let mut written = 0usize;
                        while written < blob.len() {
                            let res = unsafe {
                                libc_write(
                                    fd,
                                    blob[written..].as_ptr() as *const _,
                                    blob.len() - written,
                                )
                            };
                            if res < 0 {
                                unsafe {
                                    close(fd);
                                    close(dirfd)
                                };
                                return Err(GitShadowError::Io(std::io::Error::last_os_error()));
                            }
                            written += res as usize;
                        }

                        if *executable {
                            let r = unsafe { fchmod(fd, 0o755 as mode_t) };
                            if r != 0 {
                                unsafe {
                                    close(fd);
                                    close(dirfd)
                                };
                                return Err(GitShadowError::Io(std::io::Error::last_os_error()));
                            }
                        }

                        unsafe {
                            close(fd);
                            close(dirfd)
                        };
                    }

                    #[cfg(not(unix))]
                    {
                        // conservative fallback: re-check path and then write
                        ensure_path_within_repo(&self.repo, path)?;
                        fs::write(&target, blob)?;
                        if *executable {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let mut perm = fs::metadata(&target)?.permissions();
                                perm.set_mode(0o755);
                                fs::set_permissions(&target, perm)?;
                            }
                        }
                    }
                }
                Op::Delete { path } => {
                    let target = self.repo.join(path);
                    // validate before delete (path is relative)
                    ensure_path_within_repo(&self.repo, path)?;
                    if target.exists() {
                        fs::remove_file(&target)?;
                    }
                }
            }

            applied_ops.push(op);
        }

        // applied successfully; cleanup journal
        let _ = fs::remove_dir_all(&journal_dir);

        // verify HEAD unchanged
        let after_head = run_git(&self.repo, &["rev-parse", "HEAD"])?;
        if after_head != self.original_head {
            return Err(GitShadowError::GitFailed(
                "HEAD changed".into(),
                "unexpected HEAD change after apply".into(),
            ));
        }

        // restore index snapshot (if any) so pre-existing staged entries are preserved
        if let Some(input) = index_info_input {
            // update-index --index-info reads lines of: "<mode> <sha>\t<path>"
            let _ = run_git_with_input(&self.repo, &["update-index", "--index-info"], &input)
                .map_err(|e| GitShadowError::GitFailed("update-index".into(), format!("{}", e)))?;
            // verify index matches snapshot by comparing ls-files -s
            if let Ok(after) = run_git(&self.repo, &["ls-files", "-s"]) {
                let before_raw = index_snapshot_raw.unwrap_or_default();
                if before_raw.trim_end() != after.trim_end() {
                    return Err(GitShadowError::GitFailed(
                        "index_restore_mismatch".into(),
                        format!(
                            "index mismatch after restore\nbefore:\n{}\nafter:\n{}",
                            before_raw, after
                        ),
                    ));
                }
            }
        }

        // attempt cleanup; if cleanup fails, record pending marker and return AppliedCleanupPending
        if let Err(_e) = self.discard() {
            // record recovery state in ReproDeck-managed storage (not in user repo)
            let id = crate::recovery::create_pending(
                &self.repo,
                &self.base_commit,
                &self.worktree,
                &self.branch,
            )
            .map_err(|e| {
                GitShadowError::Io(std::io::Error::other(format!(
                    "recovery store failed: {}",
                    e
                )))
            })?;
            return Err(GitShadowError::AppliedCleanupPending(
                std::path::PathBuf::from(id),
            ));
        }

        Ok(())
    }

    /// Discard shadow (remove worktree and delete branch). If force is true,
    /// force removal of worktree.
    pub fn discard(&self) -> Result<()> {
        // remove worktree
        let wt = self.worktree.to_str().ok_or_else(|| {
            GitShadowError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid worktree path",
            ))
        })?;
        // Inspect `git worktree list` and remove any worktree entries that reference this branch
        if let Ok(list) = run_git(&self.repo, &["worktree", "list"]) {
            for line in list.lines() {
                // line format: <path> <HEAD> [branch]
                if line.contains(&format!("[{}]", self.branch)) || line.contains(wt) {
                    if let Some(path_tok) = line.split_whitespace().next() {
                        let _ = run_git(&self.repo, &["worktree", "remove", path_tok, "--force"]);
                    }
                }
            }
        }

        // Delete branch only if it exists
        if run_git(
            &self.repo,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", self.branch),
            ],
        )
        .is_ok()
        {
            // attempt to delete branch; if it fails because some worktree still references it, try to remove referencing entries and retry once
            if let Err(_e) = run_git(&self.repo, &["branch", "-D", &self.branch]) {
                // try removing any worktree entries that reference this branch and retry
                if let Ok(list) = run_git(&self.repo, &["worktree", "list"]) {
                    for line in list.lines() {
                        if line.contains(&format!("[{}]", self.branch)) {
                            if let Some(path_tok) = line.split_whitespace().next() {
                                let _ = run_git(
                                    &self.repo,
                                    &["worktree", "remove", path_tok, "--force"],
                                );
                            }
                        }
                    }
                }
                // retry delete
                run_git(&self.repo, &["branch", "-D", &self.branch])?;
            }
        }

        // remove filesystem dir if still exists
        if self.worktree.exists() {
            fs::remove_dir_all(&self.worktree)?;
        }
        Ok(())
    }
}

/// Snapshot the working tree content (byte-for-byte) for comparison.
#[cfg(test)]
fn snapshot_working_tree(repo: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(repo)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            // skip .git and .reprodeck metadata directory
            let p = e.path();
            if p.is_dir() && (p.ends_with(".git") || p.ends_with(".reprodeck")) {
                return false;
            }
            true
        })
    {
        let p = entry.path();
        if p.is_file() {
            let rel = p.strip_prefix(repo).unwrap().to_path_buf();
            let data = std::fs::read(p)?;
            out.push((rel, data));
        }
    }
    // sort for deterministic order
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Ensure that a user-provided path (from git diff) is safe to operate on inside `repo`.
fn ensure_path_within_repo(repo: &Path, rel: &Path) -> Result<()> {
    // Reject absolute paths
    if rel.is_absolute() {
        return Err(GitShadowError::PatchApplyFailed(format!(
            "absolute path not allowed: {}",
            rel.display()
        )));
    }

    // Reject parent traversal
    for comp in rel.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(GitShadowError::PatchApplyFailed(format!(
                "parent traversal not allowed: {}",
                rel.display()
            )));
        }
    }

    let repo_canon = repo.canonicalize().map_err(GitShadowError::Io)?;

    // find nearest existing ancestor of repo.join(rel)
    let target = repo.join(rel);
    let mut anc = target.clone();
    while !anc.exists() {
        if !anc.pop() {
            break;
        }
    }
    let anc_canon = anc.canonicalize().map_err(GitShadowError::Io)?;
    if !anc_canon.starts_with(&repo_canon) {
        return Err(GitShadowError::PatchApplyFailed(format!(
            "path escapes repo: {}",
            rel.display()
        )));
    }

    // Walk each component from repo root to the nearest ancestor and ensure no symlink points outside
    let mut p = repo.to_path_buf();
    for comp in rel.components() {
        p.push(comp.as_os_str());
        if p.exists() {
            let md = p.symlink_metadata().map_err(GitShadowError::Io)?;
            if md.file_type().is_symlink() {
                // resolve symlink target
                let link = fs::read_link(&p).map_err(GitShadowError::Io)?;
                let abs = if link.is_absolute() {
                    link
                } else {
                    p.parent().unwrap().join(link)
                };
                let abs_canon = abs.canonicalize().map_err(GitShadowError::Io)?;
                if !abs_canon.starts_with(&repo_canon) {
                    return Err(GitShadowError::PatchApplyFailed(format!(
                        "symlink escapes repo at {}",
                        p.display()
                    )));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{read_to_string, write};
    use tempfile::tempdir;

    fn init_repo_with_file(dir: &Path, filename: &str, content: &str) -> Result<()> {
        run_git(dir, &["init"])?;
        // ensure local commit identity so tests don't depend on global git config
        run_git(dir, &["config", "user.name", "Tester"])?;
        run_git(dir, &["config", "user.email", "tester@example.com"])?;
        write(dir.join(filename), content)?;
        run_git(dir, &["add", filename])?;
        run_git(dir, &["commit", "-m", "initial"])?;
        Ok(())
    }

    #[test]
    fn shadow_does_not_modify_original_until_apply() {
        let td = tempdir().unwrap();
        let repo = td.path();

        init_repo_with_file(repo, "foo.txt", "base").unwrap();

        let shadow = Shadow::create(repo, None).unwrap();

        // modify file in shadow worktree
        let shadow_file = shadow.worktree.join("foo.txt");
        write(&shadow_file, "modified in shadow").unwrap();
        // commit in shadow
        shadow.commit_all("shadow change").unwrap();

        // ensure original file remains unchanged
        let orig = read_to_string(repo.join("foo.txt")).unwrap();
        assert_eq!(orig, "base");

        // check diff reports change (machine-safe NUL-delimited output)
        let diff = shadow.diff_name_status().unwrap();
        let parts: Vec<&str> = diff.split('\u{0}').filter(|s| !s.is_empty()).collect();
        let mut found = false;
        let mut i = 0usize;
        while i + 1 < parts.len() {
            let status = parts[i];
            let path = parts[i + 1];
            if status.starts_with('M') && path == "foo.txt" {
                found = true;
                break;
            }
            i += 2;
        }
        assert!(found, "expected modified foo.txt in diff");

        // ensure original file remains unchanged until apply
        let new_orig = read_to_string(repo.join("foo.txt")).unwrap();
        assert_eq!(new_orig, "base");

        // apply shadow (no commit)
        shadow.apply().unwrap();

        // now original file should be updated in working tree
        let new_orig = read_to_string(repo.join("foo.txt")).unwrap();
        assert_eq!(new_orig, "modified in shadow");
    }

    #[test]
    fn apply_detects_conflict_when_original_changed() {
        let td = tempdir().unwrap();
        let repo = td.path();

        init_repo_with_file(repo, "bar.txt", "base").unwrap();

        let shadow = Shadow::create(repo, None).unwrap();

        // modify file in shadow
        let shadow_file = shadow.worktree.join("bar.txt");
        write(&shadow_file, "changed in shadow").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // now change original and commit
        write(repo.join("bar.txt"), "changed in original").unwrap();
        run_git(repo, &["add", "bar.txt"]).unwrap();
        run_git(
            repo,
            &[
                "commit",
                "-m",
                "orig change",
                "--author=Orig <o@example.com>",
            ],
        )
        .unwrap();

        // applying shadow should fail due to original HEAD mismatch
        let res = shadow.apply();
        assert!(res.is_err());
    }

    #[test]
    fn original_unchanged_before_apply() {
        let td = tempdir().unwrap();
        let repo = td.path();

        init_repo_with_file(repo, "a.txt", "hello").unwrap();
        let shadow = Shadow::create(repo, None).unwrap();

        let shadow_file = shadow.worktree.join("a.txt");
        write(&shadow_file, "shadowed").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // original must still have original content until apply
        let orig = read_to_string(repo.join("a.txt")).unwrap();
        assert_eq!(orig, "hello");
    }

    #[test]
    fn apply_updates_worktree_without_commit() {
        let td = tempdir().unwrap();
        let repo = td.path();

        init_repo_with_file(repo, "b.txt", "one").unwrap();
        let before_head = run_git(repo, &["rev-parse", "HEAD"]).unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        let shadow_file = shadow.worktree.join("b.txt");
        write(&shadow_file, "two").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // apply
        shadow.apply().unwrap();

        // working tree updated
        let content = read_to_string(repo.join("b.txt")).unwrap();
        assert_eq!(content, "two");

        // HEAD unchanged
        let after_head = run_git(repo, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(before_head, after_head);
    }

    #[test]
    fn apply_refuses_when_conflicting_user_change_exists() {
        let td = tempdir().unwrap();
        let repo = td.path();

        init_repo_with_file(repo, "c.txt", "base").unwrap();
        let shadow = Shadow::create(repo, None).unwrap();

        // modify in shadow and commit
        let shadow_file = shadow.worktree.join("c.txt");
        write(&shadow_file, "shadow").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // create conflicting dirty change in original (not committed)
        write(repo.join("c.txt"), "local-dirty").unwrap();

        let res = shadow.apply();
        assert!(res.is_err());

        // original working tree content preserved
        let orig = read_to_string(repo.join("c.txt")).unwrap();
        assert_eq!(orig, "local-dirty");
    }

    #[test]
    fn apply_preserves_unrelated_dirty_user_changes() {
        let td = tempdir().unwrap();
        let repo = td.path();

        init_repo_with_file(repo, "d.txt", "base").unwrap();
        write(repo.join("unrelated.txt"), "me").unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        let shadow_file = shadow.worktree.join("d.txt");
        write(&shadow_file, "shadowed").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // unrelated file is dirty locally
        write(repo.join("unrelated.txt"), "me-mod").unwrap();

        shadow.apply().unwrap();

        // unrelated preserved
        let u = read_to_string(repo.join("unrelated.txt")).unwrap();
        assert_eq!(u, "me-mod");

        // applied change present
        let d = read_to_string(repo.join("d.txt")).unwrap();
        assert_eq!(d, "shadowed");
    }

    #[test]
    fn apply_preserves_preexisting_staged_index() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one").unwrap();

        // create and stage an unrelated file in the original repo
        std::fs::write(repo.join("staged.txt"), "staged").unwrap();
        run_git(repo, &["add", "staged.txt"]).unwrap();
        let before_index = run_git(repo, &["ls-files", "-s"]).unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        // make a change in shadow and commit
        let shadow_file = shadow.worktree.join("a.txt");
        write(&shadow_file, "two").unwrap();
        run_git(&shadow.worktree, &["add", "a.txt"]).unwrap();
        shadow.commit_all("shadow change").unwrap();

        // apply shadow
        shadow.apply().unwrap();

        // index must be preserved exactly
        let after_index = run_git(repo, &["ls-files", "-s"]).unwrap();
        assert_eq!(before_index, after_index);
    }

    #[test]
    fn discard_never_changes_original() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "e.txt", "base").unwrap();
        let shadow = Shadow::create(repo, None).unwrap();

        // modify in shadow and commit
        let shadow_file = shadow.worktree.join("e.txt");
        write(&shadow_file, "shadow").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // discard shadow resources
        shadow.discard().unwrap();

        // original untouched
        let orig = read_to_string(repo.join("e.txt")).unwrap();
        assert_eq!(orig, "base");
    }

    #[test]
    fn cleanup_is_idempotent() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "f.txt", "base").unwrap();
        let shadow = Shadow::create(repo, None).unwrap();

        // discard twice
        shadow.discard().unwrap();
        shadow.discard().unwrap();

        // repo unchanged
        let orig = read_to_string(repo.join("f.txt")).unwrap();
        assert_eq!(orig, "base");
    }

    #[test]
    fn path_traversal_rejected() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "h.txt", "x").unwrap();

        let bad = Path::new("../evil.txt");
        assert!(ensure_path_within_repo(repo, bad).is_err());
    }

    #[test]
    fn absolute_path_rejected() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "i.txt", "x").unwrap();
        let bad = if cfg!(windows) {
            Path::new("C:\\Windows\\system.ini")
        } else {
            Path::new("/etc/passwd")
        };
        assert!(ensure_path_within_repo(repo, bad).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn symlink_escape_rejected() {
        use std::os::unix::fs::symlink;
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "j.txt", "x").unwrap();

        let outside = td.path().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();

        let link = repo.join("link_out");
        symlink(&outside, &link).unwrap();

        let rel = Path::new("link_out");
        let res = ensure_path_within_repo(repo, rel);
        assert!(res.is_err());
    }

    #[test]
    fn rename_over_existing_conflict_rejected() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "k1.txt", "one").unwrap();
        init_repo_with_file(repo, "k2.txt", "two").unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        // in shadow, remove existing k2 and rename k1 -> k2
        run_git(&shadow.worktree, &["rm", "k2.txt"]).unwrap();
        run_git(&shadow.worktree, &["mv", "k1.txt", "k2.txt"]).unwrap();
        shadow.commit_all("shadow rename to k2").unwrap();

        // create conflicting dirty change in original (not committed)
        write(repo.join("k2.txt"), "local-mod").unwrap();

        // applying should fail because target exists and is locally modified
        let res = shadow.apply();
        assert!(res.is_err());
    }

    #[test]
    fn mid_apply_failure_rolls_back_all_previous_file_changes() {
        // Ordering not needed here
        // prepare repo
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "f1.txt", "one").unwrap();
        init_repo_with_file(repo, "f2.txt", "two").unwrap();
        init_repo_with_file(repo, "f3.txt", "three").unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        // modify files in shadow
        write(shadow.worktree.join("f1.txt"), "ONE").unwrap();
        write(shadow.worktree.join("f2.txt"), "TWO").unwrap();
        write(shadow.worktree.join("f3.txt"), "THREE").unwrap();
        run_git(&shadow.worktree, &["add", "f1.txt", "f2.txt", "f3.txt"]).unwrap();
        shadow.commit_all("shadow changes").unwrap();

        // set failure after first apply op (index 1)
        shadow.set_apply_fail_after(1);

        let orig1 = std::fs::read_to_string(repo.join("f1.txt")).unwrap();
        let orig2 = std::fs::read_to_string(repo.join("f2.txt")).unwrap();
        let orig3 = std::fs::read_to_string(repo.join("f3.txt")).unwrap();

        let res = shadow.apply();
        assert!(res.is_err());

        // ensure original restored
        let now1 = std::fs::read_to_string(repo.join("f1.txt")).unwrap();
        let now2 = std::fs::read_to_string(repo.join("f2.txt")).unwrap();
        let now3 = std::fs::read_to_string(repo.join("f3.txt")).unwrap();
        assert_eq!(orig1, now1);
        assert_eq!(orig2, now2);
        assert_eq!(orig3, now3);

        // No global reset necessary; the failure injection was per-Shadow and the shadow was consumed.
    }

    #[test]
    fn unborn_repository_is_rejected_cleanly() {
        let td = tempdir().unwrap();
        let repo = td.path();
        // init repository but do not commit
        run_git(repo, &["init"]).unwrap();
        // attempt to create shadow should yield UnbornRepository
        let res = Shadow::create(repo, None);
        assert!(matches!(res, Err(GitShadowError::UnbornRepository(_))));
    }

    #[test]
    fn failed_apply_leaves_original_unchanged() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "g.txt", "orig").unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        let shadow_file = shadow.worktree.join("g.txt");
        write(&shadow_file, "shadow").unwrap();
        shadow.commit_all("shadow change").unwrap();

        // create conflicting dirty change in original (not committed)
        write(repo.join("g.txt"), "local-dirty").unwrap();

        let before = snapshot_working_tree(repo).unwrap();
        let res = shadow.apply();
        assert!(res.is_err());
        let after = snapshot_working_tree(repo).unwrap();
        assert_eq!(
            before, after,
            "working tree must be byte-for-byte identical after failed apply"
        );
    }

    #[test]
    fn apply_supports_new_deleted_and_renamed_files() {
        let td = tempdir().unwrap();
        let repo = td.path();
        // initial files
        init_repo_with_file(repo, "a.txt", "one").unwrap();
        write(repo.join("b.txt"), "two").unwrap();
        run_git(repo, &["add", "b.txt"]).unwrap();
        run_git(repo, &["commit", "-m", "add b"]).unwrap();

        let shadow = Shadow::create(repo, None).unwrap();
        // rename a.txt -> a2.txt
        run_git(&shadow.worktree, &["mv", "a.txt", "a2.txt"]).unwrap();
        // delete b.txt
        run_git(&shadow.worktree, &["rm", "b.txt"]).unwrap();
        // new file c.txt
        write(shadow.worktree.join("c.txt"), "three").unwrap();
        run_git(&shadow.worktree, &["add", "c.txt"]).unwrap();
        shadow.commit_all("shadow changes").unwrap();

        shadow.apply().unwrap();

        // checks
        assert!(repo.join("a2.txt").exists());
        assert!(!repo.join("b.txt").exists());
        let c = read_to_string(repo.join("c.txt")).unwrap();
        assert_eq!(c, "three");
    }

    #[test]
    fn apply_supports_binary_file_changes() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "bin.dat", "").unwrap();
        // write binary data in shadow
        let shadow = Shadow::create(repo, None).unwrap();
        let bin = vec![0u8, 1, 2, 3, 4, 255u8];
        std::fs::write(shadow.worktree.join("bin.dat"), &bin).unwrap();
        run_git(&shadow.worktree, &["add", "bin.dat"]).unwrap();
        shadow.commit_all("binary").unwrap();

        shadow.apply().unwrap();

        let got = std::fs::read(repo.join("bin.dat")).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    #[cfg(unix)]
    fn file_mode_executable_bit_behavior() {
        use std::os::unix::fs::PermissionsExt;
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "ex.sh", "echo hi").unwrap();
        let shadow = Shadow::create(repo, None).unwrap();
        let p = shadow.worktree.join("ex.sh");
        let mut perm = std::fs::metadata(&p).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&p, perm).unwrap();
        run_git(&shadow.worktree, &["add", "ex.sh"]).unwrap();
        shadow.commit_all("make exec").unwrap();

        shadow.apply().unwrap();
        let meta = std::fs::metadata(repo.join("ex.sh")).unwrap();
        assert!(
            meta.permissions().mode() & 0o111 != 0,
            "executable bit should be set on unix"
        );
    }
}
