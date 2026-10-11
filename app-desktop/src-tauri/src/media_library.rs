//! Owned media library. Source files and generated caches are addressed by asset ID.
use crate::project_service::{Project, ProjectSource, ProjectStore};
use crate::timeline::{
    self, Clip, ClipContent, ClipTransform, LegacyBinding, MediaAsset, MediaKind, Rational, Track,
    TrackKind, VideoContent, VideoSettings,
};
use crate::video_service::{copy_and_hash, probe_video, resolve_project_file, sha256_file};
use crate::worker_manager::WorkerCommandExt;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use uuid::Uuid;

static MEDIA_IO_LOCK: Mutex<()> = Mutex::new(());
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v"];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub source_path: String,
    pub asset_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBatch {
    pub project: Project,
    pub items: Vec<ImportItem>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetStatus {
    pub asset_id: String,
    pub state: String,
    pub message: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    pub schema_version: u32,
    pub checksum: String,
    pub duration_us: u64,
    pub peaks: Vec<f32>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetCache {
    pub thumbnail_path: Option<String>,
    pub waveform: Option<Waveform>,
}

fn valid_thumbnail(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.len() <= 2 * 1024 * 1024)
        && image::image_dimensions(path).is_ok_and(|(w, h)| w > 0 && h > 0 && w <= 320 && h <= 180)
        && image::open(path).is_ok()
}

fn asset<'a>(project: &'a Project, id: &str) -> Result<&'a MediaAsset, String> {
    project
        .timeline
        .as_ref()
        .and_then(|d| d.assets.iter().find(|a| a.id == id))
        .ok_or("Không tìm thấy nguồn media.".into())
}
fn kind_for_extension(extension: &str) -> Result<MediaKind, String> {
    if VIDEO_EXTENSIONS.contains(&extension) {
        Ok(MediaKind::Video)
    } else if IMAGE_EXTENSIONS.contains(&extension) {
        Ok(MediaKind::Image)
    } else if AUDIO_EXTENSIONS.contains(&extension) {
        Ok(MediaKind::Audio)
    } else {
        Err("Định dạng chưa được hỗ trợ. Chọn video, ảnh PNG/JPG/WEBP hoặc audio.".into())
    }
}
fn owned_file(
    store: &ProjectStore,
    project: &Project,
    source: &MediaAsset,
    verify: bool,
) -> Result<PathBuf, String> {
    let path = resolve_project_file(store, &project.id, &source.relative_path, "media")?;
    // Resolve every directory component without accepting junctions/reparse points.
    let parent = Path::new(&source.relative_path)
        .parent()
        .ok_or("Đường dẫn nguồn không hợp lệ.")?;
    store.data_directory(&project.id, &parent.to_string_lossy())?;
    let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if source
        .metadata
        .file_size
        .is_some_and(|expected| size != expected)
    {
        return Err("Tệp nguồn đã thay đổi dung lượng. Hãy tìm lại tệp gốc.".into());
    }
    if verify {
        if let Some(hash) = &source.metadata.checksum_sha256 {
            if !crate::file_integrity::verified(
                &path,
                source.metadata.file_size.unwrap_or(size),
                hash,
            ) {
                return Err(
                    "Tệp nguồn không còn khớp định danh ban đầu. Hãy tìm lại tệp gốc.".into(),
                );
            }
        }
    }
    Ok(path)
}
pub fn inspect(store: &ProjectStore, id: &str, verify: bool) -> Result<Vec<AssetStatus>, String> {
    let project = store.get(id)?;
    let doc = project.timeline.as_ref().ok_or("Thiếu timeline.")?;
    Ok(doc
        .assets
        .iter()
        .map(|source| {
            let missing = !store
                .project_directory(id)
                .map(|p| p.join(&source.relative_path).exists())
                .unwrap_or(false);
            match owned_file(store, &project, source, verify) {
                Ok(_) => AssetStatus {
                    asset_id: source.id.clone(),
                    state: "available".into(),
                    message: None,
                },
                Err(error) => AssetStatus {
                    asset_id: source.id.clone(),
                    state: if missing { "missing" } else { "invalid" }.into(),
                    message: Some(error),
                },
            }
        })
        .collect())
}

