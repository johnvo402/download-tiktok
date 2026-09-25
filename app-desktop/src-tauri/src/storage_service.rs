use crate::job_service::{is_active, JobStore};
use crate::project_service::ProjectStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub models_bytes: u64,
    pub faster_whisper_bytes: u64,
    pub ocr_bytes: u64,
    pub vie_neu_tts_bytes: u64,
    pub other_model_data_bytes: u64,
    pub projects_bytes: u64,
    pub project_cache_bytes: u64,
    pub temporary_bytes: u64,
    pub logs_bytes: u64,
    pub cache_directory: String,
    pub faster_whisper_directory: String,
    pub ocr_directory: String,
    pub vie_neu_tts_directory: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum ModelFolderKind {
    #[serde(rename = "fasterWhisper")]
    FasterWhisper,
    #[serde(rename = "vieNeuTts")]
    VieNeuTts,
    #[serde(rename = "ocr")]
    Ocr,
}

/// Move only application-owned model/cache directories from Tauri's Local
/// cache root into the single Roaming app-data root. The old Roaming model
/// cache is merged too. WebView2's own profile is intentionally not touched.
pub fn migrate_local_storage(app: &AppHandle) -> Result<Vec<String>, String> {
    let data_root = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let local_root = app.path().app_cache_dir().map_err(|error| error.to_string())?;
    let mappings = [
        (local_root.join("fw"), data_root.join("models").join("fw")),
        (data_root.join("models").join("faster-whisper"), data_root.join("models").join("fw")),
        (local_root.join("downloads"), data_root.join("cache").join("downloads")),
        (local_root.join("tmp"), data_root.join("cache").join("tmp")),
    ];
    let mut warnings = Vec::new();
    for (source, target) in mappings {
        if source.exists() {
            if let Err(error) = move_managed_directory(&source, &target) {
                warnings.push(format!("Không thể chuyển {} vào thư mục dữ liệu chung: {error}", source.display()));
            }
        }
    }
    Ok(warnings)
}

fn move_managed_directory(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("thư mục nguồn không an toàn hoặc không hợp lệ".into());
    }
    if let Some(parent) = target.parent() { fs::create_dir_all(parent).map_err(|error| error.to_string())?; }
    if target.exists() {
        let target_metadata = fs::symlink_metadata(target).map_err(|error| error.to_string())?;
        if target_metadata.file_type().is_symlink() || is_reparse_point(&target_metadata) || !target_metadata.is_dir() {
            return Err("đích đã tồn tại nhưng không phải thư mục dữ liệu an toàn".into());
        }
        merge_directory_contents(source, target)?;
        fs::remove_dir(source).map_err(|error| error.to_string())?;
        return Ok(());
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            // Cross-volume move fallback: copy each regular file, verify its
            // length, then remove the source only after every copy succeeded.
            let staging = target.with_file_name(format!(".{}-migration", target.file_name().unwrap_or_default().to_string_lossy()));
            if staging.exists() { return Err(format!("còn thư mục di chuyển tạm {} từ lần trước; dữ liệu nguồn được giữ nguyên", staging.display())); }
            fs::create_dir_all(&staging).map_err(|error| format!("{rename_error}; {error}"))?;
            if let Err(error) = copy_managed_tree(source, &staging) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("sao chép chéo ổ đĩa thất bại: {error}"));
            }
            fs::rename(&staging, target).map_err(|error| format!("không thể hoàn tất thư mục đích sau khi sao chép: {error}"))?;
            fs::remove_dir_all(source).map_err(|error| format!("Đã sao chép dữ liệu nhưng chưa xóa được cache cũ: {error}"))?;
            Ok(())
        }
    }
}

