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
    // Timeline captures live in a sibling TempDir, while the job journal uses its UUID.
    if let Ok(bytes) = fs::read(render.join("timeline-snapshot.json")) {
        let manifest: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let name = manifest["directory"]
            .as_str()
            .ok_or("Thiếu thư mục snapshot.")?;
        if !name.starts_with("timeline-")
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Thư mục snapshot không an toàn.".into());
        }
        if let Some(snapshot) = safe_directory(&root, &format!("cache/render/{name}"))? {
            crate::project_service::verify_tree_has_no_reparse_points(&snapshot)?;
            fs::remove_dir_all(snapshot).map_err(|e| e.to_string())?;
        }
    }
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
    fn timeline_snapshot_journal_survives_restart_and_cleanup_preserves_sources() {
        let (dir, store, mut p, _, _) = crate::video_content::tests::fixture();
        let root = store.project_directory(&p.id).unwrap();
        p.source.checksum_sha256 = None;
        for asset in &mut p.timeline.as_mut().unwrap().assets {
            asset.metadata.checksum_sha256 = None;
            fs::write(root.join(&asset.relative_path), b"immutable source").unwrap();
        }
        p = store.update_project(&p).unwrap();
        let opts = crate::render_spec::ExportOptions {
            encoder: crate::hardware_acceleration::VideoEncoder::Cpu,
            timeline: Some(crate::timeline_export::TimelineExportOptions {
                width: 320,
                height: 180,
                frame_rate: crate::timeline::Rational {
                    numerator: 25,
                    denominator: 1,
                },
                start_us: None,
                end_us: None,
            }),
            ..Default::default()
        };
        let mut snapshot = crate::timeline_export::capture(&store, &p.id, &opts).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        snapshot.bind_job(&store, &p.id, &id).unwrap();
        let journal = snapshot.journal.as_ref().unwrap().1.clone();
        let path = snapshot.directory.path().to_path_buf();
        let partial = root
            .join("exports")
            .join(format!(".avl-render-{id}-test.partial.mp4"));
        fs::write(&partial, b"not complete").unwrap();
        fs::write(root.join("exports/final.mp4"), b"verified").unwrap();
        crate::recovery_store::write(
            &journal.join("temporary-output.json"),
            &serde_json::to_vec(&serde_json::json!({"outputPath":partial})).unwrap(),
        )
        .unwrap();
        let jobs_file = dir.path().join("jobs.json");
        let before = crate::job_service::JobStore::default();
        before.initialize(jobs_file.clone()).unwrap();
        let mut record = job(&p.id, &id);
        record.status = "running".into();
        record.retry_payload = Some(serde_json::to_value(&opts).unwrap());
        before.register(record).unwrap();
        // Model a killed process: normal TempDir/Drop cleanup did not run.
        std::mem::forget(snapshot);
        let reopened = crate::job_service::JobStore::default();
        reopened.initialize(jobs_file).unwrap();
        let recovered = reopened.get(&id).unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert!(recovered.error.unwrap().retryable);
        let retry: crate::render_spec::ExportOptions =
            serde_json::from_value(recovered.retry_payload.unwrap()).unwrap();
        assert!(retry.timeline.is_some());
        cleanup(&store, &reopened.list().unwrap());
        assert!(!path.exists());
        assert!(!journal.exists());
        assert!(!partial.exists());
        assert_eq!(
            fs::read(root.join("exports/final.mp4")).unwrap(),
            b"verified"
        );
        for asset in &p.timeline.as_ref().unwrap().assets {
            assert_eq!(
                fs::read(root.join(&asset.relative_path)).unwrap(),
                b"immutable source"
            );
        }
        let fresh = crate::timeline_export::capture(&store, &p.id, &retry).unwrap();
        assert_eq!(fresh.plan.document, p.timeline.unwrap());
    }
    #[test]
    fn snapshot_journal_rejects_traversal_and_never_removes_an_active_job() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let p = store.create("Recovery").unwrap();
        let root = store.project_directory(&p.id).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let render = root.join("cache/render").join(&id);
        fs::create_dir_all(&render).unwrap();
        fs::write(
            render.join("timeline-snapshot.json"),
            br#"{"directory":"../source"}"#,
        )
        .unwrap();
        assert!(cleanup_job(&store, &job(&p.id, &id)).is_err());
        assert!(render.exists());
        fs::write(
            render.join("timeline-snapshot.json"),
            br#"{"directory":"timeline-keep"}"#,
        )
        .unwrap();
        let snapshot = root.join("cache/render/timeline-keep");
        fs::create_dir(&snapshot).unwrap();
        fs::write(snapshot.join("keep"), b"active").unwrap();
        let mut active = job(&p.id, &id);
        active.status = "running".into();
        cleanup(&store, &[active]);
        assert!(snapshot.join("keep").exists());
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