fn probe_asset(
    path: &Path,
    kind: MediaKind,
    id: String,
    relative_path: String,
    original_name: Option<String>,
    checksum: String,
) -> Result<MediaAsset, String> {
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    let probe = probe_video(path, size)?;
    match kind {
        MediaKind::Video if !probe.has_video || probe.duration.is_none() => {
            return Err("Tệp không có video hoặc không đọc được thời lượng.".into())
        }
        MediaKind::Audio
            if probe.has_video || probe.audio_streams.is_empty() || probe.duration.is_none() =>
        {
            return Err("Tệp không chứa nguồn audio hợp lệ.".into())
        }
        MediaKind::Image => {
            let dimensions = image::ImageReader::open(path)
                .map_err(|e| e.to_string())?
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .into_dimensions()
                .map_err(|_| "Ảnh bị lỗi hoặc không được hỗ trợ.")?;
            if dimensions.0 == 0
                || dimensions.1 == 0
                || dimensions.0 > 16384
                || dimensions.1 > 16384
                || u64::from(dimensions.0) * u64::from(dimensions.1) > 64_000_000
            {
                return Err("Ảnh quá lớn; tối đa 16.384 px mỗi chiều và 64 triệu pixel.".into());
            }
            image::ImageReader::open(path)
                .map_err(|e| e.to_string())?
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .decode()
                .map_err(|_| "Không giải mã được ảnh.")?;
        }
        _ => {}
    }
    let duration = if kind == MediaKind::Image {
        None
    } else {
        probe.duration
    };
    let fps = if kind == MediaKind::Video {
        probe.fps
    } else {
        None
    };
    Ok(MediaAsset {
        id,
        kind,
        relative_path: relative_path.clone(),
        duration_us: duration.map(timeline::seconds_to_time_us).transpose()?,
        frame_rate: fps.map(Rational::from_fps).transpose()?,
        variable_frame_rate: None,
        metadata: ProjectSource {
            video_path: Some(relative_path),
            original_name,
            duration,
            width: probe.width,
            height: probe.height,
            fps,
            video_codec: probe.video_codec,
            video_profile: probe.video_profile,
            video_pixel_format: probe.video_pixel_format,
            audio_codec: probe.audio_codec,
            file_size: Some(size),
            checksum_sha256: Some(checksum),
            audio_streams: probe.audio_streams,
            subtitle_tracks: probe.subtitle_tracks,
        },
    })
}