fn merge_directory_contents(source: &Path, target: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let from = entry.path();
        let mut to = target.join(entry.file_name());
        let source_meta = fs::symlink_metadata(&from).map_err(|error| error.to_string())?;
        if source_meta.file_type().is_symlink() || is_reparse_point(&source_meta) {
            return Err("gặp symlink/reparse point; dữ liệu nguồn được giữ lại".into());
        }
        if !to.exists() {
            if fs::rename(&from, &to).is_ok() { continue; }
            if source_meta.is_dir() {
                fs::create_dir_all(&to).map_err(|error| error.to_string())?;
                merge_directory_contents(&from, &to)?;
                fs::remove_dir(&from).map_err(|error| error.to_string())?;
            } else if source_meta.is_file() {
                fs::copy(&from, &to).map_err(|error| error.to_string())?;
                if fs::metadata(&to).map_err(|error| error.to_string())?.len() != source_meta.len() {
                    return Err("kích thước file sau sao chép không khớp".into());
                }
                fs::remove_file(&from).map_err(|error| error.to_string())?;
            }
            continue;
        }
        let destination_meta = fs::symlink_metadata(&to).map_err(|error| error.to_string())?;
        if destination_meta.file_type().is_symlink() || is_reparse_point(&destination_meta) {
            return Err("đích chứa symlink/reparse point; dữ liệu nguồn được giữ lại".into());
        }
        if source_meta.is_dir() && destination_meta.is_dir() {
            merge_directory_contents(&from, &to)?;
            fs::remove_dir(&from).map_err(|error| error.to_string())?;
        } else if source_meta.is_file() && destination_meta.is_file() && same_file_contents(&from, &to)? {
            fs::remove_file(&from).map_err(|error| error.to_string())?;
        } else {
            to = unique_legacy_path(&to);
            fs::rename(&from, &to).map_err(|error| format!("không thể giữ mục trùng tên mà không ghi đè: {error}"))?;
        }
    }
    Ok(())
}

fn unique_legacy_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut candidate = path.with_file_name(format!("{name}.legacy"));
    let mut index = 2_u32;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.legacy-{index}"));
        index += 1;
    }
    candidate
}

fn same_file_contents(left: &Path, right: &Path) -> Result<bool, String> {
    if fs::metadata(left).map_err(|error| error.to_string())?.len() != fs::metadata(right).map_err(|error| error.to_string())?.len() {
        return Ok(false);
    }
    fn digest(path: &Path) -> Result<[u8; 32], String> {
        let mut file = File::open(path).map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 { break; }
            hasher.update(&buffer[..count]);
        }
        Ok(hasher.finalize().into())
    }
    Ok(digest(left)? == digest(right)?)
}

fn copy_managed_tree(source: &Path, target: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) { return Err("gặp symlink/reparse point; giữ lại cache cũ".into()); }
        if metadata.is_dir() {
            fs::create_dir_all(&to).map_err(|error| error.to_string())?;
            copy_managed_tree(&from, &to)?;
        } else if metadata.is_file() {
            fs::copy(&from, &to).map_err(|error| error.to_string())?;
            let copied = fs::metadata(&to).map_err(|error| error.to_string())?;
            if copied.len() != metadata.len() { return Err("kích thước file sau sao chép không khớp".into()); }
        }
    }
    Ok(())
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x0400 != 0;
    }
    #[cfg(not(windows))] { let _ = metadata; false }
}

pub fn usage(app: &AppHandle, jobs: &JobStore) -> Result<StorageUsage, String> {
    let data_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let app_cache = data_root.join("cache");
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
    let faster_whisper_directory = app_models.join("fw");
    let ocr_directory = app_models.join("rapidocr");
    let vie_neu_tts_directory = app_models.join("huggingface");
    let faster_whisper_bytes = directory_size(&faster_whisper_directory);
    let ocr_bytes = directory_size(&ocr_directory);
    let vie_neu_tts_bytes = directory_size(&vie_neu_tts_directory);
    let models_bytes = directory_size(&app_models)
        .saturating_sub(directory_size(&app_models.join("tts-preview")));
    let other_model_data_bytes = models_bytes
        .saturating_sub(faster_whisper_bytes)
        .saturating_sub(ocr_bytes)
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
        ocr_bytes,
        vie_neu_tts_bytes,
        other_model_data_bytes,
        projects_bytes,
        project_cache_bytes: project_cache_all.saturating_sub(project_temporary),
        temporary_bytes: project_temporary.saturating_add(global_temporary),
        logs_bytes,
        cache_directory: app_cache.to_string_lossy().into_owned(),
        faster_whisper_directory: faster_whisper_directory.to_string_lossy().into_owned(),
        ocr_directory: ocr_directory.to_string_lossy().into_owned(),
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
    let app_cache = data_root.join("cache");
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
    let folder = app.path().app_data_dir().map_err(|error| error.to_string())?.join("cache");
    open_directory(&folder)?;
    Ok(folder.to_string_lossy().into_owned())
}

pub fn open_model_folder(app: &AppHandle, kind: ModelFolderKind) -> Result<String, String> {
    let folder = match kind {
        ModelFolderKind::FasterWhisper => app
            .path().app_data_dir().map_err(|error| error.to_string())?.join("models").join("fw"),
        ModelFolderKind::VieNeuTts => app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("models")
            .join("huggingface"),
        ModelFolderKind::Ocr => app.path().app_data_dir().map_err(|error| error.to_string())?.join("models").join("rapidocr"),
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
