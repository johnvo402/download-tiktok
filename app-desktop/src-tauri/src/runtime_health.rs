use crate::runtime_paths;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealth {
    pub app_version: String,
    pub build_commit: String,
    pub os: String,
    pub architecture: String,
    pub project_storage_available: bool,
    pub ffmpeg_available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_available: bool,
    pub worker_available: bool,
    pub worker_mode: String,
    pub worker_version: Option<String>,
    pub cpu_threads: usize,
}

pub fn check(app: &AppHandle) -> RuntimeHealth {
    let projects = app
        .path()
        .app_data_dir()
        .map(|path| path.join("projects"))
        .ok();
    let storage_available = projects.as_ref().is_some_and(storage_writable);
    let ffmpeg = runtime_paths::resolve_ffmpeg();
    let ffprobe = runtime_paths::resolve_ffprobe();
    let ffmpeg_version = ffmpeg
        .as_ref()
        .ok()
        .and_then(|path| runtime_paths::program_version(path));
    let bundled_worker = runtime_paths::packaged_worker_dir().is_some();
    let worker_available = bundled_worker || runtime_paths::development_worker_dir().is_some();
    RuntimeHealth {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        build_commit: option_env!("GIT_COMMIT").unwrap_or("unknown").to_string(),
        os: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        project_storage_available: storage_available,
        ffmpeg_available: runtime_paths::binary_available(ffmpeg),
        ffmpeg_version,
        ffprobe_available: runtime_paths::binary_available(ffprobe),
        worker_available,
        worker_mode: if bundled_worker {
            "bundled".into()
        } else if runtime_paths::development_worker_dir().is_some() {
            "development".into()
        } else {
            "missing".into()
        },
        worker_version: worker_version(),
        cpu_threads: std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1),
    }
}

fn worker_version() -> Option<String> {
    if let Some(root) = runtime_paths::packaged_worker_dir() {
        return fs::read_to_string(root.join("worker-version.txt"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    runtime_paths::development_worker_dir().map(|_| "Source worker".to_string())
}

fn storage_writable(projects: &PathBuf) -> bool {
    if fs::create_dir_all(projects).is_err() {
        return false;
    }
    let path = projects.join(format!(".health-{}", Uuid::new_v4()));
    let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&path) else {
        return false;
    };
    let result = file.write_all(b"ok").and_then(|_| file.sync_all()).is_ok();
    drop(file);
    let _ = fs::remove_file(path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn writable_storage_check_leaves_no_probe_file() {
        let temp = tempdir().unwrap();
        let projects = temp.path().join("projects");
        assert!(storage_writable(&projects));
        assert_eq!(fs::read_dir(projects).unwrap().count(), 0);
    }
}