/// An import restores a matching missing source instead of assigning a new identity.
fn import_one(
    store: &ProjectStore,
    project_id: &str,
    input: &str,
    relink_id: Option<&str>,
) -> Result<(String, String), String> {
    let source = Path::new(input)
        .canonicalize()
        .map_err(|e| format!("Không mở được tệp: {e}"))?;
    let info = fs::metadata(&source).map_err(|e| e.to_string())?;
    if !info.is_file() || info.len() == 0 {
        return Err("Tệp đã chọn rỗng hoặc không phải tệp media.".into());
    }
    let extension = source
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = kind_for_extension(&extension)?;
    let hash = sha256_file(&source)?;
    let project = store.get(project_id)?;
    let existing = if let Some(id) = relink_id {
        let existing = asset(&project, id)?;
        if existing.kind != kind || existing.metadata.checksum_sha256.as_deref() != Some(&hash) {
            return Err("Tệp này không trùng nguồn ban đầu. Chọn đúng tệp để giữ transcript, bản dịch và giọng đọc.".into());
        }
        Some(existing.clone())
    } else {
        project
            .timeline
            .as_ref()
            .unwrap()
            .assets
            .iter()
            .find(|a| a.kind == kind && a.metadata.checksum_sha256.as_deref() == Some(&hash))
            .cloned()
    };
    if let Some(existing) = &existing {
        if owned_file(store, &project, existing, true).is_ok() {
            return Ok((existing.id.clone(), "duplicate".into()));
        }
    }
    let id = existing
        .as_ref()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let folder = store.data_directory(project_id, &format!("media/assets/{id}"))?;
    crate::disk_space::require(&folder, info.len())?;
    let staging = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
    if copy_and_hash(&source, staging.path())? != hash {
        return Err("Tệp nguồn thay đổi trong lúc nhập. Hãy thử lại.".into());
    }
    let name = format!("source-{}.{}", Uuid::new_v4(), extension);
    let relative = format!("media/assets/{id}/{name}");
    let probed = probe_asset(
        staging.path(),
        kind,
        id.clone(),
        relative,
        source
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_string),
        hash,
    )?;
    let destination = folder.join(name);
    staging
        .persist(&destination)
        .map_err(|e| format!("Không hoàn tất bản sao media: {}", e.error))?;
    let saved = store.edit_project(project_id, |project| {
        let settings = VideoSettings::from_project(project);
        let doc = project.timeline.as_mut().ok_or("Thiếu timeline.")?;
        if let Some(existing) = &existing {
            let target = doc
                .assets
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or("Nguồn đã bị xóa trong lúc nhập.")?;
            if target != existing {
                return Err("Nguồn đã thay đổi trong lúc nhập. Hãy thử lại.".into());
            }
            *target = probed.clone();
        } else {
            // A repeated video uses the same content; independent copies are a later explicit action.
            if doc.assets.iter().any(|a| {
                a.kind == probed.kind
                    && a.metadata.checksum_sha256 == probed.metadata.checksum_sha256
            }) {
                return Err("Nguồn vừa được nhập từ thao tác khác. Hãy tải lại thư viện.".into());
            }
            if probed.kind == MediaKind::Video {
                let mut settings = settings;
                settings.transcript = None;
                settings.translation.segments.clear();
                settings.blur_regions.clear();
                settings.logos.clear();
                settings.text_overlays.clear();
                doc.video_contents.push(VideoContent {
                    id: Uuid::new_v4().to_string(),
                    asset_id: id.clone(),
                    revision: 0,
                    settings,
                });
            }
            doc.assets.push(probed.clone());
        }
        Ok(())
    });
    if let Err(error) = saved {
        let _ = fs::remove_file(&destination);
        return Err(error);
    }
    // Old missing/corrupt paths may be shared by another legacy asset. Keep them for recovery.
    Ok((
        id,
        if existing.is_some() {
            "restored"
        } else {
            "imported"
        }
        .into(),
    ))
}
pub fn import_batch(
    store: &ProjectStore,
    id: &str,
    paths: &[String],
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<ImportBatch, String> {
    if paths.is_empty() || paths.len() > 100 {
        return Err("Chọn từ 1 đến 100 tệp mỗi lần nhập.".into());
    }
    let _guard = MEDIA_IO_LOCK
        .lock()
        .map_err(|_| "Không khóa được kho nguồn.")?;
    store.get(id)?;
    let mut items = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        progress(index, paths.len(), path);
        let item = match import_one(store, id, path, None) {
            Ok((asset_id, status)) => ImportItem {
                source_path: path.clone(),
                asset_id: Some(asset_id),
                status,
                error: None,
            },
            Err(error) => ImportItem {
                source_path: path.clone(),
                asset_id: None,
                status: "failed".into(),
                error: Some(error),
            },
        };
        items.push(item);
    }
    Ok(ImportBatch {
        project: store.get(id)?,
        items,
    })
}
pub fn relink(
    store: &ProjectStore,
    id: &str,
    asset_id: &str,
    path: &str,
) -> Result<Project, String> {
    let _guard = MEDIA_IO_LOCK
        .lock()
        .map_err(|_| "Không khóa được kho nguồn.")?;
    import_one(store, id, path, Some(asset_id))?;
    store.get(id)
}

