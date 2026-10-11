use crate::worker_manager::WorkerCommandExt;
use crate::project_service::{
    AudioStreamMetadata, Project, ProjectSource, ProjectStore, SubtitleTrackMetadata,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tempfile::NamedTempFile;
use uuid::Uuid;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoImportResult {
    pub project: Project,
    pub duplicate: bool,
    pub replaced: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    size: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ProbeStream {
    index: u32,
    codec_name: Option<String>,
    profile: Option<String>,
    pix_fmt: Option<String>,
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    channels: Option<u16>,
    sample_rate: Option<String>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    disposition: std::collections::HashMap<String, i32>,
}

#[derive(Clone, Debug)]
pub struct ProbeMetadata {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub video_codec: Option<String>,
    pub video_profile: Option<String>,
    pub video_pixel_format: Option<String>,
    pub audio_codec: Option<String>,
    pub file_size: u64,
    pub audio_streams: Vec<AudioStreamMetadata>,
    pub subtitle_tracks: Vec<SubtitleTrackMetadata>,
    pub has_video: bool,
}

pub fn import_video(
    store: &ProjectStore,
    project_id: &str,
    source_path: &str,
    replace: bool,
) -> Result<VideoImportResult, String> {
    import_video_mode(store,project_id,source_path,replace,false)
}

pub fn restore_source_video(store: &ProjectStore, project_id: &str, source_path: &str) -> Result<VideoImportResult,String> {
    import_video_mode(store,project_id,source_path,true,true)
}

fn import_video_mode(store: &ProjectStore,project_id: &str,source_path: &str,replace: bool,restore: bool) -> Result<VideoImportResult,String> {
    let mut project = store.get(project_id)?;
    let source = PathBuf::from(source_path);
    let source = source
        .canonicalize()
        .map_err(|error| format!("Không thể mở video đã chọn: {error}"))?;
    let source_meta =
        fs::metadata(&source).map_err(|error| format!("Không thể đọc video đã chọn: {error}"))?;
    if !source_meta.is_file() {
        return Err("Tệp đã chọn không phải là video.".to_string());
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !["mp4", "mov", "mkv", "webm"].contains(&extension.as_str()) {
        return Err("Định dạng chưa được hỗ trợ. Hãy chọn MP4, MOV, MKV hoặc WEBM.".to_string());
    }
    log::info!("Video import started; project={project_id}, extension={extension}, bytes={}", source_meta.len());

    log::info!("Video import: hashing source; project={project_id}");
    let checksum = sha256_file(&source)?;
    if restore && project.source.checksum_sha256.as_deref()!=Some(&checksum) {
        return Err("Tệp này không trùng video nguồn ban đầu. Hãy chọn đúng video; transcript và bản dịch được giữ nguyên.".into());
    }
    if let (Some(existing_hash), Some(existing_relative)) = (
        project.source.checksum_sha256.as_deref(),
        project.source.video_path.as_deref(),
    ) {
        if existing_hash == checksum
            && resolve_project_file(store, project_id, existing_relative, "media").is_ok()
        {
            return Ok(VideoImportResult {
                project,
                duplicate: true,
                replaced: false,
            });
        }
    }
    let had_video = project.source.video_path.is_some();
    if had_video && !replace {
        return Err("Dự án đã có video. Hãy xác nhận thay thế video hiện tại.".to_string());
    }

    let source_dir = store.data_directory(project_id, "media/source")?;
    crate::disk_space::require(&source_dir,source_meta.len())?;
    let staging = NamedTempFile::new_in(&source_dir)
        .map_err(|error| format!("Không thể chuẩn bị tệp video tạm thời: {error}"))?;
    let staging_path = staging.path().to_path_buf();
    log::info!("Video import: copying source; project={project_id}");
    let staged_checksum = match copy_and_hash(&source, &staging_path) {
        Ok(checksum) => checksum,
        Err(error) => {
            let _ = staging.close();
            return Err(error);
        }
    };
    if staged_checksum != checksum {
        let _ = staging.close();
        return Err(
            "Video nguồn đã thay đổi trong lúc nhập. Hãy chọn lại tệp và thử lại.".to_string(),
        );
    }
    let staged_size = fs::metadata(&staging_path)
        .map_err(|error| format!("Không thể xác minh bản sao video: {error}"))?
        .len();
    log::info!("Video copied to project staging; project={project_id}, bytes={staged_size}");

    log::info!("Video import: probing staged video; project={project_id}");
    let probe = match probe_video(&staging_path, staged_size) {
        Ok(probe) if probe.has_video => probe,
        Ok(_) => {
            let _ = staging.close();
            return Err("Tệp này không chứa video stream hợp lệ.".to_string());
        }
        Err(error) => {
            let _ = staging.close();
            return Err(error);
        }
    };
    log::info!("Video probe completed; project={project_id}, codec={}, profile={}, pixel_format={}, audio={}",
        probe.video_codec.as_deref().unwrap_or("unknown"),
        probe.video_profile.as_deref().unwrap_or("unknown"),
        probe.video_pixel_format.as_deref().unwrap_or("unknown"),
        probe.audio_codec.as_deref().unwrap_or("none"));

    let destination_name = format!("{}.{}", Uuid::new_v4(), extension);
    let relative_path = format!("media/source/{destination_name}");
    let destination = source_dir.join(&destination_name);
    staging
        .persist(&destination)
        .map_err(|error| format!("Không thể hoàn tất sao chép video: {}", error.error))?;

    let previous_relative = project.source.video_path.clone();
    let old_transcript = project.transcript.as_ref().map(|entry| entry.path.clone());
    project.schema_version = 2;
    project.source = ProjectSource {
        video_path: Some(relative_path),
        original_name: source
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        duration: probe.duration,
        width: probe.width,
        height: probe.height,
        fps: probe.fps,
        video_codec: probe.video_codec,
        video_profile: probe.video_profile,
        video_pixel_format: probe.video_pixel_format,
        audio_codec: probe.audio_codec,
        file_size: Some(probe.file_size),
        checksum_sha256: Some(checksum),
        audio_streams: probe.audio_streams,
        subtitle_tracks: probe.subtitle_tracks,
    };
    if !restore { project.transcript = None; project.translation.segments.clear(); }
    let updated = match store.update_project(&project) {
        Ok(updated) => updated,
        Err(error) => {
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
    };
    log::info!("Video import completed; project={project_id}");

    if had_video && !restore {
        if let Some(previous) = previous_relative {
            if let Ok(path) = resolve_project_file(store, project_id, &previous, "media") {
                let _ = fs::remove_file(path);
            }
        }
        if let Some(previous) = old_transcript {
            if let Ok(path) = resolve_project_file(store, project_id, &previous, "subtitles") {
                let _ = fs::remove_file(path);
            }
        }
        for (relative,prefix) in [("subtitles/original.srt","subtitles"),("audio/extracted/source.wav","audio/extracted")] {
            if let Ok(path)=resolve_project_file(store,project_id,relative,prefix) {let _=fs::remove_file(path);}
        }
    }

    Ok(VideoImportResult {
        project: updated,
        duplicate: false,
        replaced: had_video && !restore,
    })
}

pub fn resolve_video(store: &ProjectStore, project_id: &str) -> Result<PathBuf, String> {
    let project = store.get(project_id)?;
    let relative = project
        .source
        .video_path
        .as_deref()
        .ok_or_else(|| "Dự án chưa có video nguồn.".to_string())?;
    resolve_project_file(store, project_id, relative, "media")
}

pub fn create_preview_proxy(
    store: &ProjectStore,
    project_id: &str,
    force_rebuild: bool,
) -> Result<PathBuf, String> {
    let project = store.get(project_id)?;
    let source = resolve_video(store, project_id)?;
    let checksum = project
        .source
        .checksum_sha256
        .as_deref()
        .ok_or_else(|| "Thiếu checksum video để tạo preview.".to_string())?;
    let preview_dir = store.data_directory(project_id, "cache/preview")?;
    let checksum_prefix: String = checksum.chars().take(24).collect();
    if checksum_prefix.len() < 24 || !checksum_prefix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("Checksum video không hợp lệ để tạo preview.".to_string());
    }
    // Version the proxy cache so older H.264 profiles from previous app builds
    // are not reused after tightening WebView compatibility requirements.
    let filename = format!("{checksum_prefix}-webview-v2.mp4");
    let destination = preview_dir.join(&filename);
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Tệp cache preview không hợp lệ; hãy xóa cache preview của project rồi thử lại.".into());
        }
        if !force_rebuild
            && validate_preview_proxy(&destination, !project.source.audio_streams.is_empty()).is_ok()
        {
            return Ok(destination);
        }
        // A proxy left by an older or interrupted run must never be treated as
        // ready just because a file with the expected checksum name exists.
        fs::remove_file(&destination)
            .map_err(|error| format!("Không thể dọn preview H.264/AAC bị lỗi: {error}"))?;
    }
    let temporary = preview_dir.join(format!(".preview-{}.mp4", Uuid::new_v4()));
    let output = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-vf",
            "scale='min(1280,iw)':'min(1280,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2",
            "-c:v",
            "libx264",
            "-profile:v",
            "baseline",
            "-preset",
            "ultrafast",
            "-crf",
            "27",
            "-pix_fmt",
            "yuv420p",
            "-tag:v",
            "avc1",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+faststart",
        ])
        .arg(&temporary)
        .output_contained(std::time::Duration::from_secs(7200))
        .map_err(|error| format!("Không thể khởi chạy thành phần FFmpeg của ứng dụng: {error}"))?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "FFmpeg không thể tạo preview H.264 tương thích với WebView.".to_string()
        } else {
            format!("Không thể tạo preview H.264: {detail}")
        });
    }
    if let Err(error) = validate_preview_proxy(&temporary, !project.source.audio_streams.is_empty()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("FFmpeg kết thúc nhưng preview không hợp lệ: {error}"));
    }
    fs::rename(&temporary, &destination).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("Không thể lưu video preview: {error}")
    })?;
    Ok(destination)
}

