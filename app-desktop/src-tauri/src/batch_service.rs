use crate::project_service::ProjectStore;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

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
    if !path.exists() && !crate::recovery_store::backup_path(&path,1).exists() {
        return Ok(None);
    }
    let mut queue:BatchVideoQueue=crate::recovery_store::read(&path,Some(QUEUE_SCHEMA_VERSION),|bytes|{
        let queue=serde_json::from_slice(bytes).map_err(|e|format!("Dữ liệu hàng đợi video bị lỗi: {e}"))?;
        validate(&queue,project_id)?;Ok(queue)
    })?;

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
    for file in std::iter::once(path.clone()).chain((1..=3).map(|i|crate::recovery_store::backup_path(&path,i))) {
        if file.exists() {fs::remove_file(file).map_err(|error|format!("Không thể xóa hàng đợi video: {error}"))?;}
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
    store.get(project_id)?;
    Ok(store.data_directory(project_id,"cache")?.join(BATCH_QUEUE_FILE))
}

fn persist(path: &Path, queue: &BatchVideoQueue) -> Result<(), String> {
    let contents=serde_json::to_vec_pretty(queue).map_err(|e|e.to_string())?;
    crate::recovery_store::write(path,&contents)
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
