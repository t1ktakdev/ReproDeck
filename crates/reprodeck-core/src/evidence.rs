use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn is_symlink_or_reparse(path: &Path) -> std::io::Result<bool> {
    let meta = fs::symlink_metadata(path)?;
    let is_symlink = meta.file_type().is_symlink();

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        let is_reparse = (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
        Ok(is_symlink || is_reparse)
    }

    #[cfg(not(windows))]
    {
        Ok(is_symlink)
    }
}

fn verify_existing_artifact(
    path: &Path,
    expected_checksum: &str,
    expected_size: usize,
) -> std::io::Result<()> {
    if is_symlink_or_reparse(path)? {
        return Err(std::io::Error::other(
            "artifact final path is a symlink or reparse point",
        ));
    }

    let bytes = fs::read(path)?;
    if bytes.len() != expected_size {
        return Err(std::io::Error::other(
            "artifact store integrity mismatch: existing size differs",
        ));
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected_checksum {
        return Err(std::io::Error::other(
            "artifact store integrity mismatch: existing checksum differs",
        ));
    }
    Ok(())
}

pub fn store_artifact(storage_dir: &Path, data: &[u8]) -> std::io::Result<(String, PathBuf)> {
    fs::create_dir_all(storage_dir)?;
    let base = storage_dir.canonicalize()?;

    let checksum = hex::encode(Sha256::digest(data));
    let prefix = checksum
        .get(0..2)
        .ok_or_else(|| std::io::Error::other("checksum too short"))?;
    let dir = storage_dir.join(prefix);

    if dir.exists() && is_symlink_or_reparse(&dir)? {
        return Err(std::io::Error::other(
            "artifact storage prefix is a symlink or reparse point",
        ));
    }

    fs::create_dir_all(&dir)?;
    let dir_canon = dir.canonicalize()?;
    if !dir_canon.starts_with(&base) {
        return Err(std::io::Error::other(
            "artifact dir canonicalization outside storage root",
        ));
    }

    let finalp = dir.join(&checksum);
    if finalp.exists() {
        verify_existing_artifact(&finalp, &checksum, data.len())?;
        return Ok((checksum, finalp));
    }

    // A unique temp name avoids concurrent writers clobbering each other's
    // temporary file before the final content-addressed rename.
    let tmp = dir.join(format!("{}.{}.tmp", checksum, Uuid::new_v4()));
    if let Err(e) = fs::write(&tmp, data) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    // Re-check containment after the temporary write and before rename.
    let current_dir_canon = dir.canonicalize()?;
    if current_dir_canon != dir_canon || !current_dir_canon.starts_with(&base) {
        let _ = fs::remove_file(&tmp);
        return Err(std::io::Error::other(
            "artifact directory changed or escaped storage root",
        ));
    }

    match fs::rename(&tmp, &finalp) {
        Ok(()) => {}
        Err(_e) if finalp.exists() => {
            // Another writer may have won the race. Accept it only if the
            // existing content matches the content-addressed identity.
            let _ = fs::remove_file(&tmp);
            verify_existing_artifact(&finalp, &checksum, data.len())?;
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }

    let final_canon = finalp.canonicalize()?;
    if !final_canon.starts_with(&base) {
        let _ = fs::remove_file(&finalp);
        return Err(std::io::Error::other(
            "artifact stored outside storage root",
        ));
    }
    verify_existing_artifact(&finalp, &checksum, data.len())?;

    Ok((checksum, finalp))
}

/// Ensure a candidate path is contained within storage_dir and not a symlink escape.
pub fn path_within_storage(storage_dir: &Path, p: &Path) -> bool {
    match p.canonicalize() {
        Ok(c) => match storage_dir.canonicalize() {
            Ok(base) => c.starts_with(base),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_and_check_artifact() {
        let dir = tempdir().unwrap();
        let (checksum, path) = store_artifact(dir.path(), b"hello world").unwrap();
        assert!(path.exists());
        assert_eq!(checksum.len(), 64);
        assert!(path.starts_with(dir.path()));
        assert_eq!(fs::read(path).unwrap(), b"hello world");
    }

    #[test]
    fn duplicate_artifact_idempotent() {
        let dir = tempdir().unwrap();
        let data = b"same content";
        let (c1, p1) = store_artifact(dir.path(), data).unwrap();
        let (c2, p2) = store_artifact(dir.path(), data).unwrap();
        assert_eq!(c1, c2);
        assert_eq!(p1, p2);
        assert_eq!(fs::read(p1).unwrap(), data);
    }

    #[test]
    fn existing_corrupt_content_is_rejected() {
        let dir = tempdir().unwrap();
        let data = b"expected content";
        let checksum = hex::encode(Sha256::digest(data));
        let prefix = &checksum[0..2];
        let prefix_dir = dir.path().join(prefix);
        fs::create_dir_all(&prefix_dir).unwrap();
        fs::write(prefix_dir.join(&checksum), b"corrupt").unwrap();

        let res = store_artifact(dir.path(), data);
        assert!(res.is_err());
    }

    #[test]
    fn path_within_storage_detects_outside() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("foo");
        fs::write(&outside_file, b"x").unwrap();
        assert!(!path_within_storage(dir.path(), &outside_file));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_prefix_prevented() {
        use std::os::unix::fs as unixfs;
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let data = b"symlink test";
        let checksum = hex::encode(Sha256::digest(data));
        let prefix = &checksum[0..2];
        let prefix_path = dir.path().join(prefix);
        unixfs::symlink(outside.path(), &prefix_path).unwrap();
        assert!(prefix_path.exists());
        let res = store_artifact(dir.path(), data);
        assert!(res.is_err());
        let outside_file = outside.path().join(&checksum);
        assert!(!outside_file.exists());
    }
}
