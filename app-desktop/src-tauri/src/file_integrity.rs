use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

type Entry = (u64, Option<SystemTime>, bool);
/// Hash once per unchanged file. No network and no model imports.
pub fn verified(path: &Path, expected_size: u64, expected_hash: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, String), Entry>>> = OnceLock::new();
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != expected_size {
        return false;
    }
    let key = (path.to_path_buf(), expected_hash.into());
    let modified = metadata.modified().ok();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Ok(cache) = cache.lock() {
        if let Some((size, time, valid)) = cache.get(&key) {
            if *size == metadata.len() && *time == modified {
                return *valid;
            }
        }
    }
    let valid = (|| -> std::io::Result<bool> {
        let mut file = fs::File::open(path)?;
        let mut digest = Sha256::new();
        let mut block = [0_u8; 65536];
        loop {
            let n = file.read(&mut block)?;
            if n == 0 {
                break;
            }
            digest.update(&block[..n]);
        }
        let after = file.metadata()?;
        Ok(after.len() == metadata.len()
            && after.modified().ok() == modified
            && format!("{:x}", digest.finalize()) == expected_hash)
    })()
    .unwrap_or(false);
    if let Ok(mut cache) = cache.lock() {
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(key, (metadata.len(), modified, valid));
    }
    valid
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_size_corruption_is_not_ready_and_repair_is_rechecked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.pth");
        let good = b"real-model";
        let hash = format!("{:x}", Sha256::digest(good));
        fs::write(&path, b"bad--model").unwrap();
        assert!(!verified(&path, good.len() as u64, &hash));
        // Atomic repair supplies a new file identity/mtime as downloads do.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(&path, good).unwrap();
        assert!(verified(&path, good.len() as u64, &hash));
        assert!(!verified(&path, 1, &hash));
    }
}
