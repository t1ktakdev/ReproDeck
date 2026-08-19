use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub fn store_artifact(storage_dir: &Path, data: &[u8]) -> std::io::Result<(String, PathBuf)> {
    // compute checksum
    let mut hasher = Sha256::new();
    hasher.update(data);
    let checksum = hex::encode(hasher.finalize());

    // two-level directory by first two chars
    if checksum.len() < 2 {
        return Err(std::io::Error::other("checksum too short"));
    }

    // canonicalize storage root
    let base = storage_dir.canonicalize()?;
    let prefix = &checksum[0..2];
    let dir = storage_dir.join(prefix);

    // If an attacker pre-created a symlink or reparse point at dir, refuse to proceed.
    if let Ok(meta) = std::fs::symlink_metadata(&dir) {
        // On Unix, file_type().is_symlink() detects symlinks.
        let mut is_bad = meta.file_type().is_symlink();
        // On Windows, also treat reparse points/junctions as unsafe (FILE_ATTRIBUTE_REPARSE_POINT = 0x400).
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                is_bad = true;
            }
        }
        if is_bad {
            return Err(std::io::Error::other(
                "artifact storage prefix is a symlink or reparse point",
            ));
        }
    }

    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{}.tmp", checksum));
    let finalp = dir.join(&checksum);

    // write to tmp
    match fs::write(&tmp, data) {
        Ok(()) => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }

    // Re-check containment before rename to mitigate TOCTOU where possible
    let dir_canon = dir.canonicalize()?;
    if !dir_canon.starts_with(&base) {
        let _ = fs::remove_file(&tmp);
        return Err(std::io::Error::other(
            "artifact dir canonicalization outside storage root",
        ));
    }

    // atomic rename into final path
    if let Err(e) = fs::rename(&tmp, &finalp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    // Verify final path containment
    let final_canon = finalp.canonicalize()?;
    if !final_canon.starts_with(&base) {
        // attempt to remove the file we just created
        let _ = fs::remove_file(&finalp);
        return Err(std::io::Error::other(
            "artifact stored outside storage root",
        ));
    }

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
        // create symlink at prefix pointing outside
        unixfs::symlink(outside.path(), &prefix_path).unwrap();
        // ensure symlink exists
        assert!(prefix_path.exists());
        // attempt to store artifact -> should error and not write outside file
        let res = store_artifact(dir.path(), data);
        assert!(res.is_err());
        // ensure outside did not receive file named checksum
        let outside_file = outside.path().join(&checksum);
        assert!(!outside_file.exists());
    }
}
