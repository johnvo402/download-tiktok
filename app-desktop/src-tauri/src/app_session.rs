//! Per-data-directory lifetime lock and crash marker; no model probes at startup.
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static SESSION_ID: OnceLock<String> = OnceLock::new();
pub fn id() -> &'static str {
    SESSION_ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub session_id: String,
    pub started_at: String,
    pub clean_shutdown: bool,
    pub previous_session_interrupted: bool,
}

pub struct AppSession {
    _lock: File,
    path: PathBuf,
    pub status: SessionStatus,
}
impl AppSession {
    pub fn begin(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock_path = root.join("application.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| unsafe_path(&m)) {
            return Err("Khóa ứng dụng không an toàn.".into());
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let lock = options.open(lock_path).map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                "JC AI đã mở.".to_string()
            } else {
                format!("Không thể ghi thư mục dữ liệu ứng dụng: {error}")
            }
        })?;
        let path = root.join("session.json");
        let previous = crate::recovery_store::read(&path, None, |b| {
            serde_json::from_slice::<SessionStatus>(b).map_err(|e| e.to_string())
        })
        .ok();
        let status = SessionStatus {
            session_id: id().into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            clean_shutdown: false,
            previous_session_interrupted: previous.is_some_and(|s| !s.clean_shutdown),
        };
        crate::recovery_store::write(
            &path,
            &serde_json::to_vec_pretty(&status).map_err(|e| e.to_string())?,
        )?;
        Ok(Self {
            _lock: lock,
            path,
            status,
        })
    }
    pub fn finish(&self) -> Result<(), String> {
        let mut status = self.status.clone();
        status.clean_shutdown = true;
        crate::recovery_store::write(
            &self.path,
            &serde_json::to_vec_pretty(&status).map_err(|e| e.to_string())?,
        )
    }
}
fn unsafe_path(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink() || !metadata.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_and_clean_sessions_are_distinguished() {
        let dir = tempfile::tempdir().unwrap();
        let first = AppSession::begin(dir.path()).unwrap();
        assert!(!first.status.previous_session_interrupted);
        drop(first);
        let recovered = AppSession::begin(dir.path()).unwrap();
        assert!(recovered.status.previous_session_interrupted);
        recovered.finish().unwrap();
        drop(recovered);
        assert!(
            !AppSession::begin(dir.path())
                .unwrap()
                .status
                .previous_session_interrupted
        );
    }
    #[cfg(windows)]
    #[test]
    fn second_instance_cannot_open_same_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = AppSession::begin(dir.path()).unwrap();
        assert!(AppSession::begin(dir.path()).is_err());
        drop(first);
        assert!(AppSession::begin(dir.path()).is_ok());
    }
}