pub fn append_clip(
    store: &ProjectStore,
    id: &str,
    asset_id: &str,
    at_us: Option<u64>,
) -> Result<Project, String> {
    let loaded = store.get(id)?;
    owned_file(store, &loaded, asset(&loaded, asset_id)?, true)?;
    store.edit_project(id, |project| {
        let doc = project.timeline.as_mut().ok_or("Thiếu timeline.")?;
        let source = doc
            .assets
            .iter()
            .find(|a| a.id == asset_id)
            .ok_or("Không tìm thấy nguồn.")?
            .clone();
        let track_kind = if source.kind == MediaKind::Audio {
            TrackKind::Audio
        } else {
            TrackKind::Visual
        };
        let source_out = if source.kind == MediaKind::Image {
            5_000_000
        } else {
            source
                .duration_us
                .ok_or("Chưa đọc được thời lượng nguồn.")?
        };
        let track_index = if source.kind == MediaKind::Image {
            None
        } else {
            doc.tracks
                .iter()
                .position(|t| t.kind == track_kind && !t.locked)
        };
        let start = match at_us {
            Some(at) => at,
            None => track_index
                .map(|i| {
                    doc.clips
                        .iter()
                        .filter(|c| c.track_id == doc.tracks[i].id)
                        .try_fold(0, |end, c| Ok::<u64, String>(end.max(c.end_us()?)))
                })
                .transpose()?
                .unwrap_or(0),
        };
        let end = start
            .checked_add(source_out)
            .filter(|t| *t <= timeline::MAX_TIME_US)
            .ok_or("Thời gian clip bị tràn.")?;
        let track_index = track_index.filter(|i| {
            !doc.clips.iter().any(|c| {
                c.track_id == doc.tracks[*i].id
                    && c.end_us().is_ok_and(|out| start < out && end > c.start_us)
            })
        });
        let track_id = if let Some(index) = track_index {
            doc.tracks[index].id.clone()
        } else {
            let track_id = Uuid::new_v4().to_string();
            let order = doc
                .tracks
                .iter()
                .map(|t| t.order)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("Quá nhiều track.")?;
            doc.tracks.push(Track {
                id: track_id.clone(),
                kind: track_kind,
                name: match source.kind {
                    MediaKind::Image => "Ảnh",
                    MediaKind::Video => "Video",
                    MediaKind::Audio => "Âm thanh",
                }
                .into(),
                order,
                locked: false,
                enabled: true,
                muted: false,
                solo: false,
            });
            track_id
        };
        let content = match source.kind {
            MediaKind::Video => ClipContent::Video {
                asset_id: source.id.clone(),
                video_content_id: doc
                    .video_contents
                    .iter()
                    .find(|c| c.asset_id == source.id)
                    .ok_or("Thiếu nội dung video.")?
                    .id
                    .clone(),
            },
            MediaKind::Image => ClipContent::Image {
                asset_id: source.id.clone(),
            },
            MediaKind::Audio => ClipContent::Audio {
                asset_id: source.id.clone(),
            },
        };
        let clip_id = Uuid::new_v4().to_string();
        doc.clips.push(Clip {
            id: clip_id.clone(),
            track_id,
            start_us: start,
            source_in_us: 0,
            source_out_us: source_out,
            rate: Rational::default(),
            content: content.clone(),
            transform: ClipTransform::default(),
            linked_clip_id: None,
            group_id: None,
        });
        // Keep the existing one-video player usable until T03/T08 introduce selection/playback.
        if project.source.video_path.is_none() {
            if let ClipContent::Video {
                video_content_id, ..
            } = content
            {
                let settings = doc
                    .video_contents
                    .iter()
                    .find(|c| c.id == video_content_id)
                    .unwrap()
                    .settings
                    .clone();
                doc.legacy_binding = Some(LegacyBinding {
                    asset_id: source.id.clone(),
                    video_content_id,
                    clip_id: Some(clip_id),
                });
                if let Some(width) = source.metadata.width {
                    doc.settings.width = width;
                }
                if let Some(height) = source.metadata.height {
                    doc.settings.height = height;
                }
                if let Some(fps) = source.frame_rate {
                    doc.settings.frame_rate = fps;
                }
                project.source = source.metadata;
                project.transcript = settings.transcript;
                project.translation = settings.translation;
                project.subtitle = settings.subtitle;
                project.dubbing = settings.dubbing;
                project.transcript_settings = settings.transcript_settings;
                project.blur_regions = settings.blur_regions;
                project.logos = settings.logos;
                project.text_overlays = settings.text_overlays;
            }
        }
        Ok(())
    })
}

