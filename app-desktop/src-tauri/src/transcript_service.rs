use crate::project_service::{
    Project, ProjectStore, TranscriptMetadata, TranscriptSourceType, TranscriptStatus,
};
use crate::video_service::{resolve_project_file, resolve_video};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use tempfile::NamedTempFile;
use uuid::Uuid;

const TRANSCRIPT_PATH: &str = "subtitles/transcript.json";
// Speech recognizers can split one repeated utterance at a timestamp boundary,
// or produce slightly overlapping chunks. Bridge only tiny gaps so an actual
// pause between two identical lines remains two separate transcript entries.
const DUPLICATE_SEGMENT_MAX_GAP_SECONDS: f64 = 0.15;
static TRANSCRIPT_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub schema_version: u32,
    pub project_id: String,
    pub source_type: TranscriptSourceType,
    pub source_track: Option<u32>,
    pub language: Option<String>,
    pub video_checksum: Option<String>,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub source: String,
    pub translated: String,
    #[serde(default)]
    pub source_revision: u64,
    #[serde(default)]
    pub translated_revision: Option<u64>,
    #[serde(default)]
    pub translation_meta: Option<TranscriptTranslationMeta>,
    pub speaker_id: Option<String>,
    pub source_type: TranscriptSourceType,
    pub translation_status: TranscriptTranslationStatus,
    pub tts: Option<TranscriptTts>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptTranslationStatus {
    Pending,
    Translated,
    Edited,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTranslationMeta {
    pub provider: String,
    pub model: Option<String>,
    pub target_language: String,
    pub translated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTts {
    pub audio_path: Option<String>,
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TranscriptTtsStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_revision: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptTtsStatus {
    Ready,
    Outdated,
    DurationOverflow,
    Failed,
}

impl TranscriptSegment {
    fn new(start: f64, end: f64, source: String, source_type: TranscriptSourceType) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            start,
            end,
            source,
            translated: String::new(),
            source_revision: 0,
            translated_revision: None,
            translation_meta: None,
            speaker_id: None,
            source_type,
            translation_status: TranscriptTranslationStatus::Pending,
            tts: None,
        }
    }
}

pub fn get_transcript(
    store: &ProjectStore,
    project_id: &str,
) -> Result<Option<Transcript>, String> {
    let project = store.get(project_id)?;
    let Some(metadata) = project.transcript else {
        return Ok(None);
    };
    if metadata.status != TranscriptStatus::Completed {
        return Ok(None);
    }
    let path = resolve_project_file(store, project_id, &metadata.path, "subtitles")?;
    let bytes = fs::read(path).map_err(|error| format!("Không thể đọc transcript: {error}"))?;
    let transcript: Transcript = serde_json::from_slice(&bytes)
        .map_err(|error| format!("File transcript.json bị lỗi hoặc không hợp lệ: {error}"))?;
    if transcript.project_id != project_id {
        return Err("Transcript không thuộc dự án hiện tại.".to_string());
    }
    if transcript.video_checksum != project.source.checksum_sha256 {
        return Err("Transcript không khớp với video nguồn hiện tại.".to_string());
    }
    if transcript.schema_version != 1 {
        return Err(format!(
            "Phiên bản transcript {} chưa được hỗ trợ.",
            transcript.schema_version
        ));
    }
    validate_transcript(&transcript, project.source.duration)?;
    Ok(Some(transcript))
}

pub fn save_transcript(
    store: &ProjectStore,
    project_id: &str,
    mut transcript: Transcript,
) -> Result<Project, String> {
    with_transcript_lock(project_id, || {
        if let Some(latest) = get_transcript(store, project_id)? {
            preserve_newer_translation_state(&mut transcript, &latest);
        }
        save_transcript_locked(store, project_id, transcript)
    })
}

fn preserve_newer_translation_state(incoming: &mut Transcript, persisted: &Transcript) {
    for segment in &mut incoming.segments {
        let Some(current) = persisted
            .segments
            .iter()
            .find(|saved| saved.id == segment.id)
        else {
            continue;
        };
        let current_has_translation = !current.translated.is_empty()
            || current.translation_status == TranscriptTranslationStatus::Edited;
        if current_has_translation
            && segment.translation_status != TranscriptTranslationStatus::Edited
            && (segment.translated != current.translated
                || segment.translation_status != current.translation_status
                || segment.translated_revision != current.translated_revision
                || segment.translation_meta != current.translation_meta)
        {
            segment.translated = current.translated.clone();
            segment.translation_status = current.translation_status.clone();
            segment.translated_revision = current.translated_revision;
            segment.translation_meta = current.translation_meta.clone();
        }
    }
}

pub fn save_transcript_locked(
    store: &ProjectStore,
    project_id: &str,
    mut transcript: Transcript,
) -> Result<Project, String> {
    let project = store.get(project_id)?;
    if transcript.project_id != project_id {
        return Err("Transcript không thuộc dự án hiện tại.".to_string());
    }
    if project.source.video_path.is_none() {
        return Err("Không thể lưu transcript khi dự án chưa có video.".to_string());
    }
    if transcript.video_checksum != project.source.checksum_sha256 {
        return Err(
            "Transcript thuộc video cũ. Hãy tạo transcript mới cho video hiện tại.".to_string(),
        );
    }
    transcript.schema_version = 1;
    transcript
        .segments
        .sort_by(|left, right| left.start.total_cmp(&right.start));
    if let Some(previous) = get_transcript(store, project_id)? {
        for current in &mut transcript.segments {
            if let Some(saved) = previous
                .segments
                .iter()
                .find(|saved| saved.id == current.id)
            {
                if current.translated != saved.translated {
                    if let Some(tts) = current.tts.as_mut() {
                        tts.status = Some(TranscriptTtsStatus::Outdated);
                        tts.warning = Some("Bản dịch đã thay đổi sau khi tạo audio.".to_string());
                    }
                }
            }
        }
    }
    validate_transcript(&transcript, project.source.duration)?;
    let project_dir = store.project_directory(project_id)?;
    let transcript_path = project_dir.join(TRANSCRIPT_PATH);
    let previous_bytes = fs::read(&transcript_path).ok();
    let bytes = serde_json::to_vec_pretty(&transcript)
        .map_err(|error| format!("Không thể chuyển transcript thành JSON: {error}"))?;
    atomic_write(&transcript_path, &bytes)?;

    let mut updated = project;
    updated.transcript = Some(TranscriptMetadata {
        status: TranscriptStatus::Completed,
        path: TRANSCRIPT_PATH.to_string(),
        source_type: Some(transcript.source_type.clone()),
        source_track: transcript.source_track,
        language: transcript.language.clone(),
        segment_count: transcript.segments.len(),
    });
    match store.update_project(&updated) {
        Ok(project) => Ok(project),
        Err(error) => {
            match previous_bytes {
                Some(bytes) => {
                    let _ = atomic_write(&transcript_path, &bytes);
                }
                None => {
                    let _ = fs::remove_file(&transcript_path);
                }
            }
            Err(error)
        }
    }
}

pub fn with_transcript_lock<T>(
    project_id: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let locks = TRANSCRIPT_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let project_lock = {
        let mut locks = locks
            .lock()
            .map_err(|_| "Không thể truy cập khóa transcript.".to_string())?;
        locks
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = project_lock
        .lock()
        .map_err(|_| "Không thể khóa dữ liệu transcript của dự án.".to_string())?;
    operation()
}

pub fn extract_subtitle(
    store: &ProjectStore,
    project_id: &str,
    track_index: u32,
) -> Result<(Transcript, Project), String> {
    let project = store.get(project_id)?;
    let track = project
        .source
        .subtitle_tracks
        .iter()
        .find(|track| track.stream_index == track_index)
        .ok_or_else(|| "Không tìm thấy subtitle track đã chọn.".to_string())?;
    if !track.text_supported || track.image_based {
        return Err("Subtitle track này là ảnh (bitmap) và không thể trích xuất thành văn bản. Có thể tạo transcript bằng Speech Recognition.".to_string());
    }
    let video_path = resolve_video(store, project_id)?;
    let map = format!("0:{track_index}");
    let output = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&video_path)
        .args(["-map", map.as_str(), "-f", "srt", "pipe:1"])
        .output()
        .map_err(|error| {
            format!("Không thể khởi chạy FFmpeg. Hãy cài FFmpeg và thêm ffmpeg vào PATH: {error}")
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "FFmpeg không thể trích xuất subtitle track này.".to_string()
        } else {
            format!("Không thể trích xuất subtitle: {detail}")
        });
    }
    let original = String::from_utf8(output.stdout)
        .map_err(|_| "Subtitle không phải văn bản UTF-8 hợp lệ.".to_string())?;
    let mut segments = parse_srt_vtt(&original, TranscriptSourceType::Subtitle)?;
    if segments.is_empty() {
        return Err("Subtitle track không có cue văn bản hợp lệ.".to_string());
    }
    segments = merge_adjacent_duplicate_segments(segments);
    let transcript = Transcript {
        schema_version: 1,
        project_id: project_id.to_string(),
        source_type: TranscriptSourceType::Subtitle,
        source_track: Some(track_index),
        language: track.language.clone(),
        video_checksum: project.source.checksum_sha256.clone(),
        segments,
    };
    validate_transcript(&transcript, project.source.duration)?;
    let project_dir = store.project_directory(project_id)?;
    atomic_write(
        &project_dir.join("subtitles").join("original.srt"),
        original.as_bytes(),
    )?;
    let project = save_transcript(store, project_id, transcript.clone())?;
    Ok((transcript, project))
}

