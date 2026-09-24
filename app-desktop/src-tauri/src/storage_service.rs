use crate::job_service::{is_active, JobStore};
use crate::project_service::ProjectStore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub models_bytes: u64,
    pub faster_whisper_bytes: u64,
    pub vie_neu_tts_bytes: u64,
    pub other_model_data_bytes: u64,
    pub projects_bytes: u64,
    pub project_cache_bytes: u64,
    pub temporary_bytes: u64,
    pub logs_bytes: u64,
    pub cache_directory: String,
    pub faster_whisper_directory: String,
    pub vie_neu_tts_directory: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum ModelFolderKind {
    #[serde(rename = "fasterWhisper")]
    FasterWhisper,
    #[serde(rename = "vieNeuTts")]
    VieNeuTts,
}

pub fn usage(app: &AppHandle, jobs: &JobStore) -> Result<StorageUsage, String> {
    let data_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let app_cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let projects = ProjectStore::new(data_root.join("projects"));
    let mut warnings = Vec::new();
    let project_entries = match projects.list() {
        Ok(project_list) => {
            warnings.extend(project_list.warnings);
            project_list.projects
        }
        Err(error) => {
            warnings.push(format!("Không thể đọc dung lượng dự án: {error}"));
            Vec::new()
        }
    };
    let active_exports = jobs
        .list()
        .unwrap_or_else(|error| {
            warnings.push(format!("Không thể đọc trạng thái export; file tạm đang chạy có thể được tính vào cache: {error}"));
            Vec::new()
        })
        .into_iter()
        .filter(|job| job.job_type == "export" && is_active(&job.status))
        .map(|job| job.id)
        .collect::<Vec<_>>();

    let app_models = data_root.join("models");
    let faster_whisper_directory = app_cache.join("fw");
    let vie_neu_tts_directory = app_models.join("huggingface");
    let faster_whisper_bytes = directory_size(&faster_whisper_directory);
    let vie_neu_tts_bytes = directory_size(&vie_neu_tts_directory);
    let models_bytes = directory_size(&app_models)
        .saturating_sub(directory_size(&app_models.join("tts-preview")))
        .saturating_add(faster_whisper_bytes);
    let other_model_data_bytes = models_bytes
        .saturating_sub(faster_whisper_bytes)
        .saturating_sub(vie_neu_tts_bytes);
    let mut project_cache_all = 0_u64;
    let mut project_temporary = 0_u64;
    let mut projects_bytes = 0_u64;
    for project in project_entries {
        let root = match projects.project_directory(&project.id) {
            Ok(root) => root,
            Err(error) => {
                warnings.push(format!("Không thể tính dung lượng dự án ‘{}’: {error}", project.name));
                continue;
            }
        };
        projects_bytes = projects_bytes.saturating_add(directory_size(&root));
        let cache = root.join("cache");
        project_cache_all = project_cache_all.saturating_add(directory_size(&cache));
        let render = cache.join("render");
        project_temporary =
            project_temporary.saturating_add(directory_size_excluding(&render, &active_exports));
    }
    let global_temporary = directory_size(&app_cache.join("downloads"))
        .saturating_add(directory_size(&app_cache.join("tmp")));
    let logs_bytes = directory_size(&data_root.join("logs"));
    Ok(StorageUsage {
        models_bytes,
        faster_whisper_bytes,
        vie_neu_tts_bytes,
        other_model_data_bytes,
        projects_bytes,
        project_cache_bytes: project_cache_all.saturating_sub(project_temporary),
        temporary_bytes: project_temporary.saturating_add(global_temporary),
        logs_bytes,
        cache_directory: app_cache.to_string_lossy().into_owned(),
        faster_whisper_directory: faster_whisper_directory.to_string_lossy().into_owned(),
        vie_neu_tts_directory: vie_neu_tts_directory.to_string_lossy().into_owned(),
        warnings,
    })
}