fn validate_preview_proxy(path: &Path, require_audio: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Không đọc được tệp preview: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err("Tệp preview rỗng hoặc không phải file thường.".into());
    }
    let probe = probe_video(path, metadata.len())?;
    if !probe.has_video || probe.video_codec.as_deref() != Some("h264") {
        return Err("Không tìm thấy video stream H.264 hợp lệ trong preview.".into());
    }
    if !probe.video_profile.as_deref().is_some_and(|profile| {
        profile.to_ascii_lowercase().contains("baseline")
    }) || probe.video_pixel_format.as_deref() != Some("yuv420p")
    {
        return Err("Preview không dùng profile H.264 Baseline / pixel format yuv420p tương thích WebView.".into());
    }
    if require_audio && probe.audio_codec.as_deref() != Some("aac") {
        return Err("Video nguồn có audio nhưng preview không có audio stream AAC hợp lệ.".into());
    }
    if !probe.duration.is_some_and(|duration| duration.is_finite() && duration > 0.0) {
        return Err("Preview không có thời lượng hợp lệ.".into());
    }
    Ok(())
}

pub fn resolve_project_file(
    store: &ProjectStore,
    project_id: &str,
    relative: &str,
    required_prefix: &str,
) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !relative
            .replace('\\', "/")
            .starts_with(&format!("{required_prefix}/"))
    {
        return Err("Đường dẫn dữ liệu dự án không hợp lệ.".to_string());
    }
    let root = store.project_directory(project_id)?;
    let path = root.join(relative_path);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Không tìm thấy tệp dự án: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Tệp dự án không hợp lệ hoặc là liên kết ngoài.".to_string());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục dự án: {error}"))?;
    let canonical_file = path
        .canonicalize()
        .map_err(|error| format!("Không thể mở tệp dự án: {error}"))?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err("Từ chối truy cập tệp bên ngoài thư mục dự án.".to_string());
    }
    Ok(canonical_file)
}

