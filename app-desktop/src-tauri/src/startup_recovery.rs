//! Clean only journalled interrupted render jobs. STTN patch/model caches are
//! resumable and intentionally retained. External output folders are untouched.
use crate::{job_service::JobRecord, project_service::ProjectStore};
use std::fs;
use std::path::{Path, PathBuf};

pub fn cleanup(store: &ProjectStore, jobs: &[JobRecord]) {
    for job in jobs
        .iter()
        .filter(|job| job.status == "interrupted" && job.job_type == "export")
    {
        if let Err(error) = cleanup_job(store, job) {
            log::warn!(
                "Interrupted export cleanup deferred; job={}: {error}",
                job.id
            );
        }
    }
}

fn safe_directory(root: &Path, relative: &str) -> Result<Option<PathBuf>, String> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        if component.is_empty() || component == ".." {
            return Err("Đường dẫn dọn tạm không an toàn.".into());
        }
        path.push(component);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
            Ok(m) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err("Bỏ qua liên kết thư mục.".into());
                    }
                }
                if !m.is_dir() || m.file_type().is_symlink() {
                    return Err("Bỏ qua đường dẫn không an toàn.".into());
                }
            }
        }
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("Bỏ qua thư mục ngoài dự án.".into());
    }
    Ok(Some(canonical))
}

fn cleanup_job(store: &ProjectStore, job: &JobRecord) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&job.id)
        .map_err(|_| "Job ID không hợp lệ.")?
        .to_string();
    if id != job.id {
        return Err("Job ID không hợp lệ.".into());
    }
    let project_id = job.project_id.as_deref().ok_or("Job không có dự án.")?;
    let root = store
        .project_directory(project_id)?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let Some(render) = safe_directory(&root, &format!("cache/render/{id}"))? else {
        return Ok(());
    };
    crate::project_service::verify_tree_has_no_reparse_points(&render)?;
    if let Ok(bytes) = fs::read(render.join("temporary-output.json")) {
        if let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(output) = manifest["outputPath"].as_str() {
                let path = PathBuf::from(output);
                if let Some(exports) = safe_directory(&root, "exports")? {
                    let prefix = format!(".avl-render-{id}-");
                    if path.parent().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&exports)
                        && path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".partial.mp4"))
                    {
                        if let Ok(metadata) = fs::symlink_metadata(&path) {
                            #[cfg(windows)]
                            {
                                use std::os::windows::fs::MetadataExt;
                                if metadata.file_attributes() & 0x400 != 0 {
                                    return Err("Bỏ qua tệp liên kết.".into());
                                }
                            }
                            if metadata.is_file() && !metadata.file_type().is_symlink() {
                                fs::remove_file(&path).map_err(|e| e.to_string())?;
                            }
                        }
                    }
                }
            }
        }
    }
    fs::remove_dir_all(render).map_err(|e| e.to_string())?;
    log::info!("Removed interrupted export temp; job={id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(project: &str, id: &str) -> JobRecord {
        JobRecord {
            id: id.into(),
            project_id: Some(project.into()),
            project_name: None,
            job_type: "export".into(),
            status: "interrupted".into(),
            priority: "normal".into(),
            resources: vec![],
            progress: 0.2,
            message: String::new(),
            created_at: String::new(),
            started_at: None,
            completed_at: None,
            attempt: 1,
            retry_of: None,
            error: None,
            retry_payload: None,
        }
    }
    #[test]
    fn cleanup_preserves_final_output_source_and_external_partial() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let project = store.create("Phục hồi").unwrap();
        let root = store.project_directory(&project.id).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let render = root.join("cache/render").join(&id);
        fs::create_dir_all(&render).unwrap();
        let external = dir
            .path()
            .join(format!(".avl-render-{id}-test.partial.mp4"));
        fs::write(&external, b"partial").unwrap();
        fs::write(root.join("exports/final.mp4"), b"good").unwrap();
        fs::write(
            render.join("temporary-output.json"),
            serde_json::to_vec(&serde_json::json!({"outputPath":external})).unwrap(),
        )
        .unwrap();
        cleanup_job(&store, &job(&project.id, &id)).unwrap();
        assert!(external.exists());
        assert!(root.join("exports/final.mp4").exists());
        assert!(!render.exists());
    }
    #[test]
    fn cleanup_removes_only_owned_project_partial() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let p = store.create("Recovery").unwrap();
        let root = store.project_directory(&p.id).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let render = root.join("cache/render").join(&id);
        fs::create_dir_all(&render).unwrap();
        let partial = root
            .join("exports")
            .join(format!(".avl-render-{id}-random.partial.mp4"));
        fs::write(&partial, b"x").unwrap();
        fs::write(
            render.join("temporary-output.json"),
            serde_json::to_vec(&serde_json::json!({"outputPath":partial})).unwrap(),
        )
        .unwrap();
        cleanup_job(&store, &job(&p.id, &id)).unwrap();
        assert!(!partial.exists());
    }
}
