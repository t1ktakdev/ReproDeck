use git2::{Delta, DiffFile, DiffOptions, FileMode, Oid, Repository};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions, Permissions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum GitShadowError {
    #[error("git failed: {0} -- {1}")]
    GitFailed(String, String),
    #[error("git output was not valid UTF-8 for command: {0}")]
    GitOutputNotUtf8(String),
    #[error("git repository error: {0}")]
    Git2(#[from] git2::Error),
    #[error("repository has no commits (unborn) for path {0}")]
    UnbornRepository(String),
    #[error("patch could not be applied cleanly: {0}")]
    PatchApplyFailed(String),
    #[error("submodule/gitlink changes are not supported")]
    SubmoduleNotSupported,
    #[error("unsupported Git file type for {0}")]
    UnsupportedFileType(String),
    #[error("Git path cannot be represented safely on this platform")]
    UnsupportedPathEncoding,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("apply failed and rollback also failed; apply={apply_error}; rollback={rollback_error}")]
    RollbackFailed {
        apply_error: String,
        rollback_error: String,
    },
    #[error("apply succeeded but cleanup failed; pending cleanup marker at {0}")]
    AppliedCleanupPending(PathBuf),
}

type Result<T> = std::result::Result<T, GitShadowError>;

fn run_git_bytes(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(GitShadowError::Io)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(GitShadowError::GitFailed(
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String> {
    let bytes = run_git_bytes(cwd, args)?;
    String::from_utf8(bytes)
        .map(|value| value.trim().to_string())
        .map_err(|_| GitShadowError::GitOutputNotUtf8(args.join(" ")))
}

fn run_worktree_add(repo: &Path, branch: &str, worktree: &Path, base: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo)
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(branch)
        .arg(worktree)
        .arg(base)
        .output()
        .map_err(GitShadowError::Io)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitShadowError::GitFailed(
            "worktree add".to_string(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

#[cfg(unix)]
fn git_path(bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}

#[cfg(windows)]
fn git_path(bytes: &[u8]) -> Result<PathBuf> {
    let value = std::str::from_utf8(bytes).map_err(|_| GitShadowError::UnsupportedPathEncoding)?;
    Ok(PathBuf::from(value))
}

fn diff_path(file: DiffFile<'_>) -> Result<PathBuf> {
    file.path_bytes()
        .ok_or(GitShadowError::UnsupportedPathEncoding)
        .and_then(git_path)
}

fn mode_is_executable(mode: FileMode, path: &Path) -> Result<bool> {
    match mode {
        FileMode::BlobExecutable => Ok(true),
        FileMode::Blob | FileMode::BlobGroupWritable => Ok(false),
        FileMode::Commit => Err(GitShadowError::SubmoduleNotSupported),
        FileMode::Link | FileMode::Tree | FileMode::Unreadable => {
            Err(GitShadowError::UnsupportedFileType(format!("{:?}", path)))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DesiredState {
    Missing,
    File { data: Vec<u8>, executable: bool },
}

#[derive(Debug, Clone)]
struct Mutation {
    path: PathBuf,
    expected: DesiredState,
    desired: DesiredState,
}

fn file_state_from_diff(repo: &Repository, file: DiffFile<'_>, path: &Path) -> Result<DesiredState> {
    let executable = mode_is_executable(file.mode(), path)?;
    let blob = repo.find_blob(file.id())?;
    Ok(DesiredState::File {
        data: blob.content().to_vec(),
        executable,
    })
}

fn insert_mutation(mutations: &mut BTreeMap<PathBuf, Mutation>, mutation: Mutation) -> Result<()> {
    if mutations.contains_key(&mutation.path) {
        return Err(GitShadowError::PatchApplyFailed(format!(
            "ambiguous multiple changes for {:?}",
            mutation.path
        )));
    }
    mutations.insert(mutation.path.clone(), mutation);
    Ok(())
}

fn build_mutations(repo: &Repository, base: Oid, target: Oid) -> Result<Vec<Mutation>> {
    let base_tree = repo.find_commit(base)?.tree()?;
    let target_tree = repo.find_commit(target)?.tree()?;
    let mut options = DiffOptions::new();
    options.include_typechange(true);
    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&target_tree), Some(&mut options))?;
    let mut mutations = BTreeMap::<PathBuf, Mutation>::new();

    for delta in diff.deltas() {
        match delta.status() {
            Delta::Added => {
                let new_file = delta.new_file();
                let path = diff_path(new_file)?;
                let desired = file_state_from_diff(repo, new_file, &path)?;
                insert_mutation(
                    &mut mutations,
                    Mutation {
                        path,
                        expected: DesiredState::Missing,
                        desired,
                    },
                )?;
            }
            Delta::Deleted => {
                let old_file = delta.old_file();
                let path = diff_path(old_file)?;
                let expected = file_state_from_diff(repo, old_file, &path)?;
                insert_mutation(
                    &mut mutations,
                    Mutation {
                        path,
                        expected,
                        desired: DesiredState::Missing,
                    },
                )?;
            }
            Delta::Modified => {
                let old_file = delta.old_file();
                let new_file = delta.new_file();
                let old_path = diff_path(old_file)?;
                let new_path = diff_path(new_file)?;
                if old_path != new_path {
                    return Err(GitShadowError::PatchApplyFailed(format!(
                        "unexpected path change in modified delta: {:?} -> {:?}",
                        old_path, new_path
                    )));
                }
                let expected = file_state_from_diff(repo, old_file, &old_path)?;
                let desired = file_state_from_diff(repo, new_file, &new_path)?;
                insert_mutation(
                    &mut mutations,
                    Mutation {
                        path: new_path,
                        expected,
                        desired,
                    },
                )?;
            }
            Delta::Renamed => {
                let old_file = delta.old_file();
                let new_file = delta.new_file();
                let old_path = diff_path(old_file)?;
                let new_path = diff_path(new_file)?;
                let expected = file_state_from_diff(repo, old_file, &old_path)?;
                let desired = file_state_from_diff(repo, new_file, &new_path)?;
                insert_mutation(
                    &mut mutations,
                    Mutation {
                        path: old_path,
                        expected,
                        desired: DesiredState::Missing,
                    },
                )?;
                insert_mutation(
                    &mut mutations,
                    Mutation {
                        path: new_path,
                        expected: DesiredState::Missing,
                        desired,
                    },
                )?;
            }
            Delta::Copied => {
                let new_file = delta.new_file();
                let path = diff_path(new_file)?;
                let desired = file_state_from_diff(repo, new_file, &path)?;
                insert_mutation(
                    &mut mutations,
                    Mutation {
                        path,
                        expected: DesiredState::Missing,
                        desired,
                    },
                )?;
            }
            Delta::Typechange => {
                return Err(GitShadowError::UnsupportedFileType(format!(
                    "type change {:?} -> {:?}",
                    delta.old_file().path_bytes(),
                    delta.new_file().path_bytes()
                )));
            }
            Delta::Unmodified => {}
            Delta::Ignored | Delta::Untracked | Delta::Unreadable | Delta::Conflicted => {
                return Err(GitShadowError::PatchApplyFailed(format!(
                    "unsupported diff status {:?}",
                    delta.status()
                )));
            }
        }
    }

    #[cfg(windows)]
    {
        let mut folded = BTreeSet::new();
        for path in mutations.keys() {
            let value = path
                .to_str()
                .ok_or(GitShadowError::UnsupportedPathEncoding)?
                .replace('\\', "/")
                .to_lowercase();
            if !folded.insert(value) {
                return Err(GitShadowError::PatchApplyFailed(
                    "case-only or case-colliding path change is not safe on Windows".to_string(),
                ));
            }
        }
    }

    Ok(mutations.into_values().collect())
}

fn is_symlink_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn ensure_path_within_repo(repo: &Path, relative: &Path) -> Result<()> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(GitShadowError::PatchApplyFailed(format!(
            "unsafe path {:?}",
            relative
        )));
    }
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(GitShadowError::PatchApplyFailed(format!(
                "unsafe path component in {:?}",
                relative
            )));
        }
    }

    let repo_root = repo.canonicalize()?;
    let mut current = repo_root.clone();
    for component in relative.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if is_symlink_or_reparse(&metadata) {
                        return Err(GitShadowError::PatchApplyFailed(format!(
                            "symlink or reparse point is not allowed in apply path {:?}",
                            relative
                        )));
                    }
                    let canonical = current.canonicalize()?;
                    if !canonical.starts_with(&repo_root) {
                        return Err(GitShadowError::PatchApplyFailed(format!(
                            "path escapes repository: {:?}",
                            relative
                        )));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(windows)]
fn executable(_metadata: &fs::Metadata) -> bool {
    false
}

#[derive(Debug, Clone)]
enum SnapshotState {
    Missing,
    File {
        data: Vec<u8>,
        permissions: Permissions,
        executable: bool,
    },
}

fn snapshot_path(repo: &Path, relative: &Path) -> Result<SnapshotState> {
    ensure_path_within_repo(repo, relative)?;
    let absolute = repo.join(relative);
    let metadata = match fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(SnapshotState::Missing),
        Err(error) => return Err(error.into()),
    };
    if is_symlink_or_reparse(&metadata) || !metadata.file_type().is_file() {
        return Err(GitShadowError::PatchApplyFailed(format!(
            "apply target is not a regular file: {:?}",
            relative
        )));
    }
    Ok(SnapshotState::File {
        data: fs::read(&absolute)?,
        permissions: metadata.permissions(),
        executable: executable(&metadata),
    })
}

fn snapshot_matches_expected(snapshot: &SnapshotState, expected: &DesiredState) -> bool {
    match (snapshot, expected) {
        (SnapshotState::Missing, DesiredState::Missing) => true,
        (
            SnapshotState::File {
                data, executable, ..
            },
            DesiredState::File {
                data: expected_data,
                executable: expected_executable,
            },
        ) => {
            if data != expected_data {
                return false;
            }
            #[cfg(unix)]
            {
                executable == expected_executable
            }
            #[cfg(windows)]
            {
                let _ = expected_executable;
                true
            }
        }
        _ => false,
    }
}

fn collect_missing_parent_dirs(repo: &Path, relative: &Path, output: &mut BTreeSet<PathBuf>) {
    let Some(parent) = relative.parent() else {
        return;
    };
    let mut current = repo.to_path_buf();
    for component in parent.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            if !current.exists() {
                output.insert(current.clone());
            }
        }
    }
}