pub fn probe_video(path: &Path, file_size: u64) -> Result<ProbeMetadata, String> {
    let output = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output_contained(std::time::Duration::from_secs(30))
        .map_err(|error| format!("Không thể khởi chạy FFprobe của ứng dụng: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "FFprobe không nhận diện được tệp video hoặc tệp bị hỏng.".to_string()
        } else {
            format!("FFprobe không đọc được video: {detail}")
        });
    }
    let parsed: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Kết quả FFprobe không hợp lệ: {error}"))?;
    let video = parsed.streams.iter().find(|stream| {
        stream.codec_type.as_deref() == Some("video")
            && stream.disposition.get("attached_pic").copied().unwrap_or(0) == 0
    });
    let mut audio_streams = Vec::new();
    let mut subtitle_tracks = Vec::new();
    for stream in &parsed.streams {
        let codec = stream
            .codec_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let language = stream.tags.get("language").cloned();
        let title = stream.tags.get("title").cloned();
        match stream.codec_type.as_deref() {
            Some("audio") => audio_streams.push(AudioStreamMetadata {
                stream_index: stream.index,
                codec,
                language,
                title,
                channels: stream.channels,
                sample_rate: stream
                    .sample_rate
                    .as_deref()
                    .and_then(|rate| rate.parse().ok()),
            }),
            Some("subtitle") => {
                let text_supported = is_text_subtitle_codec(&codec);
                subtitle_tracks.push(SubtitleTrackMetadata {
                    stream_index: stream.index,
                    codec,
                    language,
                    title,
                    text_supported,
                    image_based: !text_supported,
                });
            }
            _ => {}
        }
    }
    let duration = parsed
        .format
        .duration
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    let fps = video
        .and_then(|stream| {
            stream
                .avg_frame_rate
                .as_deref()
                .or(stream.r_frame_rate.as_deref())
        })
        .and_then(parse_rational)
        .filter(|fps| fps.is_finite() && *fps > 0.0);
    Ok(ProbeMetadata {
        duration,
        width: video.and_then(|stream| stream.width),
        height: video.and_then(|stream| stream.height),
        fps,
        video_codec: video.and_then(|stream| stream.codec_name.clone()),
        video_profile: video.and_then(|stream| stream.profile.clone()),
        video_pixel_format: video.and_then(|stream| stream.pix_fmt.clone()),
        audio_codec: audio_streams.first().map(|stream| stream.codec.clone()),
        file_size: parsed
            .format
            .size
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(file_size),
        audio_streams,
        subtitle_tracks,
        has_video: video.is_some(),
    })
}