pub fn clear_temporary(app: &AppHandle, jobs: &JobStore) -> Result<u64, String> {
    let before = usage(app, jobs)?.temporary_bytes;
    let data_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let app_cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let projects = ProjectStore::new(data_root.join("projects"));
    let active_exports = jobs
        .list()?
        .into_iter()
        .filter(|job| job.job_type == "export" && is_active(&job.status))
        .map(|job| job.id)
        .collect::<Vec<_>>();

    for project in projects.list()?.projects {
        let render_dir = projects
            .project_directory(&project.id)?
            .join("cache")
            .join("render");
        remove_children_excluding(&render_dir, &active_exports)?;
    }
    for path in [app_cache.join("downloads"), app_cache.join("tmp")] {
        remove_children(&path)?;
    }
    let after = usage(app, jobs)?.temporary_bytes;
    Ok(before.saturating_sub(after))
}

pub fn open_cache_folder(app: &AppHandle) -> Result<String, String> {
    let folder = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    open_directory(&folder)?;
    Ok(folder.to_string_lossy().into_owned())
}

pub fn open_model_folder(app: &AppHandle, kind: ModelFolderKind) -> Result<String, String> {
    let folder = match kind {
        ModelFolderKind::FasterWhisper => app
            .path()
            .app_cache_dir()
            .map_err(|error| error.to_string())?
            .join("fw"),
        ModelFolderKind::VieNeuTts => app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("models")
            .join("huggingface"),
    };
    open_directory(&folder)?;
    Ok(folder.to_string_lossy().into_owned())
}

fn open_directory(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|error| format!("Không thể tạo thư mục model: {error}"))?;
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command
        .arg(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map_err(|error| format!("Không thể mở thư mục cache: {error}"))?;
    Ok(())
}

fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

fn directory_size_excluding(path: &Path, excluded: &[String]) -> u64 {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true)
    {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            !excluded
                .iter()
                .any(|id| entry.file_name().to_string_lossy() == id.as_str())
        })
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

fn remove_children_excluding(path: &Path, excluded: &[String]) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(
            "Cache cleanup đã bỏ qua thư mục symlink để tránh xóa dữ liệu bên ngoài.".into(),
        );
    }
    let entries =
        fs::read_dir(path).map_err(|error| format!("Không thể đọc cache tạm: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Không thể đọc cache tạm: {error}"))?;
        if excluded
            .iter()
            .any(|id| entry.file_name().to_string_lossy() == id.as_str())
        {
            continue;
        }
        remove_managed_path(&entry.path())?;
    }
    Ok(())
}

fn remove_children(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(
            "Cache cleanup đã bỏ qua thư mục symlink để tránh xóa dữ liệu bên ngoài.".into(),
        );
    }
    let entries =
        fs::read_dir(path).map_err(|error| format!("Không thể đọc thư mục tạm: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Không thể đọc thư mục tạm: {error}"))?;
        remove_managed_path(&entry.path())?;
    }
    Ok(())
}

fn remove_managed_path(path: &PathBuf) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Không thể xác minh cache tạm: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("Cache cleanup đã bỏ qua symlink để tránh xóa dữ liệu bên ngoài.".into());
    }
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|error| format!("Không thể xóa cache tạm: {error}"))
    } else {
        fs::remove_file(path).map_err(|error| format!("Không thể xóa cache tạm: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cache_cleanup_removes_only_children_in_the_target_directory() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("cache");
        let outside = temp.path().join("outside.txt");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("temporary.bin"), b"temp").unwrap();
        fs::write(&outside, b"keep").unwrap();
        remove_children(&target).unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        assert_eq!(directory_size(&target), 0);
    }

    #[cfg(unix)]
    #[test]
    fn cache_cleanup_refuses_symlinked_directory_roots() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let outside = temp.path().join("outside");
        let link = temp.path().join("cache-link");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        symlink(&outside, &link).unwrap();
        assert!(remove_children(&link).is_err());
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(directory_size(&link), 0);
    }
}
