use crate::worker_manager::WorkerCommandExt;
use super::{
    manager::{Status, Transform, TranslationPatch},
    scene, Bounds, PreviewManager,
};
use crate::{overlays_service::VisualSettings, render_spec::PixelRect};
use tauri::Manager;
/// Audio-only compatibility file. Never transcodes video for native playback.
#[tauri::command]
pub async fn preview_audio_path(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = crate::commands::store(&app)?;
        let project = store.get(&project_id)?;
        let source = crate::video_service::resolve_video(&store, &project_id)?;
        let root = store.project_directory(&project_id)?;
        let dir = store.data_directory(&project_id,"cache/preview")?;
        let canonical_root = root.canonicalize().map_err(|_| "Dự án không tồn tại.")?;
        let dir = dir
            .canonicalize()
            .map_err(|_| "Không xác minh được thư mục audio preview.")?;
        if !dir.starts_with(&canonical_root) {
            return Err("Thư mục audio nằm ngoài dự án.".into());
        }
        let checksum = project
            .source
            .checksum_sha256
            .as_deref()
            .ok_or("Video chưa có checksum.")?;
        if !checksum.chars().all(|c| c.is_ascii_hexdigit()) || checksum.len() > 128 {
            return Err("Checksum video không hợp lệ.".into());
        }
        let output = dir.join(format!("audio-{checksum}.m4a"));
        if !output.is_file() {
            let temp = tempfile::Builder::new()
                .suffix(".m4a")
                .tempfile_in(&dir)
                .map_err(|_| "Không tạo được tệp audio tạm.")?;
            let status = crate::runtime_paths::ffmpeg_command()?
                .args(["-v", "error", "-y", "-i"])
                .arg(source)
                .args(["-map", "0:a:0", "-vn", "-c:a", "aac", "-b:a", "192k"])
                .arg(temp.path())
                .output_contained(std::time::Duration::from_secs(7200))
                .map_err(|_| "Không tạo được audio preview.")?;
            if !status.status.success() {
                return Err("Không chuyển đổi được âm thanh video.".into());
            }
            temp.persist_noclobber(&output)
                .map_err(|_| "Không lưu được audio preview.")?;
        }
        let path = crate::video_service::resolve_project_file(
            &store,
            &project_id,
            output
                .strip_prefix(&canonical_root)
                .map_err(|_| "Audio nằm ngoài dự án.")?
                .to_str()
                .ok_or("Đường dẫn audio không hợp lệ.")?,
            "cache/preview",
        )?;
        app.asset_protocol_scope()
            .allow_file(&path)
            .map_err(|_| "Không cấp được quyền đọc audio preview.")?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "Không tạo được audio preview.")?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanPreview {
    path: String,
    start_seconds: f64,
    end_seconds: f64,
}
#[tauri::command]
pub async fn preview_open(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_id: String,
    bounds: Bounds,
    clean: Option<CleanPreview>,
    timeline_clip_id: Option<String>,
) -> Result<String, String> {
    if std::env::var("JC_PREVIEW_BACKEND").is_ok_and(|value| value == "compatibility") {
        return Err("Đã bật chế độ xem video tương thích cho phiên này.".into());
    }
    bounds.validate()?;
    if window.label() != "main" {
        return Err("Cửa sổ không hỗ trợ preview.".into());
    }
    #[cfg(windows)]
    let parent = window.hwnd().map_err(|_| "Không xác định được cửa sổ.")?.0 as isize;
    #[cfg(not(windows))]
    let parent = 0;
    let owner = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = crate::commands::store(&app)?;
        let mut project = store.get(&project_id)?;
        let external_clock = timeline_clip_id.is_some();
        if let Some(id) = timeline_clip_id {
            if clean.is_some() { return Err("Không kết hợp clip timeline với preview AI.".into()); }
            (store, project) = timeline_project(&store, &project_id, &id)?;
        }
        if project
            .source
            .video_pixel_format
            .as_deref()
            .is_some_and(|f| f.contains("10") || f.contains("12") || f.contains("16"))
        {
            return Err("Video màu 10-bit/HDR cần preview tương thích.".into());
        }
        let transcript = crate::transcript_service::get_transcript(&store, &project_id)?
            .unwrap_or_else(|| scene::empty_transcript(&project));
        let (source, offset, end) = if let Some(clean) = clean {
            if !clean.start_seconds.is_finite()
                || !clean.end_seconds.is_finite()
                || clean.start_seconds < 0.0
                || clean.end_seconds <= clean.start_seconds
                || clean.end_seconds > project.source.duration.unwrap_or(0.0) + 0.1
            {
                return Err("Đoạn preview AI không hợp lệ.".into());
            }
            let root = store
                .project_directory(&project_id)?
                .canonicalize()
                .map_err(|_| "Dự án không tồn tại.")?;
            let path = std::path::PathBuf::from(&clean.path)
                .canonicalize()
                .map_err(|_| "Không tìm thấy preview AI.")?;
            let cache = root
                .join("cache")
                .canonicalize()
                .map_err(|_| "Không tìm thấy cache.")?;
            if !cache.starts_with(&root) || !path.starts_with(&cache) || !path.is_file() {
                return Err("Preview AI nằm ngoài dự án.".into());
            }
            (path, clean.start_seconds, clean.end_seconds)
        } else {
            (
                crate::video_service::resolve_video(&store, &project_id)?,
                0.0,
                project.source.duration.ok_or("Video thiếu thời lượng.")?,
            )
        };
        app.state::<PreviewManager>().open(
            parent,
            owner,
            app.clone(),
            store,
            project,
            transcript,
            source,
            offset,
            end,
            bounds,
            external_clock,
        )
    })
    .await
    .map_err(|_| "Không mở được preview.")?
}
fn timeline_content(project: &crate::project_service::Project, clip_id: &str) -> Result<String, String> {
    let clip = project.timeline.as_ref().and_then(|doc| doc.clips.iter().find(|c| c.id == clip_id)).ok_or("Clip không tồn tại trong dự án.")?;
    match &clip.content {
        crate::timeline::ClipContent::Video { video_content_id, .. } => Ok(video_content_id.clone()),
        _ => Err("Native preview chỉ nhận clip video.".into()),
    }
}
fn timeline_project(store: &crate::project_service::ProjectStore, project_id: &str, clip_id: &str) -> Result<(crate::project_service::ProjectStore, crate::project_service::Project), String> {
    let content_id = timeline_content(&store.get(project_id)?, clip_id)?;
    let scoped = store.for_video(project_id, Some(&content_id))?;
    let mut project = scoped.get(project_id)?;
    // Only the timeline scene owns captions/overlays. This raw acceleration view
    // never persists changes to the source editor or its legacy binding.
    project.subtitle.enabled = false;
    project.blur_regions.clear(); project.logos.clear(); project.text_overlays.clear();
    Ok((scoped, project))
}