pub fn parse_srt_vtt(
    contents: &str,
    source_type: TranscriptSourceType,
) -> Result<Vec<TranscriptSegment>, String> {
    let normalized = contents
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let mut segments = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !lines[index].contains("-->") {
            index += 1;
            continue;
        }
        let timing = lines[index]
            .split_once("-->")
            .ok_or_else(|| "Dòng timestamp subtitle không hợp lệ.".to_string())?;
        let start = parse_time(timing.0.trim())
            .ok_or_else(|| format!("Timestamp bắt đầu không hợp lệ: {}", timing.0.trim()))?;
        let end_text = timing.1.split_whitespace().next().unwrap_or_default();
        let end = parse_time(end_text)
            .ok_or_else(|| format!("Timestamp kết thúc không hợp lệ: {end_text}"))?;
        if end <= start {
            return Err("Thời điểm kết thúc subtitle phải lớn hơn thời điểm bắt đầu.".to_string());
        }
        index += 1;
        let content_start = index;
        while index < lines.len()
            && !lines[index].trim().is_empty()
            && !lines[index].contains("-->")
        {
            index += 1;
        }
        let source = lines[content_start..index].join("\n").trim().to_string();
        if !source.is_empty() {
            segments.push(TranscriptSegment::new(
                start,
                end,
                source,
                source_type.clone(),
            ));
        }
    }
    Ok(segments)
}

