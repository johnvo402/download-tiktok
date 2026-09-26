use crate::project_service::ProjectStore;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const BATCH_QUEUE_FILE: &str = "batch-video-queue.json";
const QUEUE_SCHEMA_VERSION: u32 = 1;
const MAX_QUEUE_ITEMS: usize = 500;
const MAX_PATH_LENGTH: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatchVideoQueue {
    pub schema_version: u32,
    pub project_id: String,
    pub sample_id: String,
    pub sample_loaded: bool,
    #[serde(default)]
    pub sample_checksum: Option<String>,
    #[serde(default)]
    pub output_directory: Option<String>,
    pub updated_at: String,
    pub items: Vec<BatchVideoItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatchVideoItem {
    pub id: String,
    pub source_path: String,
    pub file_name: String,
    pub status: String,
    pub stage: Option<String>,
    pub progress: f64,
    pub message: String,
    pub output_path: Option<String>,
}

pub fn load(store: &ProjectStore, project_id: &str) -> Result<Option<BatchVideoQueue>, String> {
    let path = queue_path(store, project_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("Không thể đọc hàng đợi video: {error}"))?;
    let mut queue: BatchVideoQueue = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Dữ liệu hàng đợi video bị lỗi: {error}"))?;
    validate(&queue, project_id)?;

    let mut recovered = false;
    for item in &mut queue.items {
        if item.status == "running" {
            item.status = "queued".into();
            item.stage = None;
            item.progress = 0.0;
            item.message = "Ứng dụng đã đóng trước khi hoàn tất; video sẵn sàng chạy lại.".into();
            recovered = true;
        }
    }
    if recovered {
        queue.updated_at = timestamp();
        persist(&path, &queue)?;
    }
    Ok(Some(queue))
}

pub fn save(
    store: &ProjectStore,
    project_id: &str,
    mut queue: BatchVideoQueue,
) -> Result<BatchVideoQueue, String> {
    validate(&queue, project_id)?;
    queue.schema_version = QUEUE_SCHEMA_VERSION;
    queue.updated_at = timestamp();
    let path = queue_path(store, project_id)?;
    persist(&path, &queue)?;
    Ok(queue)
}

pub fn clear(store: &ProjectStore, project_id: &str) -> Result<(), String> {
    let path = queue_path(store, project_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("Không thể xóa hàng đợi video: {error}"))?;
    }
    Ok(())
}