pub fn is_text_subtitle_codec(codec: &str) -> bool {
    matches!(
        codec.to_ascii_lowercase().as_str(),
        "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "mov_text" | "text"
    )
}

fn parse_rational(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    if denominator == 0.0 {
        None
    } else {
        Some(numerator / denominator)
    }
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Không thể đọc video: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Không thể kiểm tra video: {error}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn copy_and_hash(source: &Path, destination: &Path) -> Result<String, String> {
    let mut input =
        File::open(source).map_err(|error| format!("Không thể mở video nguồn: {error}"))?;
    let mut output = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)
        .map_err(|error| format!("Không thể tạo bản sao video: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("Không thể đọc video nguồn: {error}"))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("Không thể sao chép video vào dự án: {error}"))?;
        hash.update(&buffer[..count]);
    }
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|error| format!("Không thể lưu bản sao video: {error}"))?;
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::ProjectStore;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn parses_frame_rate_and_subtitle_codec_capabilities() {
        assert!((parse_rational("30000/1001").unwrap() - 29.97002997).abs() < 0.0001);
        assert_eq!(parse_rational("25/0"), None);
        assert!(is_text_subtitle_codec("ass"));
        assert!(is_text_subtitle_codec("mov_text"));
        assert!(!is_text_subtitle_codec("hdmv_pgs_subtitle"));
        assert!(!is_text_subtitle_codec("dvd_subtitle"));
    }

    #[test]
    fn rejects_project_paths_outside_the_media_scope() {
        assert!(Path::new("../outside.mp4")
            .components()
            .any(|part| !matches!(part, Component::Normal(_))));
        assert!(!is_text_subtitle_codec("dvd_subtitle"));
    }

    fn make_fixture_video(path: &Path, frequency: &str) -> bool {
        Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=160x120:r=8:d=1",
                "-f",
                "lavfi",
                "-i",
                frequency,
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(path)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    #[test]
    fn imports_real_video_detects_streams_and_avoids_duplicate_copy() {
        let temp = tempdir().unwrap();
        let projects = temp.path().join("projects");
        let store = ProjectStore::new(projects.clone());
        let project = store.create("Import video").unwrap();
        let source = temp.path().join("fixture.mp4");
        if !make_fixture_video(&source, "sine=frequency=770:sample_rate=44100:duration=1") {
            // The integration fixture is skipped on development machines without an H.264 encoder.
            return;
        }

        let imported = import_video(&store, &project.id, source.to_str().unwrap(), false).unwrap();
        assert!(!imported.duplicate);
        assert!(imported.project.source.duration.unwrap() > 0.0);
        assert_eq!(imported.project.source.width, Some(160));
        assert_eq!(imported.project.source.audio_streams.len(), 1);
        let relative = imported.project.source.video_path.as_ref().unwrap();
        let copied = resolve_project_file(&store, &project.id, relative, "media/source").unwrap();
        assert!(copied.is_file());
        let source_files = fs::read_dir(projects.join(&project.id).join("media/source"))
            .unwrap()
            .count();
        assert_eq!(source_files, 1);

        let duplicate = import_video(&store, &project.id, source.to_str().unwrap(), false).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(
            fs::read_dir(projects.join(&project.id).join("media/source"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn failed_video_import_keeps_existing_video_and_cleans_staging_file() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("projects");
        let store = ProjectStore::new(root.clone());
        let project = store.create("Keep old source").unwrap();
        let valid = temp.path().join("good.mp4");
        if !make_fixture_video(&valid, "sine=frequency=350:sample_rate=44100:duration=1") {
            return;
        }
        let imported = import_video(&store, &project.id, valid.to_str().unwrap(), false).unwrap();
        let original_relative = imported.project.source.video_path.clone().unwrap();
        let invalid = temp.path().join("invalid.mp4");
        fs::write(&invalid, b"not a video stream").unwrap();
        assert!(import_video(&store, &project.id, invalid.to_str().unwrap(), true).is_err());
        let after = store.get(&project.id).unwrap();
        assert_eq!(
            after.source.video_path.as_deref(),
            Some(original_relative.as_str())
        );
        assert!(
            resolve_project_file(&store, &project.id, &original_relative, "media/source").is_ok()
        );
        assert_eq!(
            fs::read_dir(root.join(&project.id).join("media/source"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn relinking_same_source_preserves_transcript_and_wrong_source_is_rejected() {
        let temp=tempdir().unwrap(); let store=ProjectStore::new(temp.path().join("projects"));
        let project=store.create("Relink tiếng Việt").unwrap(); let source=temp.path().join("nguồn.mp4");
        assert!(make_fixture_video(&source,"sine=frequency=350:sample_rate=44100:duration=1"),"FFmpeg fixture is required");
        let mut project=import_video(&store,&project.id,source.to_str().unwrap(),false).unwrap().project;
        project.translation.segments=vec![serde_json::from_value(serde_json::json!({"id":"legacy","startMs":0,"endMs":800,"source":"Giữ nguyên","translated":"Keep","translationStatus":"empty"})).unwrap()];
        project.transcript=Some(crate::project_service::TranscriptMetadata {status:crate::project_service::TranscriptStatus::Completed,path:"subtitles/transcript.json".into(),source_type:Some(crate::project_service::TranscriptSourceType::Subtitle),source_track:None,language:Some("vi".into()),segment_count:1});
        let project=store.update_project(&project).unwrap();
        let transcript=store.data_directory(&project.id,"subtitles").unwrap().join("transcript.json"); fs::write(&transcript,b"private fixture transcript").unwrap();
        let owned=resolve_video(&store,&project.id).unwrap();fs::remove_file(&owned).unwrap();
        let invalid=temp.path().join("other.mp4");fs::write(&invalid,b"different content").unwrap();
        assert!(restore_source_video(&store,&project.id,invalid.to_str().unwrap()).is_err());
        assert_eq!(store.get(&project.id).unwrap(),project);
        let restored=restore_source_video(&store,&project.id,source.to_str().unwrap()).unwrap();
        assert!(!restored.replaced);assert_eq!(restored.project.translation.segments,project.translation.segments);assert_eq!(restored.project.transcript,project.transcript);
        assert_eq!(fs::read(transcript).unwrap(),b"private fixture transcript");assert!(resolve_video(&store,&project.id).is_ok());
    }

    #[test]
    fn rejects_external_project_media_path() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Path guard").unwrap();
        let external = temp.path().join("external.mp4");
        fs::write(&external, b"outside").unwrap();
        assert!(resolve_project_file(
            &store,
            &project.id,
            "media/source/../../external.mp4",
            "media/source"
        )
        .is_err());
        assert!(resolve_project_file(
            &store,
            &project.id,
            external.to_str().unwrap(),
            "media/source"
        )
        .is_err());
        assert_eq!(fs::read(&external).unwrap(), b"outside");
    }
}