/// Merge consecutive, overlapping or immediately adjacent duplicate captions
/// produced by ASR/OCR/subtitle extraction. The first segment keeps its ID and
/// text; only its end time is extended to cover the merged interval.
pub fn merge_adjacent_duplicate_segments(
    mut segments: Vec<TranscriptSegment>,
) -> Vec<TranscriptSegment> {
    segments.sort_by(|left, right| {
        left.start
            .total_cmp(&right.start)
            .then_with(|| left.end.total_cmp(&right.end))
    });

    let mut merged: Vec<TranscriptSegment> = Vec::with_capacity(segments.len());
    for segment in segments {
        let normalized_text = normalized_spoken_text(&segment.source);
        let can_merge = merged.last().is_some_and(|previous| {
            !normalized_text.is_empty()
                && previous.source_type == segment.source_type
                && previous.speaker_id == segment.speaker_id
                && normalized_spoken_text(&previous.source) == normalized_text
                && segment.start <= previous.end + DUPLICATE_SEGMENT_MAX_GAP_SECONDS
        });

        if can_merge {
            if let Some(previous) = merged.last_mut() {
                previous.end = previous.end.max(segment.end);
            }
        } else {
            merged.push(segment);
        }
    }
    merged
}

fn normalized_spoken_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .trim_end_matches(|character: char| {
            character.is_ascii_punctuation()
                || matches!(character, '。' | '！' | '？' | '，' | '、' | '；' | '：' | '…')
        })
        .to_string()
}