#[cfg(unix)]
fn desired_permissions(snapshot: &SnapshotState, executable: bool) -> Option<Permissions> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = match snapshot {
        SnapshotState::File { permissions, .. } => permissions.clone(),
        SnapshotState::Missing => Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
    };
    let mode = permissions.mode();
    permissions.set_mode(if executable {
        mode | 0o111
    } else {
        mode & !0o111
    });
    Some(permissions)
}

#[cfg(windows)]
fn desired_permissions(snapshot: &SnapshotState, _executable: bool) -> Option<Permissions> {
    match snapshot {
        SnapshotState::File { permissions, .. } => Some(permissions.clone()),
        SnapshotState::Missing => None,
    }
}

#[cfg(windows)]
fn make_removable(path: &Path) -> io::Result<()> {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn make_removable(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn atomic_write(path: &Path, data: &[u8], permissions: Option<Permissions>) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("file path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".reprodeck-write-{}.tmp", Uuid::new_v4()));

    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        if let Some(permissions) = permissions {
            fs::set_permissions(&temp, permissions)?;
        }

        #[cfg(windows)]
        if path.exists() {
            make_removable(path)?;
            fs::remove_file(path)?;
        }

        fs::rename(&temp, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn apply_mutation(repo: &Path, mutation: &Mutation, snapshot: &SnapshotState) -> Result<()> {
    ensure_path_within_repo(repo, &mutation.path)?;
    let absolute = repo.join(&mutation.path);
    match &mutation.desired {
        DesiredState::Missing => {
            if absolute.exists() {
                make_removable(&absolute)?;
                fs::remove_file(&absolute)?;
            }
        }
        DesiredState::File { data, executable } => {
            if let Some(parent) = absolute.parent() {
                fs::create_dir_all(parent)?;
            }
            ensure_path_within_repo(repo, &mutation.path)?;
            atomic_write(
                &absolute,
                data,
                desired_permissions(snapshot, *executable),
            )?;
        }
    }
    Ok(())
}

fn restore_snapshot(repo: &Path, relative: &Path, snapshot: &SnapshotState) -> io::Result<()> {
    let absolute = repo.join(relative);
    match snapshot {
        SnapshotState::Missing => match fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.file_type().is_file() => {
                make_removable(&absolute)?;
                fs::remove_file(&absolute)
            }
            Ok(_) => Err(io::Error::other("rollback target became a non-file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
        SnapshotState::File {
            data, permissions, ..
        } => atomic_write(&absolute, data, Some(permissions.clone())),
    }
}

fn rollback(
    repo: &Path,
    snapshots: &[(PathBuf, SnapshotState)],
    created_dirs: &BTreeSet<PathBuf>,
) -> io::Result<()> {
    let mut first_error: Option<io::Error> = None;
    for (path, snapshot) in snapshots.iter().rev() {
        if let Err(error) = restore_snapshot(repo, path, snapshot) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }

    let mut dirs: Vec<&PathBuf> = created_dirs.iter().collect();
    dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for dir in dirs {
        match fs::remove_dir(dir) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
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
    apply_fail_after: std::sync::atomic::AtomicI32,
}

impl Shadow {
    pub fn create(repo: &Path, base_commit: Option<&str>) -> Result<Self> {
        let discovered = Repository::discover(repo)?;
        let repo_root = discovered
            .workdir()
            .ok_or_else(|| GitShadowError::PatchApplyFailed("bare repositories are not supported".to_string()))?
            .canonicalize()?;
        let head = discovered
            .head()
            .map_err(|_| GitShadowError::UnbornRepository(repo.display().to_string()))?;
        let original_oid = head
            .target()
            .ok_or_else(|| GitShadowError::UnbornRepository(repo.display().to_string()))?;
        let original_head = original_oid.to_string();
        let original_branch = head.shorthand().unwrap_or("HEAD").to_string();
        let base_oid = match base_commit {
            Some(revision) => discovered.revparse_single(revision)?.peel_to_commit()?.id(),
            None => original_oid,
        };
        drop(head);
        drop(discovered);

        let worktree = std::env::temp_dir().join(format!("reprodeck-shadow-{}", Uuid::new_v4()));
        fs::create_dir_all(&worktree)?;
        let branch = format!("reprodeck-shadow-{}", Uuid::new_v4());
        if let Err(error) = run_worktree_add(&repo_root, &branch, &worktree, &base_oid.to_string()) {
            let _ = fs::remove_dir_all(&worktree);
            return Err(error);
        }

        Ok(Self {
            repo: repo_root,
            worktree,
            branch,
            base_commit: base_oid.to_string(),
            original_head,
            original_branch,
            #[cfg(test)]
            apply_fail_after: std::sync::atomic::AtomicI32::new(-1),
        })
    }

    pub fn commit_all(&self, message: &str) -> Result<String> {
        run_git(&self.worktree, &["add", "-A"])?;
        run_git(&self.worktree, &["commit", "-m", message])?;
        run_git(
            &self.repo,
            &["rev-parse", &format!("refs/heads/{}", self.branch)],
        )
    }

    #[cfg(test)]
    pub fn set_apply_fail_after(&self, value: i32) {
        self.apply_fail_after
            .store(value, std::sync::atomic::Ordering::SeqCst);
    }

    /// Human/display form. Apply itself never parses this string; machine path
    /// handling uses libgit2 byte paths instead.
    pub fn diff_name_status(&self) -> Result<String> {
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

    pub fn prepare_patch(&self) -> Result<String> {
        let patch = run_git(
            &self.repo,
            &[
                "diff",
                "--binary",
                &format!("{}..{}", self.base_commit, self.branch),
            ],
        )?;
        if patch.contains("new mode 160000") || patch.contains("old mode 160000") {
            return Err(GitShadowError::SubmoduleNotSupported);
        }
        Ok(patch)
    }

    /// Apply the shadow commit to the original working tree without touching the
    /// Git index or creating a commit. All affected paths are preflighted and
    /// snapshotted before the first mutation; every apply error goes through the
    /// same rollback path.
    pub fn apply(self) -> Result<()> {
        if !self.repo.exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "original repository no longer exists").into());
        }
        let repo = Repository::open(&self.repo)?;
        let current_head = repo
            .head()?
            .target()
            .ok_or_else(|| GitShadowError::UnbornRepository(self.repo.display().to_string()))?;
        if current_head.to_string() != self.original_head {
            return Err(GitShadowError::GitFailed(
                "HEAD moved".to_string(),
                "original HEAD changed since shadow creation".to_string(),
            ));
        }
        let base = Oid::from_str(&self.base_commit)?;
        let target = repo.refname_to_id(&format!("refs/heads/{}", self.branch))?;
        let mutations = build_mutations(&repo, base, target)?;
        drop(repo);

        let mut snapshots = Vec::with_capacity(mutations.len());
        let mut created_dirs = BTreeSet::new();
        for mutation in &mutations {
            ensure_path_within_repo(&self.repo, &mutation.path)?;
            let snapshot = snapshot_path(&self.repo, &mutation.path)?;
            if !snapshot_matches_expected(&snapshot, &mutation.expected) {
                return Err(GitShadowError::PatchApplyFailed(format!(
                    "working tree changed since shadow base at {:?}",
                    mutation.path
                )));
            }
            collect_missing_parent_dirs(&self.repo, &mutation.path, &mut created_dirs);
            snapshots.push((mutation.path.clone(), snapshot));
        }

        let apply_result = (|| -> Result<()> {
            for (index, mutation) in mutations.iter().enumerate() {
                #[cfg(test)]
                if self
                    .apply_fail_after
                    .load(std::sync::atomic::Ordering::SeqCst)
                    == index as i32
                {
                    return Err(io::Error::other("injected apply IO failure").into());
                }
                apply_mutation(&self.repo, mutation, &snapshots[index].1)?;
            }
            Ok(())
        })();

        if let Err(apply_error) = apply_result {
            if let Err(rollback_error) = rollback(&self.repo, &snapshots, &created_dirs) {
                return Err(GitShadowError::RollbackFailed {
                    apply_error: apply_error.to_string(),
                    rollback_error: rollback_error.to_string(),
                });
            }
            return Err(apply_error);
        }

        if let Err(cleanup_error) = self.discard() {
            let marker = std::env::temp_dir().join(format!(
                "reprodeck-recovery-{}.txt",
                Uuid::new_v4()
            ));
            let message = format!(
                "apply succeeded; cleanup pending\nrepo={:?}\nworktree={:?}\nbranch={}\nerror={}\n",
                self.repo, self.worktree, self.branch, cleanup_error
            );
            fs::write(&marker, message)?;
            return Err(GitShadowError::AppliedCleanupPending(marker));
        }
        Ok(())
    }

    pub fn discard(&self) -> Result<()> {
        if self.worktree.exists() {
            let output = Command::new("git")
                .current_dir(&self.repo)
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(&self.worktree)
                .output()?;
            if !output.status.success() && self.worktree.exists() {
                return Err(GitShadowError::GitFailed(
                    "worktree remove --force".to_string(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                ));
            }
        }

        let _ = Command::new("git")
            .current_dir(&self.repo)
            .args(["worktree", "prune"])
            .status();

        let reference = format!("refs/heads/{}", self.branch);
        let branch_exists = Command::new("git")
            .current_dir(&self.repo)
            .args(["show-ref", "--verify", "--quiet", &reference])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if branch_exists {
            run_git(&self.repo, &["branch", "-D", &self.branch])?;
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
    use std::fs::{read_to_string, write};
    use tempfile::tempdir;

    fn init_repo_with_file(repo: &Path, name: &str, content: &str) {
        run_git(repo, &["init"]).unwrap();
        run_git(repo, &["config", "user.email", "tests@reprodeck.local"]).unwrap();
        run_git(repo, &["config", "user.name", "ReproDeck Tests"]).unwrap();
        write(repo.join(name), content).unwrap();
        run_git(repo, &["add", "-A"]).unwrap();
        run_git(repo, &["commit", "-m", "initial"]).unwrap();
    }

    #[test]
    fn original_untouched_until_apply() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("a.txt"), "two").unwrap();
        shadow.commit_all("shadow").unwrap();
        assert_eq!(read_to_string(repo.join("a.txt")).unwrap(), "one");
        shadow.apply().unwrap();
        assert_eq!(read_to_string(repo.join("a.txt")).unwrap(), "two");
    }

    #[test]
    fn apply_does_not_move_head_or_commit() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        let before = run_git(repo, &["rev-parse", "HEAD"]).unwrap();
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("a.txt"), "two").unwrap();
        shadow.commit_all("shadow").unwrap();
        shadow.apply().unwrap();
        assert_eq!(run_git(repo, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(read_to_string(repo.join("a.txt")).unwrap(), "two");
    }

    #[test]
    fn apply_rejects_moved_head() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("a.txt"), "shadow").unwrap();
        shadow.commit_all("shadow").unwrap();
        write(repo.join("b.txt"), "new").unwrap();
        run_git(repo, &["add", "-A"]).unwrap();
        run_git(repo, &["commit", "-m", "move head"]).unwrap();
        assert!(shadow.apply().is_err());
        assert_eq!(read_to_string(repo.join("a.txt")).unwrap(), "one");
    }

    #[test]
    fn dirty_conflicting_change_is_rejected() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("a.txt"), "shadow").unwrap();
        shadow.commit_all("shadow").unwrap();
        write(repo.join("a.txt"), "local").unwrap();
        assert!(shadow.apply().is_err());
        assert_eq!(read_to_string(repo.join("a.txt")).unwrap(), "local");
    }

    #[test]
    fn unrelated_dirty_change_is_preserved() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        write(repo.join("b.txt"), "base").unwrap();
        run_git(repo, &["add", "-A"]).unwrap();
        run_git(repo, &["commit", "-m", "b"]).unwrap();
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("a.txt"), "shadow").unwrap();
        shadow.commit_all("shadow").unwrap();
        write(repo.join("b.txt"), "local dirty").unwrap();
        shadow.apply().unwrap();
        assert_eq!(read_to_string(repo.join("a.txt")).unwrap(), "shadow");
        assert_eq!(read_to_string(repo.join("b.txt")).unwrap(), "local dirty");
    }

    #[test]
    fn staged_index_is_preserved() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        write(repo.join("staged.txt"), "base").unwrap();
        run_git(repo, &["add", "-A"]).unwrap();
        run_git(repo, &["commit", "-m", "staged base"]).unwrap();
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("a.txt"), "shadow").unwrap();
        shadow.commit_all("shadow").unwrap();

        write(repo.join("staged.txt"), "staged change").unwrap();
        run_git(repo, &["add", "staged.txt"]).unwrap();
        let before = run_git_bytes(repo, &["ls-files", "-s"]).unwrap();
        shadow.apply().unwrap();
        let after = run_git_bytes(repo, &["ls-files", "-s"]).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn supports_add_delete_and_rename_as_final_tree_changes() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        write(repo.join("b.txt"), "two").unwrap();
        run_git(repo, &["add", "-A"]).unwrap();
        run_git(repo, &["commit", "-m", "b"]).unwrap();
        let shadow = Shadow::create(repo, None).unwrap();
        run_git(&shadow.worktree, &["mv", "a.txt", "a2.txt"]).unwrap();
        fs::remove_file(shadow.worktree.join("b.txt")).unwrap();
        write(shadow.worktree.join("c.txt"), "three").unwrap();
        shadow.commit_all("tree changes").unwrap();
        shadow.apply().unwrap();
        assert!(!repo.join("a.txt").exists());
        assert_eq!(read_to_string(repo.join("a2.txt")).unwrap(), "one");
        assert!(!repo.join("b.txt").exists());
        assert_eq!(read_to_string(repo.join("c.txt")).unwrap(), "three");
    }

    #[test]
    fn supports_binary_modification() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "bin.dat", "base");
        let shadow = Shadow::create(repo, None).unwrap();
        let binary = vec![0, 1, 2, 3, 255, 0, 42];
        fs::write(shadow.worktree.join("bin.dat"), &binary).unwrap();
        shadow.commit_all("binary").unwrap();
        shadow.apply().unwrap();
        assert_eq!(fs::read(repo.join("bin.dat")).unwrap(), binary);
    }

    #[test]
    fn generic_apply_error_rolls_back_all_previous_mutations() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "f1.txt", "one");
        write(repo.join("f2.txt"), "two").unwrap();
        write(repo.join("f3.txt"), "three").unwrap();
        run_git(repo, &["add", "-A"]).unwrap();
        run_git(repo, &["commit", "-m", "files"]).unwrap();
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("f1.txt"), "ONE").unwrap();
        write(shadow.worktree.join("f2.txt"), "TWO").unwrap();
        write(shadow.worktree.join("f3.txt"), "THREE").unwrap();
        shadow.commit_all("changes").unwrap();
        shadow.set_apply_fail_after(1);
        assert!(shadow.apply().is_err());
        assert_eq!(read_to_string(repo.join("f1.txt")).unwrap(), "one");
        assert_eq!(read_to_string(repo.join("f2.txt")).unwrap(), "two");
        assert_eq!(read_to_string(repo.join("f3.txt")).unwrap(), "three");
    }

    #[test]
    fn discard_is_idempotent() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        let shadow = Shadow::create(repo, None).unwrap();
        shadow.discard().unwrap();
        shadow.discard().unwrap();
        assert!(!shadow.worktree.exists());
    }

    #[test]
    fn unborn_repository_is_rejected_cleanly() {
        let td = tempdir().unwrap();
        run_git(td.path(), &["init"]).unwrap();
        assert!(matches!(
            Shadow::create(td.path(), None),
            Err(GitShadowError::UnbornRepository(_))
        ));
    }

    #[test]
    fn path_traversal_is_rejected() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        assert!(ensure_path_within_repo(repo, Path::new("../outside")).is_err());
        assert!(ensure_path_within_repo(repo, Path::new("./a.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_component_is_rejected() {
        use std::os::unix::fs::symlink;
        let td = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "a.txt", "one");
        symlink(outside.path(), repo.join("escape")).unwrap();
        assert!(ensure_path_within_repo(repo, Path::new("escape/file.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn modifying_existing_file_works_on_unix() {
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "existing.txt", "old");
        let shadow = Shadow::create(repo, None).unwrap();
        write(shadow.worktree.join("existing.txt"), "new").unwrap();
        shadow.commit_all("modify").unwrap();
        shadow.apply().unwrap();
        assert_eq!(read_to_string(repo.join("existing.txt")).unwrap(), "new");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_git_path_is_applied_without_lossy_conversion() {
        use std::os::unix::ffi::OsStringExt;
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "base.txt", "base");
        let shadow = Shadow::create(repo, None).unwrap();
        let name = OsString::from_vec(b"nonutf8-\xff.txt".to_vec());
        fs::write(shadow.worktree.join(&name), b"bytes").unwrap();
        shadow.commit_all("non utf8").unwrap();
        shadow.apply().unwrap();
        assert_eq!(fs::read(repo.join(name)).unwrap(), b"bytes");
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_is_applied() {
        use std::os::unix::fs::PermissionsExt;
        let td = tempdir().unwrap();
        let repo = td.path();
        init_repo_with_file(repo, "run.sh", "echo hi\n");
        let shadow = Shadow::create(repo, None).unwrap();
        let path = shadow.worktree.join("run.sh");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        shadow.commit_all("executable").unwrap();
        shadow.apply().unwrap();
        assert_ne!(fs::metadata(repo.join("run.sh")).unwrap().permissions().mode() & 0o111, 0);
    }
}
