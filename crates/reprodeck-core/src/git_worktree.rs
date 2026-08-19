use anyhow::Result;
use git2::Repository;
use std::path::Path;

/// Return the current HEAD commit id (as hex) for the repository at `path`.
pub fn head_commit_hex(path: &Path) -> Result<String> {
    let repo = Repository::discover(path)?;
    let head = repo.head()?;
    let oid = head
        .target()
        .ok_or_else(|| anyhow::anyhow!("no HEAD target"))?;
    Ok(oid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use tempfile::tempdir;

    #[test]
    fn head_commit_reports_commit() {
        let dir = tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        // create a file and commit
        let file_path = dir.path().join("README.md");
        std::fs::write(&file_path, "hello").unwrap();
        repo.index()
            .unwrap()
            .add_path(std::path::Path::new("README.md"))
            .unwrap();
        let sig = Signature::now("Tester", "tester@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        let head = head_commit_hex(dir.path()).unwrap();
        assert_eq!(head, oid.to_string());
    }
}