pub fn validate_transcript(transcript: &Transcript, duration: Option<f64>) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for segment in &transcript.segments {
        if segment.id.trim().is_empty() || !ids.insert(segment.id.as_str()) {
            return Err("Transcript có segment ID trống hoặc bị trùng.".to_string());
        }
        if !segment.start.is_finite() || !segment.end.is_finite() || segment.start < 0.0 {
            return Err("Timestamp transcript phải là số hữu hạn và không âm.".to_string());
        }
        if segment.end <= segment.start {
            return Err("Timestamp kết thúc phải lớn hơn timestamp bắt đầu.".to_string());
        }
        if let Some(duration) = duration {
            if segment.end > duration + 0.001 {
                return Err("Timestamp transcript vượt quá thời lượng video.".to_string());
            }
        }
        if let Some(tts) = &segment.tts {
            if tts
                .duration
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            {
                return Err("Thời lượng TTS không hợp lệ.".to_string());
            }
            if tts.audio_path.as_deref().is_some_and(|path| {
                Path::new(path).is_absolute() || path.split(['/', '\\']).any(|part| part == "..")
            }) {
                return Err("Đường dẫn audio TTS phải nằm tương đối trong dự án.".to_string());
            }
        }
    }
    Ok(())
}

fn parse_time(value: &str) -> Option<f64> {
    let value = value.trim().split_whitespace().next()?;
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 2 && parts.len() != 3 {
        return None;
    }
    let (hours, minutes, seconds) = if parts.len() == 3 {
        (
            parts[0].parse::<f64>().ok()?,
            parts[1].parse::<f64>().ok()?,
            parts[2],
        )
    } else {
        (0.0, parts[0].parse::<f64>().ok()?, parts[1])
    };
    let seconds = seconds.replace(',', ".").parse::<f64>().ok()?;
    let total = hours * 3600.0 + minutes * 60.0 + seconds;
    (total.is_finite() && total >= 0.0).then_some(total)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Đường dẫn tệp project không hợp lệ.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Không thể tạo thư mục lưu dữ liệu: {error}"))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Không thể tạo tệp tạm thời: {error}"))?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("Không thể ghi transcript: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Không thể lưu transcript: {}", error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::ProjectStore;
    use crate::video_service::import_video;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn parses_srt_and_webvtt_with_unicode_and_multiline_text() {
        let srt = "1\n00:00:01,200 --> 00:00:03,500\nXin chào mọi người.\nHôm nay nhé!\n\n2\n00:00:04.000 --> 00:00:05.250\nCảm ơn bạn.\n";
        let segments = parse_srt_vtt(srt, TranscriptSourceType::Subtitle).unwrap();
        assert_eq!(segments.len(), 2);
        assert!((segments[0].start - 1.2).abs() < 0.0001);
        assert_eq!(segments[0].source, "Xin chào mọi người.\nHôm nay nhé!");
        assert_eq!(segments[1].source, "Cảm ơn bạn.");

        let vtt = "WEBVTT\n\nintro\n00:01.000 --> 00:02.500 align:start\nHello\nworld\n";
        let cues = parse_srt_vtt(vtt, TranscriptSourceType::Subtitle).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].source, "Hello\nworld");
        assert_eq!(cues[0].end, 2.5);
    }

    #[test]
    fn rejects_invalid_segment_time_and_duplicate_ids() {
        let mut transcript = Transcript {
            schema_version: 1,
            project_id: "project".to_string(),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: None,
            video_checksum: None,
            segments: vec![TranscriptSegment::new(
                0.0,
                1.0,
                "a".to_string(),
                TranscriptSourceType::Asr,
            )],
        };
        assert!(validate_transcript(&transcript, Some(2.0)).is_ok());
        transcript.segments[0].end = 0.0;
        assert!(validate_transcript(&transcript, Some(2.0)).is_err());
        transcript.segments[0].end = 1.0;
        transcript.segments.push(transcript.segments[0].clone());
        assert!(validate_transcript(&transcript, Some(2.0)).is_err());
    }

    #[test]
    fn extracts_embedded_unicode_srt_and_restores_the_saved_transcript() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Subtitle import").unwrap();
        let base = temp.path().join("base.mp4");
        let base_result = Command::new("ffmpeg")
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
                "sine=frequency=550:sample_rate=44100:duration=1",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&base)
            .output();
        if !base_result.is_ok_and(|result| result.status.success()) {
            return;
        }
        let subtitle_file = temp.path().join("captions.srt");
        fs::write(
            &subtitle_file,
            "1\n00:00:00,200 --> 00:00:00,900\nXin chào Việt Nam — café nhé!\nDòng thứ hai.\n",
        )
        .unwrap();
        let muxed = temp.path().join("with-subtitles.mkv");
        let mux_result = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&base)
            .arg("-i")
            .arg(&subtitle_file)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "0:a:0",
                "-map",
                "1:0",
                "-c:v",
                "copy",
                "-c:a",
                "copy",
                "-c:s",
                "srt",
                "-metadata:s:s:0",
                "language=vie",
            ])
            .arg(&muxed)
            .output();
        if !mux_result.is_ok_and(|result| result.status.success()) {
            return;
        }
        let imported = import_video(&store, &project.id, muxed.to_str().unwrap(), false).unwrap();
        let track = imported
            .project
            .source
            .subtitle_tracks
            .iter()
            .find(|track| track.text_supported)
            .unwrap();
        let (transcript, updated_project) =
            extract_subtitle(&store, &project.id, track.stream_index).unwrap();
        assert_eq!(transcript.segments.len(), 1);
        assert_eq!(
            transcript.segments[0].source,
            "Xin chào Việt Nam — café nhé!\nDòng thứ hai."
        );
        assert!(transcript.segments[0].end > transcript.segments[0].start);
        assert_eq!(updated_project.transcript.unwrap().segment_count, 1);
        let reloaded = get_transcript(&store, &project.id).unwrap().unwrap();
        assert_eq!(reloaded, transcript);
    }

    #[test]
    fn saves_edits_additions_and_deletions_atomically_for_reload() {
        let temp = tempdir().unwrap();
        let projects_root = temp.path().join("projects");
        let store = ProjectStore::new(projects_root.clone());
        let mut project = store.create("Transcript persistence").unwrap();
        project.source.video_path = Some("media/source/video.mp4".to_string());
        project.source.checksum_sha256 = Some("a".repeat(64));
        project.source.duration = Some(8.0);
        let project = store.update_project(&project).unwrap();

        let first = TranscriptSegment::new(
            0.5,
            1.8,
            "Hello".to_string(),
            TranscriptSourceType::Subtitle,
        );
        let first_id = first.id.clone();
        let transcript = Transcript {
            schema_version: 1,
            project_id: project.id.clone(),
            source_type: TranscriptSourceType::Subtitle,
            source_track: Some(2),
            language: Some("en".to_string()),
            video_checksum: project.source.checksum_sha256.clone(),
            segments: vec![first],
        };
        save_transcript(&store, &project.id, transcript).unwrap();

        let mut edited = get_transcript(&store, &project.id).unwrap().unwrap();
        edited.segments[0].source = "Xin chào".to_string();
        edited.segments[0].start = 0.6;
        let second = TranscriptSegment::new(
            2.0,
            3.0,
            "Thêm segment".to_string(),
            TranscriptSourceType::Subtitle,
        );
        edited.segments.push(second.clone());
        edited.segments.retain(|segment| segment.id != first_id);
        save_transcript(&store, &project.id, edited.clone()).unwrap();

        let reloaded = get_transcript(&ProjectStore::new(projects_root), &project.id)
            .unwrap()
            .unwrap();
        assert_eq!(reloaded, edited);
        assert_eq!(reloaded.segments.len(), 1);
        assert_eq!(reloaded.segments[0].source, "Thêm segment");
    }

    #[test]
    fn editing_translation_marks_audio_outdated_but_preserves_its_relative_file() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let mut project = store.create("TTS invalidation").unwrap();
        project.source.video_path = Some("media/source/video.mp4".to_string());
        project.source.checksum_sha256 = Some("b".repeat(64));
        project.source.duration = Some(5.0);
        let project = store.update_project(&project).unwrap();
        let mut segment =
            TranscriptSegment::new(0.0, 2.0, "Hello".to_string(), TranscriptSourceType::Asr);
        segment.translated = "Xin chào".to_string();
        segment.translation_status = TranscriptTranslationStatus::Translated;
        segment.tts = Some(TranscriptTts {
            audio_path: Some("audio/tts/voice.wav".to_string()),
            duration: Some(1.2),
            fingerprint: Some("fingerprint".to_string()),
            status: Some(TranscriptTtsStatus::Ready),
            voice: Some("Ngọc Lan".to_string()),
            speed: Some(1.0),
            text_hash: Some("hash".to_string()),
            generated_at: Some("2026-09-23T00:00:00Z".to_string()),
            warning: None,
            provider: Some("vieneu".to_string()),
            model_id: Some("v3-turbo".to_string()),
            model_revision: Some("revision".to_string()),
        });
        let transcript = Transcript {
            schema_version: 1,
            project_id: project.id.clone(),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: Some("en".to_string()),
            video_checksum: project.source.checksum_sha256.clone(),
            segments: vec![segment],
        };
        save_transcript(&store, &project.id, transcript).unwrap();
        let mut edited = get_transcript(&store, &project.id).unwrap().unwrap();
        edited.segments[0].translated = "Chào mọi người".to_string();
        edited.segments[0].translation_status = TranscriptTranslationStatus::Edited;
        save_transcript(&store, &project.id, edited).unwrap();
        let reloaded = get_transcript(&store, &project.id).unwrap().unwrap();
        let tts = reloaded.segments[0].tts.as_ref().unwrap();
        assert_eq!(tts.status, Some(TranscriptTtsStatus::Outdated));
        assert_eq!(tts.audio_path.as_deref(), Some("audio/tts/voice.wav"));
    }

    #[test]
    fn stale_frontend_save_preserves_a_translation_saved_during_the_request() {
        let mut incoming = Transcript {
            schema_version: 1,
            project_id: "project".to_string(),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: Some("en".to_string()),
            video_checksum: None,
            segments: vec![TranscriptSegment::new(
                0.0,
                1.0,
                "Hello".to_string(),
                TranscriptSourceType::Asr,
            )],
        };
        let mut persisted = incoming.clone();
        persisted.segments[0].translated = "Xin chào".to_string();
        persisted.segments[0].translation_status = TranscriptTranslationStatus::Translated;
        persisted.segments[0].translated_revision = Some(0);
        persisted.segments[0].translation_meta = Some(TranscriptTranslationMeta {
            provider: "gemini".to_string(),
            model: Some("model".to_string()),
            target_language: "vi".to_string(),
            translated_at: "2026-09-23T00:00:00Z".to_string(),
        });

        incoming.segments[0].source = "Hello everyone".to_string();
        incoming.segments[0].source_revision = 1;
        preserve_newer_translation_state(&mut incoming, &persisted);

        assert_eq!(incoming.segments[0].translated, "Xin chào");
        assert_eq!(
            incoming.segments[0].translation_status,
            TranscriptTranslationStatus::Translated
        );
        assert_eq!(incoming.segments[0].translated_revision, Some(0));
        assert_eq!(incoming.segments[0].source_revision, 1);
    }

    #[test]
    fn reports_corrupt_transcript_json_without_crashing() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let mut project = store.create("Broken transcript").unwrap();
        project.source.checksum_sha256 = Some("b".repeat(64));
        project.transcript = Some(TranscriptMetadata {
            status: TranscriptStatus::Completed,
            path: TRANSCRIPT_PATH.to_string(),
            source_type: Some(TranscriptSourceType::Asr),
            source_track: None,
            language: None,
            segment_count: 1,
        });
        store.update_project(&project).unwrap();
        let project_dir = store.project_directory(&project.id).unwrap();
        fs::write(project_dir.join(TRANSCRIPT_PATH), "{broken json").unwrap();
        assert!(get_transcript(&store, &project.id)
            .unwrap_err()
            .contains("transcript.json"));
    }
}
