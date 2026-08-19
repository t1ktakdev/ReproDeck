
use std::fs;
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};

pub fn store_artifact(storage_dir: &Path, data: &[u8]) -> std::io::Result<(String, PathBuf)> {
    // compute checksum
    let mut hasher = Sha256::new();
    hasher.update(data);
    let checksum = hex::encode(hasher.finalize());

    // two-level directory by first two chars
    let dir = storage_dir.join(&checksum[0..2]);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{}.tmp", &checksum));
    let finalp = dir.join(&checksum);

    // write atomically
    fs::write(&tmp, data)?;
    fs::rename(&tmp, &finalp)?;

    Ok((checksum, finalp))
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
}
