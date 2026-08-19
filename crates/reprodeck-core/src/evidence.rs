
use std::fs;
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};

pub fn store_artifact(storage_dir: &Path, data: &[u8]) -> std::io::Result<(String, PathBuf)> {
    // compute checksum
    let mut hasher = Sha256::new();
    hasher.update(data);
    let checksum = hex::encode(hasher.finalize());

    // two-level directory by first two chars
    if checksum.len() < 2 {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "checksum too short"));
    }
    let dir = storage_dir.join(&checksum[0..2]);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{}.tmp", &checksum));
    let finalp = dir.join(&checksum);

    // write atomically
    fs::write(&tmp, data)?;
    fs::rename(&tmp, &finalp)?;

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
        // ensure containment under storage dir
        assert!(path.starts_with(dir.path()));
    }

    #[test]
    fn duplicate_artifact_idempotent() {
        let dir = tempdir().unwrap();
        let data = b"same content";
        let (c1, p1) = store_artifact(dir.path(), data).unwrap();
        let (c2, p2) = store_artifact(dir.path(), data).unwrap();
        assert_eq!(c1, c2);
        assert!(p1.exists());
        assert!(p2.exists());
    }

    #[test]
    fn path_within_storage_detects_outside() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("foo");
        std::fs::write(&outside_file, b"x").unwrap();
        assert!(!path_within_storage(dir.path(), &outside_file));
    }
}
