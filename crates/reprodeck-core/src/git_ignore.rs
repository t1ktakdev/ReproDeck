use git2::Repository;
use std::path::{Path, PathBuf};

/// Git-aware ignore matcher shared by repository discovery and context selection.
///
/// ReproDeck canonicalizes project roots before walking them. On Windows that can
/// introduce the extended-length `\\?\` prefix, while libgit2 normally reports a
/// non-prefixed workdir. Canonicalizing the workdir once keeps both path forms
/// comparable and prevents `.gitignore` rules from being silently bypassed.
pub(crate) struct GitIgnoreMatcher {
    repository: Option<Repository>,
    canonical_workdir: Option<PathBuf>,
}

impl GitIgnoreMatcher {
    pub(crate) fn discover(root: &Path) -> Self {
        let repository = Repository::discover(root).ok();
        let canonical_workdir = repository
            .as_ref()
            .and_then(|repository| repository.workdir())
            .and_then(|workdir| workdir.canonicalize().ok());
        Self {
            repository,
            canonical_workdir,
        }
    }

    pub(crate) fn is_ignored(&self, absolute_path: &Path) -> bool {
        let (Some(repository), Some(workdir)) =
            (self.repository.as_ref(), self.canonical_workdir.as_deref())
        else {
            return false;
        };
        let Ok(repo_relative) = absolute_path.strip_prefix(workdir) else {
            return false;
        };
        if repo_relative.as_os_str().is_empty() {
            return false;
        }
        repository
            .status_should_ignore(repo_relative)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn honors_gitignore_from_a_canonical_project_root() {
        let dir = tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.ts\ngenerated/\n").unwrap();
        fs::write(dir.path().join("ignored.ts"), "ignored\n").unwrap();
        fs::create_dir_all(dir.path().join("generated")).unwrap();

        let canonical_root = dir.path().canonicalize().unwrap();
        let matcher = GitIgnoreMatcher::discover(&canonical_root);

        assert!(matcher.is_ignored(&canonical_root.join("ignored.ts")));
        assert!(matcher.is_ignored(&canonical_root.join("generated")));
        assert!(!matcher.is_ignored(&canonical_root.join("visible.ts")));
    }
}