fn validate(queue: &BatchVideoQueue, project_id: &str) -> Result<(), String> {
    let canonical_id = uuid::Uuid::parse_str(project_id)
        .map_err(|_| "Mã dự án không hợp lệ.".to_string())?
        .to_string();
    if canonical_id != project_id || queue.project_id != project_id {
        return Err("Hàng đợi không thuộc dự án hiện tại.".into());
    }
    if queue.schema_version > QUEUE_SCHEMA_VERSION {
        return Err("Phiên bản hàng đợi video mới hơn phiên bản ứng dụng này.".into());
    }
    if queue.items.len() > MAX_QUEUE_ITEMS {
        return Err(format!(
            "Hàng đợi vượt quá giới hạn {MAX_QUEUE_ITEMS} video."
        ));
    }
    if queue.items.is_empty() {
        return Err("Hàng đợi phải có ít nhất một video.".into());
    }
    let mut ids = HashSet::new();
    for item in &queue.items {
        if item.id.trim().is_empty() || !ids.insert(item.id.as_str()) {
            return Err("Mã video trong hàng đợi bị thiếu hoặc trùng.".into());
        }
        if item.source_path.trim().is_empty()
            || item.source_path.len() > MAX_PATH_LENGTH
            || item.source_path.chars().any(char::is_control)
        {
            return Err("Đường dẫn video trong hàng đợi không hợp lệ.".into());
        }
        if item.file_name.trim().is_empty() || item.file_name.len() > 512 {
            return Err("Tên video trong hàng đợi không hợp lệ.".into());
        }
        if !matches!(
            item.status.as_str(),
            "queued" | "running" | "completed" | "failed" | "cancelled"
        ) {
            return Err("Trạng thái video trong hàng đợi không hợp lệ.".into());
        }
        if !item.progress.is_finite() || !(0.0..=1.0).contains(&item.progress) {
            return Err("Tiến độ video trong hàng đợi không hợp lệ.".into());
        }
    }
    if !ids.contains(queue.sample_id.as_str()) {
        return Err("Không tìm thấy video mẫu trong hàng đợi.".into());
    }
    if queue.output_directory.as_ref().is_some_and(|path| {
        path.trim().is_empty() || path.len() > MAX_PATH_LENGTH || path.chars().any(char::is_control)
    }) {
        return Err("Thư mục export của hàng đợi không hợp lệ.".into());
    }
    if queue.sample_checksum.as_ref().is_some_and(|checksum| {
        checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err("Checksum của video mẫu không hợp lệ.".into());
    }
    Ok(())
}

fn queue_path(store: &ProjectStore, project_id: &str) -> Result<PathBuf, String> {
    // Resolve through ProjectStore first: this validates the UUID and any registered custom root.
    let project = store.get(project_id)?;
    let project_dir = store.project_directory(&project.id)?;
    let canonical_project = project_dir
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục dự án: {error}"))?;
    let cache = canonical_project.join("cache");
    if !cache.exists() {
        fs::create_dir_all(&cache)
            .map_err(|error| format!("Không thể tạo thư mục cache project: {error}"))?;
    }
    let metadata = fs::symlink_metadata(&cache)
        .map_err(|error| format!("Không thể xác minh thư mục cache project: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Thư mục cache của project không an toàn.".into());
    }
    let canonical_cache = cache
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục cache project: {error}"))?;
    if canonical_cache.parent() != Some(canonical_project.as_path()) {
        return Err("Thư mục cache nằm ngoài project.".into());
    }
    Ok(canonical_cache.join(BATCH_QUEUE_FILE))
}

fn persist(path: &Path, queue: &BatchVideoQueue) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Không xác định được thư mục hàng đợi.".to_string())?;
    let contents = serde_json::to_vec_pretty(queue)
        .map_err(|error| format!("Không thể mã hóa hàng đợi video: {error}"))?;
    let mut temp = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Không thể tạo file hàng đợi tạm: {error}"))?;
    temp.write_all(&contents)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|error| format!("Không thể ghi hàng đợi video: {error}"))?;
    temp.persist(path)
        .map_err(|error| format!("Không thể lưu hàng đợi video: {}", error.error))?;
    Ok(())
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::ProjectStore;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn queue(project_id: &str) -> BatchVideoQueue {
        BatchVideoQueue {
            schema_version: 1,
            project_id: project_id.into(),
            sample_id: "item-1".into(),
            sample_loaded: false,
            sample_checksum: None,
            output_directory: None,
            updated_at: timestamp(),
            items: vec![BatchVideoItem {
                id: "item-1".into(),
                source_path: "C:\\videos\\sample.mp4".into(),
                file_name: "sample.mp4".into(),
                status: "queued".into(),
                stage: None,
                progress: 0.0,
                message: "Đang chờ".into(),
                output_path: None,
            }],
        }
    }

    #[test]
    fn queue_persists_inside_project_and_recovers_interrupted_items() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Batch test").unwrap();
        let mut value = queue(&project.id);
        value.items[0].status = "running".into();
        save(&store, &project.id, value).unwrap();

        let loaded = load(&store, &project.id).unwrap().unwrap();
        assert_eq!(loaded.items[0].status, "queued");
        assert!(loaded.items[0].message.contains("đã đóng"));
        assert!(store
            .project_directory(&project.id)
            .unwrap()
            .join("cache")
            .join(BATCH_QUEUE_FILE)
            .exists());
    }

    #[test]
    fn queue_rejects_path_like_project_ids_and_duplicate_item_ids() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Batch test").unwrap();
        assert!(save(&store, "..", queue("..")).is_err());

        let mut value = queue(&project.id);
        let mut duplicate = value.items[0].clone();
        duplicate.source_path = "C:\\videos\\other.mp4".into();
        value.items.push(duplicate);
        assert!(save(&store, &project.id, value)
            .unwrap_err()
            .contains("trùng"));
    }

    #[test]
    fn queue_only_loads_for_the_project_that_owns_it() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Batch test").unwrap();
        let wrong = Uuid::new_v4().to_string();
        assert!(save(&store, &project.id, queue(&wrong))
            .unwrap_err()
            .contains("không thuộc"));
    }
}