#[cfg(test)]
mod timeline_tests {
    use super::*;
    #[test]
    fn timeline_native_resolves_each_clip_source_and_keeps_the_saved_editor_untouched() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let before = serde_json::to_value(store.get(&project.id).unwrap()).unwrap();
        let doc = project.timeline.as_ref().unwrap();
        for (clip, content, checksum) in [(&doc.clips[0], &a, "a".repeat(64)), (&doc.clips[1], &b, "b".repeat(64))] {
            let (scoped, view) = timeline_project(&store, &project.id, &clip.id).unwrap();
            assert_eq!(scoped.content_id(), Some(content.as_str()));
            assert_eq!(view.source.checksum_sha256.as_deref(), Some(checksum.as_str()));
            assert!(!view.subtitle.enabled && view.blur_regions.is_empty() && view.logos.is_empty() && view.text_overlays.is_empty());
        }
        assert!(timeline_project(&store, &project.id, "missing").is_err());
        let mut wrong = project.clone();
        wrong.timeline.as_mut().unwrap().clips[0].content = crate::timeline::ClipContent::Text { text: "title".into() };
        assert!(timeline_content(&wrong, &doc.clips[0].id).is_err());
        assert_eq!(serde_json::to_value(store.get(&project.id).unwrap()).unwrap(), before);
    }
}
#[tauri::command]
pub async fn preview_close(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<(), String> {
    let owner = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PreviewManager>()
            .close(Some(&session_id), Some(&owner));
    })
    .await
    .map_err(|_| "Không đóng được preview.")?;
    Ok(())
}
#[tauri::command]
pub fn preview_control(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    session_id: String,
    action: String,
    time: Option<f64>,
) -> Result<(), String> {
    let s = app
        .state::<PreviewManager>()
        .get(&session_id, window.label())?;
    let mut updates = s.updates.lock().map_err(|_| "Preview bận.")?;
    match action.as_str() {
        "play" => {
            if s.external_clock { return Err("Timeline điều khiển đồng hồ của phiên preview này.".into()); }
            let status = s.status.lock().map_err(|_| "Preview bận.")?;
            if status.position >= s.range.1 {
                let generation = s
                    .generation
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                updates.seek = Some((generation, s.range.0));
            }
            updates.playing = Some(true);
        }
        "pause" => updates.playing = Some(false),
        "seek" | "audio" => {
            let t = time.ok_or("Thiếu timestamp.")?;
            let duration = s.status.lock().map_err(|_| "Preview bận.")?.duration;
            if !t.is_finite() || t < 0.0 || t > duration + 0.001 {
                return Err("Timestamp không hợp lệ.".into());
            }
            if action == "seek" {
                let generation = s
                    .generation
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                updates.seek = Some((generation, t));
            } else {
                updates.audio = Some(t);
            }
        }
        _ => return Err("Lệnh phát không hợp lệ.".into()),
    };
    drop(updates);
    let _ = s.wake.try_send(());
    Ok(())
}
#[tauri::command]
pub fn preview_resize(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    session_id: String,
    bounds: Bounds,
) -> Result<(), String> {
    bounds.validate()?;
    let s = app
        .state::<PreviewManager>()
        .get(&session_id, window.label())?;
    s.updates.lock().map_err(|_| "Preview bận.")?.bounds = Some(bounds);
    let _ = s.wake.try_send(());
    Ok(())
}
#[tauri::command]
pub fn preview_update_scene(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    session_id: String,
    version: u64,
    visual: Option<VisualSettings>,
    translations: Option<Vec<TranslationPatch>>,
    guide: Option<PixelRect>,
    transform: Option<Transform>,
) -> Result<(), String> {
    let s = app
        .state::<PreviewManager>()
        .get(&session_id, window.label())?;
    if translations.as_ref().is_some_and(|p| {
        p.len() > 100_000
            || p.iter().any(|s| {
                s.translated.len() > 100_000
                    || s.id.is_empty()
                    || !s.start.is_finite()
                    || !s.end.is_finite()
                    || s.start < 0.0
                    || s.end <= s.start
            })
    }) {
        return Err("Cập nhật transcript quá lớn.".into());
    }
    let duration = s.status.lock().map_err(|_| "Preview bận.")?.duration;
    if translations
        .as_ref()
        .is_some_and(|patches| patches.iter().any(|p| p.end > duration + 0.001))
    {
        return Err("Timestamp vượt thời lượng video.".into());
    }
    if let Some(v) = &visual {
        crate::overlays_service::validate_subtitle_settings(&v.subtitle)?;
        crate::overlays_service::validate_blur_regions(&v.blur_regions)?;
        crate::overlays_service::validate_text_overlays(&v.text_overlays)?;
    }
    let mut updates = s.updates.lock().map_err(|_| "Preview bận.")?;
    if version <= s.scene_version.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    s.scene_version
        .store(version, std::sync::atomic::Ordering::SeqCst);
    if let Some(t) = transform {
        if !["blur", "logo", "text", "subtitle"].contains(&t.kind.as_str())
            || ![t.x, t.y, t.width, t.height].iter().all(|v| v.is_finite())
            || t.x < 0.0
            || t.y < 0.0
            || t.width <= 0.0
            || t.height <= 0.0
            || t.x + t.width > 1.000001
            || t.y + t.height > 1.000001
        {
            return Err("Vùng hiệu ứng không hợp lệ.".into());
        }
        updates.transforms.insert(format!("{}:{}", t.kind, t.id), t);
    }
    if let Some(rect) = guide {
        if rect.width == 0
            || rect.height == 0
            || rect.x.saturating_add(rect.width) > s.size.0
            || rect.y.saturating_add(rect.height) > s.size.1
        {
            return Err("Vùng chọn không hợp lệ.".into());
        }
    }
    updates.patch_scene(version, visual, translations);
    updates.guide = Some(guide);
    drop(updates);
    let _ = s.wake.try_send(());
    Ok(())
}
#[tauri::command]
pub fn preview_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<Status, String> {
    let s = app
        .state::<PreviewManager>()
        .get(&session_id, window.label())?;
    let result = s.status.lock().map_err(|_| "Preview bận.")?.clone();
    Ok(result)
}