pub fn cache(
    store: &ProjectStore,
    id: &str,
    asset_id: &str,
    kind: &str,
) -> Result<AssetCache, String> {
    let _guard = MEDIA_IO_LOCK
        .lock()
        .map_err(|_| "Không khóa được cache media.")?;
    let project = store.get(id)?;
    let source = asset(&project, asset_id)?;
    let input = owned_file(store, &project, source, true)?;
    let checksum = match &source.metadata.checksum_sha256 {
        Some(hash) if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => {
            hash.clone()
        }
        _ => sha256_file(&input)?,
    };
    let folder = store.data_directory(id, &format!("cache/assets/{asset_id}/{checksum}"))?;
    match kind {
        "thumbnail" if source.kind != MediaKind::Audio => {
            let path = folder.join("thumbnail-v1.png");
            if resolve_project_file(
                store,
                id,
                &format!("cache/assets/{asset_id}/{checksum}/thumbnail-v1.png"),
                "cache",
            )
            .is_ok()
                && valid_thumbnail(&path)
            {
                return Ok(AssetCache {
                    thumbnail_path: Some(path.to_string_lossy().into_owned()),
                    waveform: None,
                });
            }
            let temp = tempfile::Builder::new()
                .suffix(".png")
                .tempfile_in(&folder)
                .map_err(|e| e.to_string())?;
            let output = crate::runtime_paths::ffmpeg_command()?
                .args(["-y", "-v", "error", "-i"])
                .arg(input)
                .args([
                    "-map",
                    "0:v:0",
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=320:180:force_original_aspect_ratio=decrease",
                    "-update",
                    "1",
                ])
                .arg(temp.path())
                .output_contained(std::time::Duration::from_secs(30))
                .map_err(|e| format!("Không tạo được thumbnail: {e}"))?;
            if !output.status.success() || !valid_thumbnail(temp.path()) {
                return Err("Không giải mã được hình ảnh của nguồn.".into());
            }
            crate::recovery_store::write_blob(
                &path,
                &fs::read(temp.path()).map_err(|e| e.to_string())?,
            )?;
            Ok(AssetCache {
                thumbnail_path: Some(path.to_string_lossy().into_owned()),
                waveform: None,
            })
        }
        "waveform" if !source.metadata.audio_streams.is_empty() => {
            let relative = format!("cache/assets/{asset_id}/{checksum}/waveform-v1.json");
            let path = folder.join("waveform-v1.json");
            if resolve_project_file(store, id, &relative, "cache").is_ok()
                && fs::metadata(&path).is_ok_and(|m| m.len() <= 128 * 1024)
            {
                if let Ok(wave) = fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| serde_json::from_slice::<Waveform>(&b).map_err(|e| e.to_string()))
                {
                    if wave.schema_version == 1
                        && wave.checksum == checksum
                        && wave.duration_us == source.duration_us.unwrap_or(0)
                        && wave.peaks.len() == 512
                        && wave
                            .peaks
                            .iter()
                            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    {
                        return Ok(AssetCache {
                            thumbnail_path: None,
                            waveform: Some(wave),
                        });
                    }
                }
            }
            let duration = source
                .duration_us
                .ok_or("Nguồn không có thời lượng audio.")?;
            crate::disk_space::require(&folder, duration / 1_000_000 * 16_000 + 4096)?;
            let temp = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
            let output = crate::runtime_paths::ffmpeg_command()?
                .args(["-y", "-v", "error", "-i"])
                .arg(input)
                .args([
                    "-map", "0:a:0", "-vn", "-ac", "1", "-ar", "8000", "-f", "s16le",
                ])
                .arg(temp.path())
                .output_contained(std::time::Duration::from_secs(90))
                .map_err(|e| format!("Không tạo được waveform: {e}"))?;
            if !output.status.success() {
                return Err("Không giải mã được âm thanh của nguồn.".into());
            }
            let mut file = fs::File::open(temp.path()).map_err(|e| e.to_string())?;
            let samples = file.metadata().map_err(|e| e.to_string())?.len() / 2;
            if samples == 0 {
                return Err("Nguồn không có mẫu âm thanh.".into());
            }
            let mut peaks = vec![0.0_f32; 512];
            let mut buffer = [0_u8; 8192];
            let mut index = 0_u64;
            loop {
                let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                for pair in buffer[..n].chunks_exact(2) {
                    let value = (i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0).abs();
                    let bucket = (index as u128 * 512 / samples as u128).min(511) as usize;
                    peaks[bucket] = peaks[bucket].max(value);
                    index += 1;
                }
            }
            let wave = Waveform {
                schema_version: 1,
                checksum,
                duration_us: duration,
                peaks,
            };
            crate::recovery_store::write(
                &path,
                &serde_json::to_vec(&wave).map_err(|e| e.to_string())?,
            )?;
            Ok(AssetCache {
                thumbnail_path: None,
                waveform: Some(wave),
            })
        }
        _ => Err("Nguồn không hỗ trợ loại cache này.".into()),
    }
}

fn block_active_jobs(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    if crate::render_service::has_active_project_job(
        &app.state::<crate::render_service::RenderJobs>(),
        id,
    ) || crate::asr_service::has_active_project_job(
        &app.state::<crate::asr_service::AsrJobs>(),
        id,
    ) || crate::translation_service::has_active_project_job(
        &app.state::<crate::translation_service::TranslationJobs>(),
        id,
    ) || crate::tts_service::has_active_project_job(
        &app.state::<crate::tts_service::TtsJobs>(),
        id,
    ) {
        return Err(
            "Dự án đang có tác vụ xử lý. Đợi hoặc hủy tác vụ trước khi sửa kho nguồn.".into(),
        );
    }
    Ok(())
}
#[tauri::command]
pub async fn import_media_assets(
    app: tauri::AppHandle,
    project_id: String,
    paths: Vec<String>,
) -> Result<ImportBatch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = crate::commands::PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không khóa được tác vụ.")?;
        block_active_jobs(&app, &project_id)?;
        import_batch(
            &crate::commands::store(&app)?,
            &project_id,
            &paths,
            |index, total, path| {
                let _ = app.emit(
                    "media-import-progress",
                    serde_json::json!({"projectId":project_id, "index":index, "total":total,
                "name":Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("Media")}),
                );
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn inspect_media_assets(
    app: tauri::AppHandle,
    project_id: String,
    verify: bool,
) -> Result<Vec<AssetStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        inspect(&crate::commands::store(&app)?, &project_id, verify)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn relink_media_asset(
    app: tauri::AppHandle,
    project_id: String,
    asset_id: String,
    source_path: String,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = crate::commands::PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không khóa được tác vụ.")?;
        block_active_jobs(&app, &project_id)?;
        let store = crate::commands::store(&app)?;
        if store
            .get(&project_id)?
            .timeline
            .as_ref()
            .and_then(|d| d.legacy_binding.as_ref())
            .is_some_and(|b| b.asset_id == asset_id)
        {
            app.state::<crate::preview::PreviewManager>()
                .close_project(&project_id);
            app.state::<crate::audio::transport::AudioManager>()
                .close(Some(&project_id));
        }
        relink(&store, &project_id, &asset_id, &source_path)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn append_media_clip(
    app: tauri::AppHandle,
    project_id: String,
    asset_id: String,
    at_us: Option<u64>,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = crate::commands::PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không khóa được tác vụ.")?;
        block_active_jobs(&app, &project_id)?;
        append_clip(
            &crate::commands::store(&app)?,
            &project_id,
            &asset_id,
            at_us,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_media_cache(
    app: tauri::AppHandle,
    project_id: String,
    asset_id: String,
    kind: String,
) -> Result<AssetCache, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = cache(
            &crate::commands::store(&app)?,
            &project_id,
            &asset_id,
            &kind,
        )?;
        if let Some(path) = &result.thumbnail_path {
            app.asset_protocol_scope()
                .allow_file(Path::new(path))
                .map_err(|e| e.to_string())?;
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: ProjectStore,
        id: String,
        a: PathBuf,
        b: PathBuf,
        image: PathBuf,
        audio: PathBuf,
    }
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let project = store.create("Kho nguồn Việt Nam").unwrap();
        let a = dir.path().join("Video Việt A.mp4");
        let b = dir.path().join("Video B.mp4");
        for (path, color, duration) in [(&a, "red", "1"), (&b, "blue", "2")] {
            let output = crate::runtime_paths::ffmpeg_command()
                .unwrap()
                .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
                .arg(format!("color=c={color}:s=64x48:r=25"))
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=16000",
                    "-t",
                    duration,
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                ])
                .arg(path)
                .output_contained(std::time::Duration::from_secs(30))
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let image = dir.path().join("Ảnh.jpg");
        image::RgbImage::from_pixel(32, 24, image::Rgb([20, 200, 40]))
            .save(&image)
            .unwrap();
        let audio = dir.path().join("Nhạc.wav");
        let output = crate::runtime_paths::ffmpeg_command()
            .unwrap()
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:sample_rate=16000",
                "-t",
                "1",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&audio)
            .output_contained(std::time::Duration::from_secs(30))
            .unwrap();
        assert!(output.status.success());
        Fixture {
            _dir: dir,
            store,
            id: project.id,
            a,
            b,
            image,
            audio,
        }
    }
    fn paths(paths: &[&Path]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }
    #[test]
    fn imports_mixed_sources_deduplicates_and_appends_repeated_clips_without_copying() {
        let f = fixture();
        let stale = f.store.get(&f.id).unwrap();
        let bad = f._dir.path().join("bad.jpg");
        fs::write(&bad, b"not an image").unwrap();
        let result = import_batch(
            &f.store,
            &f.id,
            &paths(&[&f.a, &f.b, &f.image, &f.audio, &f.a, &bad]),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|i| i.status.as_str())
                .collect::<Vec<_>>(),
            [
                "imported",
                "imported",
                "imported",
                "imported",
                "duplicate",
                "failed"
            ]
        );
        assert_eq!(result.items[0].asset_id, result.items[4].asset_id);
        let doc = result.project.timeline.as_ref().unwrap();
        assert_eq!(doc.assets.len(), 4);
        assert_eq!(doc.video_contents.len(), 2);
        assert!(doc.clips.is_empty());
        assert!(result.project.source.video_path.is_none());
        for source in &doc.assets {
            assert!(source
                .relative_path
                .starts_with(&format!("media/assets/{}/", source.id)));
            assert!(owned_file(&f.store, &result.project, source, true).is_ok());
        }
        assert_eq!(f.store.get(&f.id).unwrap(), result.project);
        // A delayed preference save cannot erase media imported after its snapshot.
        assert!(f.store.update_project(&stale).is_err());
        assert_eq!(
            f.store.get(&f.id).unwrap().timeline,
            result.project.timeline
        );
        let a = result.items[0].asset_id.as_ref().unwrap();
        let b = result.items[1].asset_id.as_ref().unwrap();
        append_clip(&f.store, &f.id, a, None).unwrap();
        append_clip(&f.store, &f.id, b, None).unwrap();
        let project = append_clip(&f.store, &f.id, a, None).unwrap();
        let doc = project.timeline.as_ref().unwrap();
        assert_eq!(
            doc.clips.iter().map(|c| c.start_us).collect::<Vec<_>>(),
            [0, 1_000_000, 3_000_000]
        );
        assert_eq!(doc.assets.len(), 4);
        assert_eq!(doc.video_contents.len(), 2);
        assert_eq!(doc.duration_us().unwrap(), 4_000_000);
        assert_eq!(doc.clips[0].content, doc.clips[2].content);
        assert_ne!(doc.clips[0].id, doc.clips[2].id);
        assert!(crate::video_service::resolve_video(&f.store, &f.id).is_ok());
        let with_image = append_clip(
            &f.store,
            &f.id,
            result.items[2].asset_id.as_ref().unwrap(),
            Some(500_000),
        )
        .unwrap();
        assert_ne!(
            with_image.timeline.as_ref().unwrap().clips[3].track_id,
            doc.clips[0].track_id
        );
        let with_audio = append_clip(
            &f.store,
            &f.id,
            result.items[3].asset_id.as_ref().unwrap(),
            Some(0),
        )
        .unwrap();
        assert_eq!(f.store.get(&f.id).unwrap(), with_audio);
        assert!(import_batch(&f.store, &f.id, &[], |_, _, _| {}).is_err());
    }
    #[test]
    fn relinking_verifies_identity_and_preserves_clips_transcript_and_tts() {
        let f = fixture();
        let imported = import_batch(&f.store, &f.id, &paths(&[&f.a]), |_, _, _| {}).unwrap();
        let asset_id = imported.items[0].asset_id.as_ref().unwrap();
        append_clip(&f.store, &f.id, asset_id, None).unwrap();
        append_clip(&f.store, &f.id, asset_id, None).unwrap();
        let root = f.store.project_directory(&f.id).unwrap();
        fs::write(
            root.join("audio/tts/original.wav"),
            fs::read(&f.audio).unwrap(),
        )
        .unwrap();
        let mut project = f.store.get(&f.id).unwrap();
        project.transcript = Some(
            serde_json::from_value(
                json!({"status":"completed", "path":"subtitles/transcript.json", "segmentCount":1}),
            )
            .unwrap(),
        );
        project.translation.segments = serde_json::from_value(json!([{"id":"seg", "startMs":0, "endMs":900,"source":"Hello", "translated":"Xin chào","translationStatus":"edited",
            "tts":{"audioRelativePath":"audio/tts/original.wav","fingerprint":"tts", "durationMs":800,"status":"ready"}}])).unwrap();
        project = f.store.update_project(&project).unwrap();
        let bytes = serde_json::to_vec(&json!({"schemaVersion":1, "projectId":f.id, "sourceType":"asr", "language":"en",
            "videoChecksum":project.source.checksum_sha256, "segments":[{"id":"seg", "start":0.0,"end":0.9,"source":"Hello","translated":"Xin chào",
                "sourceType":"asr","translationStatus":"edited","tts":{"audioPath":"audio/tts/original.wav","duration":0.8,"status":"ready","fingerprint":"tts"}}]})).unwrap();
        fs::write(root.join("subtitles/transcript.json"), &bytes).unwrap();
        let original_transcript =
            crate::transcript_service::get_transcript(&f.store, &f.id).unwrap();
        let previous = asset(&project, asset_id).unwrap().clone();
        let old_path = root.join(&previous.relative_path);
        fs::remove_file(&old_path).unwrap();
        assert_eq!(inspect(&f.store, &f.id, true).unwrap()[0].state, "missing");
        assert!(append_clip(&f.store, &f.id, asset_id, None).is_err());
        assert!(relink(&f.store, &f.id, asset_id, &f.b.to_string_lossy()).is_err());
        assert_eq!(f.store.get(&f.id).unwrap(), project);
        let moved = f._dir.path().join("Moved Việt.mp4");
        fs::rename(&f.a, &moved).unwrap();
        let restored = relink(&f.store, &f.id, asset_id, &moved.to_string_lossy()).unwrap();
        assert_eq!(
            restored.timeline.as_ref().unwrap().clips,
            project.timeline.as_ref().unwrap().clips
        );
        assert_eq!(
            restored.timeline.as_ref().unwrap().video_contents,
            project.timeline.as_ref().unwrap().video_contents
        );
        assert_eq!(
            restored.source.video_path,
            Some(asset(&restored, asset_id).unwrap().relative_path.clone())
        );
        assert_eq!(
            fs::read(root.join("subtitles/transcript.json")).unwrap(),
            bytes
        );
        assert_eq!(
            crate::transcript_service::get_transcript(&f.store, &f.id).unwrap(),
            original_transcript
        );
        assert_eq!(
            fs::read(root.join("audio/tts/original.wav")).unwrap(),
            fs::read(&f.audio).unwrap()
        );
        assert_eq!(
            inspect(&f.store, &f.id, true).unwrap()[0].state,
            "available"
        );
        // Reimport automatically restores an existing missing source with the same ID.
        fs::remove_file(root.join(&asset(&restored, asset_id).unwrap().relative_path)).unwrap();
        let again = import_batch(&f.store, &f.id, &paths(&[&moved]), |_, _, _| {}).unwrap();
        assert_eq!(again.items[0].status, "restored");
        assert_eq!(again.items[0].asset_id.as_ref(), Some(asset_id));
        assert_eq!(again.project.timeline.as_ref().unwrap().assets.len(), 1);
    }
    #[test]
    fn caches_real_thumbnails_waveforms_and_rebuilds_corrupt_cache() {
        let f = fixture();
        let imported = import_batch(
            &f.store,
            &f.id,
            &paths(&[&f.a, &f.image, &f.audio]),
            |_, _, _| {},
        )
        .unwrap();
        for index in [0, 1] {
            let id = imported.items[index].asset_id.as_ref().unwrap();
            let result = cache(&f.store, &f.id, id, "thumbnail").unwrap();
            let path = result.thumbnail_path.unwrap();
            let dimensions = image::image_dimensions(&path).unwrap();
            assert!(dimensions.0 <= 320 && dimensions.1 <= 180);
            assert!(path.contains(id));
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cache(&f.store, &f.id, id, "thumbnail").unwrap();
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            fs::write(&path, b"broken cache").unwrap();
            cache(&f.store, &f.id, id, "thumbnail").unwrap();
            assert!(image::open(path).is_ok());
        }
        for index in [0, 2] {
            let id = imported.items[index].asset_id.as_ref().unwrap();
            let wave = cache(&f.store, &f.id, id, "waveform")
                .unwrap()
                .waveform
                .unwrap();
            assert_eq!(wave.peaks.len(), 512);
            assert!(wave.peaks.iter().any(|v| *v > 0.01));
            let folder = f
                .store
                .data_directory(&f.id, &format!("cache/assets/{id}/{}", wave.checksum))
                .unwrap();
            fs::write(folder.join("waveform-v1.json"), b"interrupted cache").unwrap();
            let rebuilt = cache(&f.store, &f.id, id, "waveform")
                .unwrap()
                .waveform
                .unwrap();
            assert_eq!(wave.peaks, rebuilt.peaks);
            assert_eq!(
                fs::read_dir(folder)
                    .unwrap()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
                    .count(),
                0
            );
        }
        let source = &imported.project.timeline.as_ref().unwrap().assets[0];
        let path = f
            .store
            .project_directory(&f.id)
            .unwrap()
            .join(&source.relative_path);
        let mut damaged = fs::read(&path).unwrap();
        damaged[0] ^= 1;
        fs::write(&path, damaged).unwrap();
        assert_eq!(inspect(&f.store, &f.id, true).unwrap()[0].state, "invalid");
        assert!(cache(&f.store, &f.id, &source.id, "thumbnail").is_err());
    }
}
