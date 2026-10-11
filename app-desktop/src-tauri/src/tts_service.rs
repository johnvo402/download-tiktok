use crate::worker_manager::WorkerCommandExt;
use crate::project_service::ProjectStore;
use crate::settings_service::{self, TtsSettings, TtsVoiceProfile};
use crate::transcript_service::{
    self, Transcript, TranscriptSegment, TranscriptTts, TranscriptTtsStatus,
};
use crate::video_service::resolve_project_file;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const MODEL_ID: &str = "v3-turbo";
static MODEL_CACHE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
pub struct TtsJobs {
    inner: Arc<Mutex<HashMap<String, TtsJobEntry>>>,
    deleting: Arc<Mutex<HashSet<String>>>,
    start_lock: Arc<Mutex<()>>,
}

#[derive(Clone)]
struct TtsJobEntry {
   target: crate::video_content::JobTarget,
    project_id: String,
    canceled: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsJobEvent {
    #[serde(flatten)]
    target: crate::video_content::JobTarget,
    pub project_id: String,
    pub job_id: String,
    pub status: String,
    pub message: String,
    pub progress: f64,
    pub completed_segments: usize,
    pub total_segments: usize,
    pub segment_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsEnvironment {
    // Legacy wire name: true means the VieNeu SDK package is installed.
    pub worker_installed: bool,
    pub model_cached: bool,
    pub cuda_available: bool,
    pub model: String,
    pub model_revision: Option<String>,
    pub voices: Vec<String>,
    pub estimated_download_bytes: u64,
    pub python_package: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsJobRequest {
    #[serde(default)]
    pub video_content_id: Option<String>,
    pub scope: String,
    #[serde(default)]
    pub segment_ids: Vec<String>,
    pub voice: Option<String>,
    pub speed: Option<f64>,
    pub fit_to_duration: Option<bool>,
    pub device: Option<String>,
}

#[derive(Clone)]
struct WorkItem {
    segment: TranscriptSegment,
    slot_end: f64,
    fingerprint: String,
    text_hash: String,
    stage_path: PathBuf,
    destination: PathBuf,
    reference_audio: Option<PathBuf>,
    model_revision: String,
    raw_path: PathBuf,
    processing: crate::audio::ProcessingSettings,
    source_checksum: Option<String>,
    dubbing_snapshot: crate::project_service::DubbingSettings,
}

#[derive(Default)]
struct JobCounts {
    completed: usize,
    failed: usize,
    processed: usize,
}

pub fn has_active_project_job(jobs: &TtsJobs, project_id: &str) -> bool {
    jobs.inner
        .lock()
        .map(|jobs| jobs.values().any(|job| job.project_id == project_id))
        .unwrap_or(true)
}

pub fn get_environment(app: AppHandle) -> Result<TtsEnvironment, String> {
    let cache = crate::runtime_paths::data_dir(&app)
        .map_err(|error| format!("Không thể xác định cache model: {error}"))?
        .join("models")
        .join("huggingface");
    fs::create_dir_all(&cache)
        .map_err(|error| format!("Không thể tạo thư mục cache model: {error}"))?;
    let worker = worker_root(&app)?;
    let value = worker_request(
        &worker,
        json!({ "operation":"tts_inspect", "cacheDir": cache }),
        None,
        |_| {},
    )?;
    serde_json::from_value(value)
        .map_err(|error| format!("Kết quả kiểm tra VieNeu không hợp lệ: {error}"))
}

pub fn install_model(app: AppHandle) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK
        .lock()
        .map_err(|_| "Không thể khóa thao tác model VieNeu-TTS.".to_string())?;
    if get_environment(app.clone())?.model_cached {
        return Ok(());
    }
    let cache = crate::runtime_paths::data_dir(&app)
        .map_err(|error| format!("Không thể xác định cache model: {error}"))?
        .join("models")
        .join("huggingface");
    fs::create_dir_all(&cache).map_err(|error| format!("Không thể tạo thư mục model: {error}"))?;
    let worker = worker_root(&app)?;
    let app_for_progress = app.clone();
    worker_request(
        &worker,
        json!({ "operation":"tts_install", "cacheDir": cache }),
        None,
        move |event| {
            if event["type"] == "progress" {
                let _ = app_for_progress.emit("tts-model-progress", event);
            }
        },
    )?;
    Ok(())
}

pub fn remove_model(app: AppHandle, jobs: &TtsJobs) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK
        .lock()
        .map_err(|_| "Không thể khóa thao tác model VieNeu-TTS.".to_string())?;
    let active = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể kiểm tra TTS jobs.".to_string())?;
    if !active.is_empty() || crate::studio_service::has_active_voice() {
        return Err("Không thể gỡ VieNeu-TTS khi đang chạy một tác vụ giọng đọc.".to_string());
    }
    drop(active);
    let data_root = crate::runtime_paths::data_dir(&app)
        .map_err(|error| format!("Không thể xác định cache model: {error}"))?;
    remove_model_cache(&data_root.join("models").join("huggingface"))
}

fn remove_model_cache(cache_root: &Path) -> Result<(), String> {
    let root = cache_root;
    let metadata = fs::symlink_metadata(&root)
        .map_err(|error| format!("Không thể xác minh cache VieNeu-TTS: {error}"))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err("Cache VieNeu-TTS không hợp lệ hoặc là symlink.".to_string());
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let hub = root.join("hub");
    if let Ok(metadata) = fs::symlink_metadata(&hub) {
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err("Cache Hugging Face VieNeu không hợp lệ; không xóa dữ liệu.".to_string());
        }
        let hub = hub.canonicalize().map_err(|error| error.to_string())?;
        if !hub.starts_with(&root) {
            return Err(
                "Cache model VieNeu nằm ngoài thư mục ứng dụng; không xóa dữ liệu.".to_string(),
            );
        }
        for entry in [
            "models--pnnbao-ump--VieNeu-TTS-v3-Turbo",
            "models--OpenMOSS-Team--MOSS-Audio-Tokenizer-Nano-ONNX",
        ] {
            let path = hub.join(entry);
            match fs::symlink_metadata(&path) {
                Ok(meta) if is_reparse_or_symlink(&meta) || !meta.is_dir() => {
                    return Err("Tệp cache VieNeu-TTS không hợp lệ; không xóa dữ liệu.".to_string());
                }
                Ok(_) => {
                    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
                    if canonical.parent() != Some(hub.as_path()) {
                        return Err(
                            "Đường dẫn cache VieNeu không nằm trong thư mục model được quản lý."
                                .to_string(),
                        );
                    }
                    fs::remove_dir_all(canonical)
                        .map_err(|error| format!("Không thể xóa cache VieNeu-TTS: {error}"))?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Không thể kiểm tra cache VieNeu-TTS: {error}")),
            }
        }
    } else if hub.exists() {
        return Err("Không thể kiểm tra cache Hugging Face VieNeu.".to_string());
    }
    let manifest = root.join("vieneu-model-manifest.json");
    match fs::symlink_metadata(&manifest) {
        Ok(meta) if is_reparse_or_symlink(&meta) || !meta.is_file() => {
            return Err("Manifest VieNeu-TTS không hợp lệ; không xóa dữ liệu.".to_string());
        }
        Ok(_) => fs::remove_file(manifest)
            .map_err(|error| format!("Không thể xóa manifest VieNeu-TTS: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Không thể kiểm tra manifest VieNeu-TTS: {error}")),
    }
    Ok(())
}

fn is_reparse_or_symlink(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn preview_voice(app: AppHandle, jobs: &TtsJobs, voice: String) -> Result<String, String> {
    let _start_guard = jobs
        .start_lock
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
    let job_id = format!("preview-{}", Uuid::new_v4());
    {
        let mut active = jobs
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
        if !active.is_empty() {
            return Err(
                "Một tác vụ TTS khác đang chạy. Chờ tác vụ đó hoàn tất rồi thử lại.".to_string(),
            );
        }
        active.insert(
            job_id.clone(),
            TtsJobEntry {
                target: Default::default(),
                project_id: "__preview__".to_string(),
                canceled: Arc::new(AtomicBool::new(false)),
            },
        );
    }
    let result = preview_voice_inner(app, voice);
    if let Ok(mut active) = jobs.inner.lock() {
        active.remove(&job_id);
    }
    result
}

fn preview_voice_inner(app: AppHandle, voice: String) -> Result<String, String> {
    let env = get_environment(app.clone())?;
    if !env.worker_installed {
        return Err("VieNeu-TTS SDK chưa được cài trong Python runtime.".to_string());
    }
    if !env.model_cached {
        return Err("Hãy tải model VieNeu-TTS trước khi nghe thử.".to_string());
    }
    let settings = settings_service::read_settings(&settings_service::settings_path(&app)?)?;
    let profile = settings
        .tts
        .voice_profiles
        .iter()
        .find(|profile| profile.name == voice);
    if !env.voices.iter().any(|name| name == &voice) && profile.is_none() {
        return Err("Giọng VieNeu không hợp lệ.".to_string());
    }
    let root = crate::runtime_paths::data_dir(&app)
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu: {error}"))?;
    let cache = root.join("models").join("huggingface");
    let preview_dir = root.join("models").join("tts-preview");
    fs::create_dir_all(&preview_dir)
        .map_err(|error| format!("Không thể tạo thư mục nghe thử: {error}"))?;
    let id = Uuid::new_v4().simple().to_string();
    let output = preview_dir.join(format!("voice-{id}.wav"));
    let worker = worker_root(&app)?;
    let reference_audio = profile
        .map(|profile| resolve_voice_profile(&app, profile))
        .transpose()?;
    let mut segment = json!({ "id":"preview", "text":"Xin chào, đây là giọng đọc thử từ VieNeu TTS.", "voice":voice, "outputPath":output });
    if let Some(reference) = reference_audio {
        segment["referenceAudio"] = json!(reference);
    }
    let request = json!({ "operation":"tts_generate", "cacheDir":cache, "device":settings.tts.device, "segments":[segment]});
    let mut segment_failure = None;
    let worker_result = worker_request(&worker, request, None, |event| {
        if event["type"].as_str() == Some("segment_failed") {
            segment_failure = event["message"].as_str().map(str::to_owned);
        }
    });
    if let Err(error) = worker_result {
        let _ = fs::remove_file(&output);
        return Err(error);
    }
    if !has_wave_header(&output) {
        let _ = fs::remove_file(&output);
        return Err(segment_failure.unwrap_or_else(|| {
            "VieNeu không tạo được file giọng đọc thử hợp lệ. Kiểm tra model và log Python worker rồi thử lại.".to_string()
        }));
    }
    Ok(output.to_string_lossy().into_owned())
}

pub fn add_voice_profile(
    app: &AppHandle,
    name: &str,
    source_path: &str,
    consent: bool,
) -> Result<(), String> {
    if !consent {
        return Err(
            "Cần xác nhận bạn có quyền và sự đồng ý khi dùng audio tham chiếu.".to_string(),
        );
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 48 || name.chars().any(char::is_control) {
        return Err("Tên voice profile phải từ 1 đến 48 ký tự.".to_string());
    }
    let source = PathBuf::from(source_path);
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("Không đọc được audio tham chiếu: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Audio tham chiếu phải là file thường, không phải liên kết.".to_string());
    }
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus"].contains(&extension.as_str()) {
        return Err(
            "Định dạng audio tham chiếu hỗ trợ WAV, MP3, M4A, AAC, FLAC, OGG hoặc OPUS."
                .to_string(),
        );
    }
    let source_duration = probe_audio(&source)?;
    if !(3.0..=8.0).contains(&source_duration) {
        return Err("Audio tham chiếu cần dài từ 3 đến 8 giây.".to_string());
    }
    let mut settings = settings_service::read_settings(&settings_service::settings_path(app)?)?;
    if settings
        .tts
        .voice_profiles
        .iter()
        .any(|profile| profile.name.eq_ignore_ascii_case(name))
    {
        return Err("Tên voice profile đã tồn tại.".to_string());
    }
    let env = get_environment(app.clone())?;
    if env
        .voices
        .iter()
        .any(|voice| voice.eq_ignore_ascii_case(name))
    {
        return Err("Tên voice profile trùng với một giọng preset VieNeu.".to_string());
    }
    let dir = voice_profiles_directory(app)?;
    let id = Uuid::new_v4().to_string();
    let destination = dir.join(format!("{id}.wav"));
    let temporary = dir.join(format!(".{id}.tmp.wav"));
    let output = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "48000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&temporary)
        .output_contained(std::time::Duration::from_secs(300))
        .map_err(|error| format!("Không thể chạy FFmpeg để chuẩn hóa audio tham chiếu: {error}"))?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Không thể chuẩn hóa audio tham chiếu: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let normalized_duration = probe_audio(&temporary)?;
    if !(3.0..=8.0).contains(&normalized_duration) {
        let _ = fs::remove_file(&temporary);
        return Err("Audio tham chiếu sau chuẩn hóa phải dài từ 3 đến 8 giây.".to_string());
    }
    fs::rename(&temporary, &destination)
        .map_err(|error| format!("Không thể lưu voice profile: {error}"))?;
    let profile = TtsVoiceProfile {
        id: id.clone(),
        name: name.to_string(),
        audio_path: format!("voice-profiles/{id}.wav"),
    };
    settings.tts.voice_profiles.push(profile);
    if let Err(error) =
        settings_service::write_settings(&settings_service::settings_path(app)?, &settings)
    {
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
}

pub fn delete_voice_profile(app: &AppHandle, profile_id: &str) -> Result<(), String> {
    let mut settings = settings_service::read_settings(&settings_service::settings_path(app)?)?;
    let position = settings
        .tts
        .voice_profiles
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| "Không tìm thấy voice profile.".to_string())?;
    let profile = settings.tts.voice_profiles.remove(position);
    let path = resolve_voice_profile(app, &profile)?;
    if settings.tts.default_voice == profile.name {
        settings.tts.default_voice = "Ngọc Huyền".to_string();
    }
    let deleting_path = path.with_file_name(format!(".{}.deleting", profile.id));
    fs::rename(&path, &deleting_path)
        .map_err(|error| format!("Không thể chuẩn bị xóa voice profile: {error}"))?;
    if let Err(error) =
        settings_service::write_settings(&settings_service::settings_path(app)?, &settings)
    {
        let _ = fs::rename(&deleting_path, &path);
        return Err(error);
    }
    fs::remove_file(deleting_path).map_err(|error| {
        format!("Voice profile đã bị xóa khỏi cài đặt nhưng file tạm chưa dọn được: {error}")
    })
}

fn voice_profiles_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let root = crate::runtime_paths::data_dir(app)
        .map_err(|error| format!("Không thể xác định thư mục voice profile: {error}"))?;
    let models = root.join("models");
    let dir = models.join("voice-profiles");
    for path in [&root, &models, &dir] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("Thư mục voice profile không an toàn.".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)
                .map_err(|create| format!("Không thể tạo thư mục voice profile: {create}"))?,
            Err(error) => return Err(format!("Không thể kiểm tra thư mục voice profile: {error}")),
        }
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục app data: {error}"))?;
    let canonical_dir = dir
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục voice profile: {error}"))?;
    if !canonical_dir.starts_with(&canonical_root) {
        return Err("Voice profile vượt khỏi thư mục dữ liệu ứng dụng.".to_string());
    }
    Ok(canonical_dir)
}

fn resolve_voice_profile(app: &AppHandle, profile: &TtsVoiceProfile) -> Result<PathBuf, String> {
    let relative = Path::new(&profile.audio_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        || !profile.audio_path.starts_with("voice-profiles/")
    {
        return Err("Đường dẫn voice profile không an toàn.".to_string());
    }
    let dir = voice_profiles_directory(app)?;
    let file_name = relative
        .file_name()
        .ok_or_else(|| "Tên file voice profile không hợp lệ.".to_string())?;
    if file_name.to_string_lossy() != format!("{}.wav", profile.id) {
        return Err("File voice profile không khớp ID.".to_string());
    }
    let path = dir.join(file_name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Không tìm thấy audio voice profile: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Audio voice profile không hợp lệ.".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh audio voice profile: {error}"))?;
    if !canonical.starts_with(&dir) {
        return Err("Audio voice profile nằm ngoài thư mục được quản lý.".to_string());
    }
    Ok(canonical)
}

fn probe_audio(path: &Path) -> Result<f64, String> {
    let output = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "json",
        ])
        .arg(path)
        .output_contained(std::time::Duration::from_secs(300))
        .map_err(|error| format!("Không thể chạy FFprobe để xác nhận audio: {error}"))?;
    if !output.status.success() {
        return Err("FFprobe không nhận diện được audio tham chiếu.".to_string());
    }
    let result: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("FFprobe trả dữ liệu không hợp lệ: {error}"))?;
    if !result["streams"]
        .as_array()
        .is_some_and(|streams| streams.iter().any(|stream| stream["codec_type"] == "audio"))
    {
        return Err("File không có audio stream.".to_string());
    }
    result["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| "Audio tham chiếu không có thời lượng hợp lệ.".to_string())
}

pub fn start_generation(
    app: AppHandle,
    jobs: &TtsJobs,
    project_id: String,
    mut request: TtsJobRequest,
) -> Result<String, String> {
    let _start_guard = jobs
        .start_lock
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
    let deleting = jobs
        .deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
    if deleting.contains(&project_id) {
        return Err("Dự án đang được xóa.".to_string());
    }
    drop(deleting);
    let store = project_store(&app)?.for_video(&project_id, request.video_content_id.as_deref())?;
    let project = store.get(&project_id)?;
    request.video_content_id = store.content_id().map(str::to_owned);
    if project.dubbing.provider != "vieneu" || project.dubbing.model_id != MODEL_ID {
        return Err(
            "Cấu hình project đang chọn TTS provider hoặc model không được hỗ trợ.".to_string(),
        );
    }
    if project.source.video_path.is_none() {
        return Err("Dự án chưa có video.".to_string());
    }
    let transcript = transcript_service::get_transcript(&store, &project_id)?
        .ok_or_else(|| "Dự án chưa có transcript.".to_string())?;
    let settings = settings_service::read_settings(&settings_service::settings_path(&app)?)?.tts;
    let environment = get_environment(app.clone())?;
    if !environment.worker_installed {
        return Err("VieNeu-TTS SDK chưa được cài trong Python runtime.".to_string());
    }
    if !environment.model_cached {
        return Err(
            "Model VieNeu chưa được cài. Mở Settings → Voice / TTS và nhấn Tải model trước."
                .to_string(),
        );
    }
    let model_revision = environment
        .model_revision
        .clone()
        .unwrap_or_else(|| MODEL_ID.to_string());
    let voice = effective_voice(request.voice.as_deref(), &project, &settings)?;
    if !environment.voices.iter().any(|item| item == &voice)
        && !settings
            .voice_profiles
            .iter()
            .any(|profile| profile.name == voice)
    {
        return Err("Giọng VieNeu đã chọn không nằm trong danh sách preset hiện có.".to_string());
    }
    let speed = request
        .speed
        .or(project.dubbing.speed)
        .unwrap_or(settings.speed);
    if !speed.is_finite() || !(0.85..=2.0).contains(&speed) {
        return Err("Tốc độ phải nằm trong khoảng 0.85–2.00×.".to_string());
    }
    let device = request.device.as_deref().unwrap_or(&settings.device);
    if !matches!(device, "auto" | "cpu" | "cuda") {
        return Err("Thiết bị TTS không hợp lệ.".to_string());
    }
    if device == "cuda" && !environment.cuda_available {
        return Err(
            "CUDA chưa khả dụng hoặc chưa cài PyTorch CUDA. Chọn Auto hoặc CPU.".to_string(),
        );
    }
    let use_gpu = device != "cpu" && environment.cuda_available;

    let ids: HashSet<&str> = request.segment_ids.iter().map(String::as_str).collect();
    if ids.len() != request.segment_ids.len() {
        return Err("Danh sách segment có ID trùng.".to_string());
    }
    if matches!(request.scope.as_str(), "selected" | "segment") && ids.is_empty() {
        return Err("Chọn ít nhất một segment.".to_string());
    }
    if request.scope == "segment" && ids.len() != 1 {
        return Err("Tạo giọng đọc từng dòng cần đúng một segment.".to_string());
    }
    for id in &ids {
        if !transcript.segments.iter().any(|segment| segment.id == *id) {
            return Err("Một segment được chọn không còn tồn tại.".to_string());
        }
    }
    if !matches!(
        request.scope.as_str(),
        "all" | "missing" | "selected" | "segment"
    ) {
        return Err("Phạm vi tạo giọng đọc không hợp lệ.".to_string());
    }

    let job_id = Uuid::new_v4().to_string();
    let canceled = Arc::new(AtomicBool::new(false));
    let entry = TtsJobEntry {
       target: store.job_target(&project),
        project_id: project_id.clone(),
        canceled: canceled.clone(),
    };
    {
        let mut all_jobs = jobs
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
        if all_jobs.values().any(|entry| entry.project_id == project_id && entry.target.video_content_id == request.video_content_id) {
            return Err(
                "Một tác vụ TTS khác đang chạy. Chờ tác vụ đó hoàn tất rồi thử lại.".to_string(),
            );
        }
        all_jobs.insert(job_id.clone(), entry);
    }
    let resources = vec![if use_gpu { "gpu" } else { "cpu" }, "python-worker", "disk"];
    crate::job_service::register_job(
        &app,
        job_id.clone(),
        project_id.clone(),
        project.name.clone(),
        "tts",
        &resources,
        serde_json::to_value(&request).ok(),
    ).map_err(|error| {if let Ok(mut entries)=jobs.inner.lock() {entries.remove(&job_id);} error})?;
    emit(
        &app,
        &job_id,
        &project_id,
        "queued",
        "Đang xếp hàng tạo giọng đọc…",
        0.0,
        0,
        0,
        None,
    );
    let jobs_clone = jobs.clone();
    let project_id_for_thread = project_id.clone();
    let job_id_for_thread = job_id.clone();
    let canceled_for_status = canceled.clone();
    thread::spawn(move || {
        let result = (|| {
            let _resource_permit = if use_gpu {
                Some(crate::resource_scheduler::wait_for_ai_gpu(&canceled)?)
            } else {
                Some(crate::resource_scheduler::wait_for_cpu(&canceled)?)
            };
            run_generation(
                app.clone(),
                &jobs_clone,
                &job_id_for_thread,
                &project_id_for_thread,
                request,
                settings,
                model_revision,
                canceled,
            )
        })();
        if let Err(error) = result {
            let status = if canceled_for_status.load(Ordering::SeqCst) {
                "cancelled"
            } else {
                "failed"
            };
            emit(
                &app,
                &job_id_for_thread,
                &project_id_for_thread,
                status,
                &error,
                0.0,
                0,
                0,
                None,
            );
        }
        if let Ok(mut active) = jobs_clone.inner.lock() {
            active.remove(&job_id_for_thread);
        }
    });
    Ok(job_id)
}

pub fn cancel(jobs: &TtsJobs, job_id: &str) -> Result<bool, String> {
    let all_jobs = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
    if let Some(job) = all_jobs.get(job_id) {
        job.canceled.store(true, Ordering::SeqCst);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn audio_path(
    store: &ProjectStore,
    project_id: &str,
    segment_id: &str,
) -> Result<String, String> {
    let transcript = transcript_service::get_transcript(store, project_id)?
        .ok_or_else(|| "Transcript chưa có audio lồng tiếng.".to_string())?;
    let segment = transcript
        .segments
        .iter()
        .find(|segment| segment.id == segment_id)
        .ok_or_else(|| "Không tìm thấy segment.".to_string())?;
    let tts = segment
        .tts
        .as_ref()
        .ok_or_else(|| "Segment chưa có audio lồng tiếng.".to_string())?;
    if !matches!(
        tts.status,
        Some(TranscriptTtsStatus::Ready | TranscriptTtsStatus::DurationOverflow)
    ) {
        return Err("Audio lồng tiếng đã cũ hoặc chưa sẵn sàng. Hãy tạo lại segment.".to_string());
    }
    let relative = tts
        .audio_path
        .as_deref()
        .ok_or_else(|| "Không tìm thấy file audio lồng tiếng.".to_string())?;
    resolve_tts_file(store, project_id, relative)
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn get_transcript_with_audio_validation(
    store: &ProjectStore,
    project_id: &str,
) -> Result<Option<Transcript>, String> {
    transcript_service::with_transcript_lock(&store.transcript_lock_key(project_id), || {
        let Some(mut transcript) = transcript_service::get_transcript(store, project_id)? else {
            return Ok(None);
        };
        let mut changed = false;
        for segment in &mut transcript.segments {
            let Some(tts) = segment.tts.as_mut() else {
                continue;
            };
            if !matches!(
                tts.status,
                Some(TranscriptTtsStatus::Ready | TranscriptTtsStatus::DurationOverflow)
            ) {
                continue;
            }
            let valid = tts
                .audio_path
                .as_deref()
                .and_then(|path| resolve_tts_file(store, project_id, path).ok())
                .is_some_and(|path| crate::audio::signal::inspect(&path).is_ok());
            if !valid {
                tts.status = Some(TranscriptTtsStatus::Failed);
                tts.warning = Some(
                    "File audio bị thiếu hoặc không phải WAV hợp lệ. Hãy tạo lại segment."
                        .to_string(),
                );
                changed = true;
            }
        }
        if changed {
            transcript_service::save_transcript_locked(store, project_id, transcript.clone())?;
        }
        Ok(Some(transcript))
    })
}

fn has_wave_header(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if metadata.len() < 44 {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 12];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    matches!(&header[..4], b"RIFF" | b"RF64") && &header[8..12] == b"WAVE"
}

pub fn set_project_deleting(jobs: &TtsJobs, project_id: &str) -> Result<(), String> {
    jobs.deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái project jobs.".to_string())?
        .insert(project_id.to_string());
    Ok(())
}

pub fn clear_project_deleting(jobs: &TtsJobs, project_id: &str) {
    if let Ok(mut deleting) = jobs.deleting.lock() {
        deleting.remove(project_id);
    }
}

pub fn cancel_project_jobs_and_wait(jobs: &TtsJobs, project_id: &str) -> Result<(), String> {
    set_project_deleting(jobs, project_id)?;
    if let Ok(active) = jobs.inner.lock() {
        for job in active.values().filter(|job| job.project_id == project_id) {
            job.canceled.store(true, Ordering::SeqCst);
        }
    }
    let until = Instant::now() + Duration::from_secs(12);
    loop {
        let active = jobs
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?
            .values()
            .any(|job| job.project_id == project_id);
        if !active {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("Đang dừng tác vụ TTS. Hãy thử xóa dự án lại sau giây lát.".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn run_generation(
    app: AppHandle,
    _jobs: &TtsJobs,
    job_id: &str,
    project_id: &str,
    request: TtsJobRequest,
    settings: TtsSettings,
    model_revision: String,
    canceled: Arc<AtomicBool>,
) -> Result<(), String> {
    let store = project_store(&app)?.for_video(project_id, request.video_content_id.as_deref())?;
    let project = store.get(project_id)?;
    if store.job_target(&project) != job_target(&app, job_id) { return Err("Nội dung video đã thay đổi trong hàng đợi TTS. Hãy tạo lại tác vụ.".into()); }
    let snapshot = transcript_service::get_transcript(&store, project_id)?
        .ok_or_else(|| "Không tìm thấy transcript.".to_string())?;
    let voice = effective_voice(request.voice.as_deref(), &project, &settings)?;
    let project_fit_mode = project.dubbing.fit_mode.as_deref();
    let fit = request.fit_to_duration.unwrap_or_else(|| {
        project_fit_mode
            .map(|mode| mode == "fit")
            .unwrap_or(settings.fit_to_duration)
    });
    let speed = if project_fit_mode == Some("natural") || fit {
        1.0
    } else {
        request
            .speed
            .or(project.dubbing.speed)
            .unwrap_or(settings.speed)
    };
    let device = request.device.unwrap_or(settings.device.clone());
    let selected: HashSet<String> = request.segment_ids.into_iter().collect();
    let voice_profile = settings
        .voice_profiles
        .iter()
        .find(|profile| profile.name == voice);
    let reference_audio = voice_profile
        .map(|profile| resolve_voice_profile(&app, profile))
        .transpose()?;
    let reference_hash = reference_audio
        .as_ref()
        .map(|path| fs::read(path).map(|bytes| sha256(&bytes)))
        .transpose()
        .map_err(|error| format!("Không thể đọc voice reference: {error}"))?;
    let tts_dir = safe_tts_directory(&store, project_id)?;
    let mut items = Vec::new();
    for segment in &snapshot.segments {
        if segment.translated.trim().is_empty() {
            continue;
        }
        let selected_scope = matches!(request.scope.as_str(), "selected" | "segment");
        if selected_scope && !selected.contains(&segment.id) {
            continue;
        }
        // Transcript timestamps remain untouched. For dubbing only, use the
        // next transcript cue as this segment's playback deadline.
        let slot_end = tts_slot_end(segment, &snapshot.segments);
        if request.scope == "missing"
            && is_cached(
                &store,
                project_id,
                segment,
                &project.translation.target_language,
                &voice,
                speed,
                fit,
                slot_end,
                reference_hash.as_deref(),
                &model_revision,
                &project.dubbing.audio_processing,
            )
        {
            continue;
        }
        let fingerprint = processed_fingerprint(fingerprint(
            segment,
            &project.translation.target_language,
            &voice,
            speed,
            fit,
            slot_end,
            reference_hash.as_deref(),
            &model_revision,
        ), &project.dubbing.audio_processing);
        let raw_key = raw_fingerprint(&segment.translated, &project.translation.target_language, &voice, reference_hash.as_deref(), &model_revision);
        let raw_path = tts_dir.join(format!("raw-{raw_key}.wav"));
        let text_hash = sha256(segment.translated.as_bytes());
        let id_hash = sha256(segment.id.as_bytes());
        let stage_path = tts_dir.join(format!(".{}.{}.tmp.wav", &id_hash[..12], job_id));
        let destination = tts_dir.join(format!(
            "{}-{}-{}.wav",
            &id_hash[..12],
            &fingerprint[..12],
            &job_id_for_name()
        ));
        items.push(WorkItem {
            segment: segment.clone(),
            slot_end,
            fingerprint,
            text_hash,
            stage_path,
            destination,
            reference_audio: reference_audio.clone(),
            model_revision: model_revision.clone(),
            raw_path,
            processing: project.dubbing.audio_processing.clone(),
            source_checksum: project.source.checksum_sha256.clone(),
            dubbing_snapshot: project.dubbing.clone(),
        });
    }
    // Refit from immutable raw cache, independently of the processing fingerprint.
    let mut pending = Vec::new();
    let total_candidates=items.len();
    let mut refitted=0;
    for item in items {
        if canceled.load(Ordering::SeqCst) { return Err("Đã hủy TTS.".into()); }
        if crate::audio::signal::inspect(&item.raw_path).is_ok() {
            fs::copy(&item.raw_path, &item.stage_path).map_err(|e| e.to_string())?;
            refitted+=1;
            match commit_segment(&store, project_id, &project.translation.target_language, &item, &voice, speed, fit, &canceled) {
                Ok((_,warning)) => emit(&app,job_id,project_id,"running",warning.as_deref().unwrap_or("Đã cập nhật giọng đọc từ audio gốc."),refitted as f64/total_candidates.max(1) as f64,refitted,total_candidates,Some(item.segment.id.clone())),
                Err(error) => { record_segment_failure(&store, project_id, &item, &error); return Err(error); }
            }
        } else { pending.push(item); }
    }
    let items = pending;
    if items.is_empty() {
        emit(
            &app,
            job_id,
            project_id,
            "completed",
            if refitted>0 {"Đã cập nhật audio từ bản gốc."}else{"Không có bản dịch mới cần tạo giọng đọc."},
            1.0,
            refitted,
            refitted,
            None,
        );
        return Ok(());
    }

    let cache_dir = crate::runtime_paths::data_dir(&app)
        .map_err(|error| format!("Không thể xác định model cache: {error}"))?
        .join("models")
        .join("huggingface");
    let worker = worker_root(&app)?;
    let segments: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "id": item.segment.id,
                "text": item.segment.translated,
                "voice": voice,
                "outputPath": item.stage_path,
                "referenceAudio": item.reference_audio,
            })
        })
        .collect();
    let message = json!({ "operation":"tts_generate", "cacheDir":cache_dir, "device":device, "segments":segments });
    let mut child = launch_worker(&worker)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Không mở được đầu ra TTS worker.".to_string())?;
    let stderr = child
    .stderr
    .take()
    .ok_or_else(|| "Không mở được log TTS worker.".to_string())?;

    let mut stderr_reader = Some(thread::spawn(move || read_worker_stderr_tail(stderr)));

    let (tx, rx) = mpsc::sync_channel::<String>(8);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            match reader.by_ref().take(8*1024*1024+1).read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) if bytes.len()>8*1024*1024=>break,
                Ok(_) => {
                    // Native runtimes may draw progress with carriage returns.
                    // Split those records so a human-readable progress line cannot
                    // be concatenated with the next JSON protocol event.
                    for line in bytes.split(|byte| *byte == b'\r') {
                        let line = line.strip_suffix(b"\n").unwrap_or(line);
                        if tx.send(String::from_utf8_lossy(line).into_owned()).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| "Không mở được đầu vào TTS worker.".to_string())?;
    writeln!(input, "{message}").map_err(|error| format!("Không thể gửi yêu cầu TTS: {error}"))?;
    drop(input);
    emit(
        &app,
        job_id,
        project_id,
        "running",
        "Đang khởi tạo VieNeu-TTS…",
        0.0,
        0,
        items.len(),
        None,
    );
    let mut counts = JobCounts::default();
    let index: HashMap<&str, &WorkItem> = items
        .iter()
        .map(|item| (item.segment.id.as_str(), item))
        .collect();
    loop {
        if canceled.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let stderr = stderr_reader
                .take()
                .and_then(|handle| handle.join().ok())
                .unwrap_or_default();

            if !stderr.is_empty() {
                log::info!("TTS worker stderr after cancellation: {}", stderr);
            }

            cleanup_staging(&items);
            emit(
                &app,
                job_id,
                project_id,
                "cancelled",
                "Đã hủy tạo giọng đọc. Các segment đã hoàn tất được giữ lại.",
                counts.processed as f64 / items.len() as f64,
                counts.completed,
                items.len(),
                None,
            );
            return Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let trimmed = line
                    .trim()
                    .trim_start_matches('\u{feff}')
                    .trim();

                if trimmed.is_empty() {
                    continue;
                }

                let event: Value = match serde_json::from_str(trimmed) {
                    Ok(value) => value,

                    Err(error) if trimmed.starts_with('{') || trimmed.starts_with('[') => {
                        let _ = child.kill();
                        let _ = child.wait();
                        let stderr = stderr_reader
                            .take()
                            .and_then(|handle| handle.join().ok())
                            .unwrap_or_default();

                        if !stderr.is_empty() {
                            log::error!("TTS worker stderr: {}", stderr);
                        }
                        cleanup_staging(&items);

                        emit(
                            &app,
                            job_id,
                            project_id,
                            if counts.completed > 0 { "partial" } else { "failed" },
                            &format!("TTS worker trả JSON protocol bị lỗi: {error}"),
                            counts.processed as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            None,
                        );

                        return Ok(());
                    }

                    Err(_) => {
                        // Đây chỉ là log/banner của thư viện native.
                        log::warn!("Ignoring non-protocol TTS worker stdout: {}", trimmed);
                        continue;
                    }
                };
                match event["type"].as_str().unwrap_or("") {
                    "progress" => {
                        let progress = event["progress"].as_f64().unwrap_or(0.0);
                        emit(
                            &app,
                            job_id,
                            project_id,
                            "running",
                            event["message"].as_str().unwrap_or("Đang tạo audio…"),
                            progress,
                            counts.completed,
                            items.len(),
                            None,
                        );
                    }
                    "segment_started" => {
                        let id = event["id"].as_str();
                        emit(
                            &app,
                            job_id,
                            project_id,
                            "running",
                            &format!(
                                "Đang tạo {}/{}…",
                                event["index"].as_u64().unwrap_or(0),
                                items.len()
                            ),
                            counts.processed as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            id.map(str::to_string),
                        );
                    }
                    "segment_completed" => {
                        let id = event["id"].as_str().unwrap_or("");
                        let Some(item) = index.get(id) else { continue };
                        if event["outputPath"].as_str()
                            != Some(item.stage_path.to_string_lossy().as_ref())
                        {
                            let _ = fs::remove_file(&item.stage_path);
                            continue;
                        }
                        counts.processed += 1;
                        match commit_segment(
                            &store,
                            project_id,
                            &project.translation.target_language,
                            item,
                            &voice,
                            speed,
                            fit,
                            &canceled,
                        ) {
                            Ok((status, warning)) => {
                                counts.completed += 1;
                                if status == TranscriptTtsStatus::DurationOverflow {
                                    counts.failed += 1;
                                }
                                let msg = warning
                                    .unwrap_or_else(|| format!("Đã tạo audio cho segment {}.", id));
                                emit(
                                    &app,
                                    job_id,
                                    project_id,
                                    "running",
                                    &msg,
                                    counts.processed as f64 / items.len() as f64,
                                    counts.completed,
                                    items.len(),
                                    Some(id.to_string()),
                                );
                            }
                            Err(error) => {
                                counts.failed += 1;
                                record_segment_failure(&store, project_id, item, &error);
                                emit(
                                    &app,
                                    job_id,
                                    project_id,
                                    "running",
                                    &error,
                                    counts.processed as f64 / items.len() as f64,
                                    counts.completed,
                                    items.len(),
                                    Some(id.to_string()),
                                );
                            }
                        }
                    }
                    "segment_failed" => {
                        let id = event["id"].as_str().unwrap_or("");
                        let text = event["message"]
                            .as_str()
                            .unwrap_or("VieNeu không thể tạo audio cho segment này.");
                        if let Some(item) = index.get(id) {
                            record_segment_failure(&store, project_id, item, text);
                        }
                        counts.failed += 1;
                        counts.processed += 1;
                        emit(
                            &app,
                            job_id,
                            project_id,
                            "running",
                            text,
                            counts.processed as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            Some(id.to_string()),
                        );
                    }
                    "failed" => {
                        let text = event["message"].as_str().unwrap_or("TTS worker gặp lỗi.");
                        let _ = child.kill();
                        let _ = child.wait();
                        let stderr = stderr_reader
                            .take()
                            .and_then(|handle| handle.join().ok())
                            .unwrap_or_default();
                        if !stderr.is_empty() {
                            log::error!("TTS worker failed; stderr: {}", stderr);
                        }
                        cleanup_staging(&items);
                        emit(
                            &app,
                            job_id,
                            project_id,
                            "failed",
                            text,
                            counts.processed as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            None,
                        );
                        return Ok(());
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("Không kiểm tra được TTS worker: {error}"))?
                {
                    let stderr = stderr_reader
                        .take()
                        .and_then(|handle| handle.join().ok())
                        .unwrap_or_default();
                    if !stderr.is_empty() {
                        if status.success() {
                            log::info!("TTS worker stderr: {}", stderr);
                        } else {
                            log::error!(
                                "TTS worker exited with {:?}; stderr: {}",
                                status.code(),
                                stderr
                            );
                        }
                    }

                    cleanup_staging(&items);
                    let reported = counts.processed;
                    let all_segments_reported = reported >= items.len();

                    if !all_segments_reported {
                        let terminal_status = if counts.completed > 0 {
                            "partial"
                        } else {
                            "failed"
                        };
                        let message = if status.success() {
                            format!(
                                "TTS worker dừng sớm: {}/{} câu có kết quả. Âm thanh đã tạo được giữ lại.",
                                reported,
                                items.len()
                            )
                        } else {
                            format!(
                                "Tạo giọng đọc dừng bất thường (mã thoát {:?}): {}/{} câu có kết quả. Âm thanh đã tạo được giữ lại.",
                                status.code(),
                                reported,
                                items.len()
                            )
                        };
                        emit(
                            &app,
                            job_id,
                            project_id,
                            terminal_status,
                            &message,
                            counts.processed as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            None,
                        );
                    } else {
                        let terminal_status = if !status.success() {
                            if counts.completed > 0 { "partial" } else { "failed" }
                        } else if counts.failed == 0 {
                            "completed"
                        } else if counts.completed > 0 {
                            "partial"
                        } else {
                            "failed"
                        };
                        let message = if status.success() {
                            format!(
                                "Hoàn tất: {} thành công, {} lỗi.",
                                counts.completed, counts.failed
                            )
                        } else {
                            format!(
                                "Worker kết thúc bất thường sau khi xử lý đủ segment: {} thành công, {} lỗi.",
                                counts.completed, counts.failed
                            )
                        };
                        emit(
                            &app,
                            job_id,
                            project_id,
                            terminal_status,
                            &message,
                            1.0,
                            counts.completed,
                            items.len(),
                            None,
                        );
                    }
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let exit_status = child
                    .wait()
                    .map_err(|error| format!("Không chờ được TTS worker: {error}"))?;
                let stderr = stderr_reader
                    .take()
                    .and_then(|handle| handle.join().ok())
                    .unwrap_or_default();
                if !stderr.is_empty() {
                    if exit_status.success() {
                        log::info!("TTS worker stderr: {}", stderr);
                    } else {
                        log::error!(
                            "TTS worker disconnected with {:?}; stderr: {}",
                            exit_status.code(),
                            stderr
                        );
                    }
                }

                cleanup_staging(&items);
                let reported = counts.processed;
                let all_segments_reported = reported >= items.len();
                let terminal_status = if !all_segments_reported || !exit_status.success() {
                    if counts.completed > 0 { "partial" } else { "failed" }
                } else if counts.failed == 0 {
                    "completed"
                } else if counts.completed > 0 {
                    "partial"
                } else {
                    "failed"
                };
                let message = if all_segments_reported && exit_status.success() {
                    format!("Hoàn tất: {} thành công, {} lỗi.", counts.completed, counts.failed)
                } else if exit_status.success() {
                    format!(
                        "Tạo giọng đọc dừng sớm: {}/{} câu có kết quả. Âm thanh đã tạo được giữ lại.",
                        reported,
                        items.len()
                    )
                } else {
                    format!(
                        "Tạo giọng đọc dừng bất thường ({}): {}/{} câu có kết quả. Âm thanh đã tạo được giữ lại.",
                        exit_status,
                        reported,
                        items.len()
                    )
                };
                emit(
                    &app,
                    job_id,
                    project_id,
                    terminal_status,
                    &message,
                    counts.completed as f64 / items.len() as f64,
                    counts.completed,
                    items.len(),
                    None,
                );
                return Ok(());
            }
        }
    }
}

fn effective_voice(
    requested: Option<&str>,
    project: &crate::project_service::Project,
    settings: &TtsSettings,
) -> Result<String, String> {
    if let Some(voice) = requested {
        return Ok(voice.trim().to_string());
    }
    if let Some(profile_id) = project.dubbing.voice_profile_id.as_deref() {
        return settings
            .voice_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .ok_or_else(|| {
                "Voice profile project đã chọn không còn khả dụng. Chọn profile khác trong Properties.".to_string()
            });
    }
    Ok(project
        .dubbing
        .voice
        .clone()
        .unwrap_or_else(|| settings.default_voice.clone()))
}

fn validate_tts_input(project: &crate::project_service::Project, target_language: &str, item: &WorkItem) -> Result<(), String> {
    let old = &item.dubbing_snapshot;
    let current = &project.dubbing;
    if project.source.checksum_sha256 != item.source_checksum || project.translation.target_language != target_language
        || current.voice != old.voice || current.voice_profile_id != old.voice_profile_id
        || current.speed != old.speed || current.fit_mode != old.fit_mode || current.audio_processing != old.audio_processing {
        return Err("Nguồn, ngôn ngữ hoặc cấu hình giọng đã đổi khi đang tạo audio. Hãy tạo lại đoạn này.".into());
    }
    Ok(())
}

fn commit_segment(
    store: &ProjectStore,
    project_id: &str,
    target_language: &str,
    item: &WorkItem,
    voice: &str,
    speed: f64,
    fit: bool,
    canceled: &AtomicBool,
) -> Result<(TranscriptTtsStatus, Option<String>), String> {
    if !item.stage_path.is_file() {
        return Err("VieNeu không tạo được file WAV hợp lệ.".to_string());
    }
    if !has_wave_header(&item.stage_path) {
        return Err("VieNeu tạo ra file không phải WAV hợp lệ.".to_string());
    }
    let settings = item.processing.clone();
    validate_tts_input(&store.get(project_id)?, target_language, item)?;
    let raw_signal = crate::audio::signal::inspect(&item.stage_path)?;
    if raw_signal.peak <= 0.00001 { return Err("Model tạo giọng đọc trống. Hãy thử tạo lại.".into()); }
    if fs::symlink_metadata(&item.raw_path).is_ok() { crate::audio::signal::regular_file(&item.raw_path)?; }
    if !item.raw_path.exists() {
        let temp = tempfile::NamedTempFile::new_in(item.raw_path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::copy(&item.stage_path, temp.path()).map_err(|e| e.to_string())?;
        temp.persist_noclobber(&item.raw_path).map_err(|e| e.to_string())?;
    } else if crate::audio::signal::inspect(&item.raw_path).is_err() {
        // The raw cache is not authoritative until the generated WAV has passed validation.
        fs::remove_file(&item.raw_path).map_err(|e| e.to_string())?;
        let temp = tempfile::NamedTempFile::new_in(item.raw_path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::copy(&item.stage_path, temp.path()).map_err(|e| e.to_string())?;
        temp.persist_noclobber(&item.raw_path).map_err(|e| e.to_string())?;
    }
    let report = crate::audio::processing::process(&item.raw_path, &item.destination,
        item.slot_end-item.segment.start,item.segment.start,speed,fit,&settings,||canceled.load(Ordering::SeqCst))?;
    let _ = fs::remove_file(&item.stage_path);
    log::info!("TTS audio validated: raw={:.3}s processed={:.3}s slot={:.3}s tempo={:.3} onset={:?} peak={:.4} rms={:.4}",
        report.raw.duration,report.processed.duration,report.slot_duration,report.tempo,report.raw.speech_start,report.raw.peak,report.raw.rms);
    if report.raw.speech_start == Some(0.0) && report.raw.peak > 0.1 {
        log::debug!("TTS raw signal starts at the first sample; inspect raw WAV for possible model onset clipping.");
    }
    let status = if report.overflow { TranscriptTtsStatus::DurationOverflow } else { TranscriptTtsStatus::Ready };
    let warning = report.overflow.then(|| format!("Giọng đọc đoạn {} vượt khung {:.2}s ở giới hạn {:.2}×. Audio được giữ đầy đủ; hãy rút gọn hoặc điều chỉnh tốc độ.", item.segment.id,report.processed.duration-report.lead_seconds-report.slot_duration,settings.max_fit_speed));
    let duration = report.processed.duration;
    let destination_rel = store.content_path(&format!("audio/tts/{}", item.destination.file_name().unwrap_or_default().to_string_lossy()));
    if canceled.load(Ordering::SeqCst) {let _=fs::remove_file(&item.destination);let _=fs::remove_file(item.destination.with_extension("audio.json"));return Err("Đã hủy tạo giọng đọc.".into());}
    transcript_service::with_transcript_lock(&store.transcript_lock_key(project_id), || {
        validate_tts_input(&store.get(project_id)?, target_language, item)?;
        let mut transcript = transcript_service::get_transcript(store, project_id)?.ok_or_else(|| "Transcript đã bị xóa khi TTS đang chạy.".to_string())?;
        if transcript.segments.iter().find(|segment| segment.id == item.segment.id).is_some_and(|segment|
            segment.start != item.segment.start || segment.end != item.segment.end || tts_slot_end(segment, &transcript.segments) != item.slot_end) {
            return Err("Thời gian lời thoại đã thay đổi khi đang tạo giọng. Hãy tạo lại đoạn này.".into());
        }
        let Some(segment) = transcript.segments.iter_mut().find(|segment| segment.id == item.segment.id) else { return Err("Segment đã bị xóa khi TTS đang chạy.".to_string()); };
        if segment.translated != item.segment.translated || segment.source_revision != item.segment.source_revision || segment.translated_revision != item.segment.translated_revision {
            return Err("Bản dịch đã thay đổi trong lúc tạo audio. Audio cũ được giữ nguyên; hãy tạo lại segment.".to_string());
        }
        segment.tts = Some(TranscriptTts {
            audio_path: Some(destination_rel.clone()), duration: Some(duration),
            fingerprint: Some(item.fingerprint.clone()), status: Some(status.clone()),
            voice: Some(voice.to_string()), speed: Some(speed), text_hash: Some(item.text_hash.clone()),
            generated_at: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
            provider: Some("vieneu".to_string()),
            model_id: Some(MODEL_ID.to_string()), warning: warning.clone(),
            model_revision: Some(item.model_revision.clone()),
        });
        transcript_service::save_transcript_locked(store, project_id, transcript)?;
        Ok(())
    }).map_err(|error| { let _ = fs::remove_file(&item.destination); let _ = fs::remove_file(item.destination.with_extension("audio.json")); error })?;
    let _ = target_language;
    Ok((status, warning))
}

#[cfg(test)]
fn plan_audio_tempo(audio_duration: f64, slot_duration: f64, speed: f64, fit: bool) -> (f64, bool) {
    crate::audio::processing::tempo(audio_duration, slot_duration, speed, fit, 1.2)
}
fn processed_fingerprint(base: String, settings: &crate::audio::ProcessingSettings) -> String {
    sha256(format!("audio-v2\0{base}\0{}",serde_json::to_string(settings).unwrap_or_default()).as_bytes())
}
fn raw_fingerprint(text: &str, language: &str, voice: &str, reference: Option<&str>, revision: &str) -> String {
    sha256(format!("raw-v2\0{MODEL_ID}\0{revision}\0{text}\0{language}\0{voice}\0{}",reference.unwrap_or("preset")).as_bytes())
}

fn record_segment_failure(store: &ProjectStore, project_id: &str, item: &WorkItem, message: &str) {
    let _ = fs::remove_file(&item.stage_path);
    let _ = transcript_service::with_transcript_lock(&store.transcript_lock_key(project_id), || {
        let Some(mut transcript) = transcript_service::get_transcript(store, project_id)? else {
            return Ok(());
        };
        let Some(segment) = transcript
            .segments
            .iter_mut()
            .find(|segment| segment.id == item.segment.id)
        else {
            return Ok(());
        };
        if segment.translated != item.segment.translated
            || segment.source_revision != item.segment.source_revision
        {
            return Ok(());
        }
        let current = segment.tts.get_or_insert(TranscriptTts {
            audio_path: None,
            duration: None,
            fingerprint: None,
            status: None,
            voice: None,
            speed: None,
            text_hash: None,
            generated_at: None,
            provider: None,
            model_id: None,
            warning: None,
            model_revision: None,
        });
        if current.audio_path.is_none() {
            current.status = Some(TranscriptTtsStatus::Failed);
        }
        current.warning = Some(message.chars().take(400).collect());
        transcript_service::save_transcript_locked(store, project_id, transcript)?;
        Ok(())
    });
}

fn is_cached(
    store: &ProjectStore,
    project_id: &str,
    segment: &TranscriptSegment,
    target_language: &str,
    voice: &str,
    speed: f64,
    fit: bool,
    slot_end: f64,
    reference_hash: Option<&str>,
    model_revision: &str,
    settings: &crate::audio::ProcessingSettings,
) -> bool {
    let Some(tts) = &segment.tts else {
        return false;
    };
    if tts.fingerprint.as_deref()
        != Some(
            processed_fingerprint(fingerprint(
                segment,
                target_language,
                voice,
                speed,
                fit,
                slot_end,
                reference_hash,
                model_revision,
            ), settings)
            .as_str(),
        )
    {
        return false;
    }
    let Some(path) = tts.audio_path.as_deref() else {
        return false;
    };
    resolve_tts_file(store, project_id, path).is_ok_and(|p| {
        crate::audio::signal::inspect(&p).is_ok_and(|signal| {
            crate::audio::processing::report(&p).is_some_and(|r| r.settings == *settings && (r.processed.duration-signal.duration).abs()<0.002)
        })
    })
        && tts.voice.as_deref() == Some(voice)
        && (tts.speed.unwrap_or(1.0) - speed).abs() < 0.001
        && matches!(tts.status, Some(TranscriptTtsStatus::Ready | TranscriptTtsStatus::DurationOverflow))
        && tts.text_hash.as_deref() == Some(sha256(segment.translated.as_bytes()).as_str())
}

fn fingerprint(
    segment: &TranscriptSegment,
    target_language: &str,
    voice: &str,
    speed: f64,
    fit: bool,
    slot_end: f64,
    reference_hash: Option<&str>,
    model_revision: &str,
) -> String {
    sha256(
        format!(
            "{MODEL_ID}\0{model_revision}\0{}\0{}\0{}\0{}\0{voice}\0{speed:.4}\0{fit}\0{slot_end:.4}\0{}",
            segment.id,
            segment.translated,
            segment.source_revision,
            target_language,
            reference_hash.unwrap_or("preset")
        )
        .as_bytes(),
    )
}

fn tts_slot_end(segment: &TranscriptSegment, segments: &[TranscriptSegment]) -> f64 {
    let next_start = segments
        .iter()
        .filter(|candidate| {
            candidate.id != segment.id && candidate.start > segment.start + 0.001
        })
        .map(|candidate| candidate.start)
        .min_by(f64::total_cmp);

    // The next cue is the TTS deadline even when ASR timestamps overlap. This
    // keeps generated dialogue from running over the following line, without
    // changing subtitle timestamps.
    next_start.unwrap_or(segment.end)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn resolve_tts_file(store: &ProjectStore, project_id: &str, relative: &str) -> Result<PathBuf, String> {
    store.validate_audio_owner(project_id, relative)?;
    resolve_project_file(store, project_id, relative, "audio/tts")
}

fn safe_tts_directory(store: &ProjectStore, project_id: &str) -> Result<PathBuf, String> {
    store.data_directory(project_id, "audio/tts")
}

fn job_id_for_name() -> String {
    Uuid::new_v4().simple().to_string()[..10].to_string()
}

fn cleanup_staging(items: &[WorkItem]) {
    for item in items {
        let _ = fs::remove_file(&item.stage_path);
        let _ = fs::remove_file(item.stage_path.with_extension("processed.wav"));
    }
}

fn emit(
    app: &AppHandle,
    job_id: &str,
    project_id: &str,
    status: &str,
    message: &str,
    progress: f64,
    completed: usize,
    total: usize,
    segment_id: Option<String>,
) {
    let event = TtsJobEvent {
        target: job_target(app, job_id),
        project_id: project_id.to_string(),
        job_id: job_id.to_string(),
        status: status.to_string(),
        message: message.to_string(),
        progress,
        completed_segments: completed,
        total_segments: total,
        segment_id,
    };
    crate::job_service::sync_app_event(app, "tts", &event);
    let _ = app.emit("tts-progress", event);
}

fn project_store(app: &AppHandle) -> Result<ProjectStore, String> {
    let root = crate::runtime_paths::data_dir(app)
        .map_err(|error| format!("Không thể xác định thư mục dự án: {error}"))?
        .join("projects");
    Ok(ProjectStore::new(root))
}

pub(crate) fn worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::ai_worker_dir()
}

fn launch_worker(worker: &Path) -> Result<crate::worker_manager::ManagedWorker, String> {
    crate::runtime_paths::worker_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_worker()
        .map_err(|error| {
            format!(
                "Không thể khởi chạy bộ xử lý âm thanh. Kiểm tra thành phần ứng dụng trong Cài đặt rồi thử lại: {error}"
            )
        })
}

pub(crate) fn worker_request(
    worker: &Path,
    request: Value,
    canceled: Option<Arc<AtomicBool>>,
    mut on_event: impl FnMut(Value),
) -> Result<Value, String> {
    let mut child = launch_worker(worker)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Không mở được đầu ra Python worker.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Không mở được log Python worker.".to_string())?;
    let stderr_reader = thread::spawn(move || read_worker_stderr_tail(stderr));
    let (tx, rx) = mpsc::sync_channel::<String>(8);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            match reader.by_ref().take(8*1024*1024+1).read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // Some download libraries draw progress with carriage returns
                    // instead of newlines. Split those too, so they cannot be
                    // concatenated with the worker's next JSON event.
                    for line in bytes.split(|byte| *byte == b'\r') {
                        let line = line.strip_suffix(b"\n").unwrap_or(line);
                        if tx.send(String::from_utf8_lossy(line).into_owned()).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Không mở được đầu vào Python worker.".to_string())?;
    writeln!(stdin, "{request}")
        .map_err(|error| format!("Không thể gửi yêu cầu cho Python worker: {error}"))?;
    drop(stdin);
    let mut terminal = None;
    let mut non_protocol_output = Vec::new();
    loop {
        if canceled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Đã hủy tác vụ TTS.".to_string());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let trimmed = line.trim().trim_start_matches('\u{feff}').trim();
                if trimmed.is_empty() {
                    continue;
                }
                let value: Value = match serde_json::from_str(trimmed) {
                    Ok(value) => value,
                    Err(error) if trimmed.starts_with('{') || trimmed.starts_with('[') => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("Không đọc được kết quả tạo giọng đọc: {error}"));
                    }
                    // Some libraries used during model downloads write human-readable
                    // progress to stdout. The worker protocol is JSONL, so ignore that
                    // noise and keep waiting for its actual progress/completion events.
                    Err(_) => {
                        remember_worker_output(&mut non_protocol_output, trimmed);
                        continue;
                    }
                };
                if !value.is_object() || value.get("type").and_then(Value::as_str).is_none() {
                    remember_worker_output(&mut non_protocol_output, trimmed);
                    continue;
                }
                match value["type"].as_str().unwrap_or("") {
                    "failed" => {
                        let message = value["message"]
                            .as_str()
                            .unwrap_or("Không thể tạo giọng đọc.");
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(message.to_string());
                    }
                    "capabilities" | "completed" => terminal = Some(value.clone()),
                    _ => {}
                }
                on_event(value);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("Không kiểm tra được Python worker: {error}"))?
                {
                    let stderr = stderr_reader.join().unwrap_or_default();
                    if status.success() {
                        return terminal
                            .ok_or_else(|| worker_missing_result(&non_protocol_output, &stderr, status.code()));
                    }
                    return Err(worker_missing_result(&non_protocol_output, &stderr, status.code()));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().ok();
    let stderr = stderr_reader.join().unwrap_or_default();
    terminal.ok_or_else(|| worker_missing_result(&non_protocol_output, &stderr, status.and_then(|s| s.code())))
}

fn remember_worker_output(lines: &mut Vec<String>, value: &str) {
    let concise: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect();
    if concise.trim().is_empty() {
        return;
    }
    if lines.len() == 6 {
        lines.remove(0);
    }
    lines.push(concise);
}

fn read_worker_stderr_tail(mut reader: impl Read) -> String {
    const MAX_BYTES: usize = 4096;
    let mut tail = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > MAX_BYTES {
                    let excess = tail.len() - MAX_BYTES;
                    tail.drain(..excess);
                }
            }
        }
    }
    String::from_utf8_lossy(&tail).trim().to_string()
}

fn worker_missing_result(stdout: &[String], stderr: &str, exit_code: Option<i32>) -> String {
    let mut details = Vec::new();
    if let Some(code) = exit_code {
        details.push(format!("mã thoát {code}"));
    }
    if !stderr.trim().is_empty() {
        details.push(format!("log: {}", stderr.trim()));
    }
    if !stdout.is_empty() {
        details.push(format!("đầu ra: {}", stdout.join(" | ")));
    }
    if details.is_empty() {
        "Python worker đã dừng mà không trả kết quả VieNeu-TTS. Hãy thử lại; nếu lỗi còn xảy ra, mở Cài đặt → Giọng đọc để kiểm tra thành phần đã cài.".to_string()
    } else {
        format!("Python worker không hoàn tất tác vụ VieNeu-TTS ({})", details.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn raw_cache_survives_fit_changes_but_processing_and_language_invalidate() {
        let raw = raw_fingerprint("Ma Đế.","vi","Ngọc Huyền",None,"revision");
        assert_eq!(raw,raw_fingerprint("Ma Đế.","vi","Ngọc Huyền",None,"revision"));
        assert_ne!(raw,raw_fingerprint("Ma Đế.","en","Ngọc Huyền",None,"revision"));
        assert_ne!(raw,raw_fingerprint("Ma Đế.","vi","other",None,"revision"));
        let mut settings=crate::audio::ProcessingSettings::default();
        let before=processed_fingerprint(raw.clone(),&settings);
        settings.head_padding_ms=120;
        assert_ne!(before,processed_fingerprint(raw,&settings));
    }

    #[test]
    fn model_cache_removal_preserves_unrelated_hugging_face_repositories() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("models").join("huggingface");
        let hub = root.join("hub");
        let vieneu = hub.join("models--pnnbao-ump--VieNeu-TTS-v3-Turbo");
        let moss = hub.join("models--OpenMOSS-Team--MOSS-Audio-Tokenizer-Nano-ONNX");
        let unrelated = hub.join("models--user--unrelated");
        fs::create_dir_all(&vieneu).unwrap();
        fs::create_dir_all(&moss).unwrap();
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(root.join("vieneu-model-manifest.json"), b"manifest").unwrap();
        fs::write(unrelated.join("keep.txt"), b"keep").unwrap();

        remove_model_cache(&root).unwrap();

        assert!(!vieneu.exists());
        assert!(!moss.exists());
        assert!(!root.join("vieneu-model-manifest.json").exists());
        assert_eq!(fs::read(unrelated.join("keep.txt")).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn model_cache_removal_refuses_a_managed_repo_symlink() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("models").join("huggingface");
        let hub = root.join("hub");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&hub).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        std::os::unix::fs::symlink(
            &outside,
            hub.join("models--pnnbao-ump--VieNeu-TTS-v3-Turbo"),
        )
        .unwrap();

        assert!(remove_model_cache(&root).is_err());
        assert!(outside.join("keep.txt").is_file());
    }

    fn sample() -> TranscriptSegment {
        TranscriptSegment {
            id: "one".into(),
            start: 0.0,
            end: 2.0,
            source: "hello".into(),
            translated: "xin chào".into(),
            source_revision: 2,
            translated_revision: Some(2),
            translation_meta: None,
            speaker_id: None,
            source_type: crate::project_service::TranscriptSourceType::Asr,
            translation_status: crate::transcript_service::TranscriptTranslationStatus::Translated,
            tts: None,
        }
    }

    #[test]
    fn cache_fingerprint_changes_when_translation_voice_or_speed_changes() {
        let item = sample();
        assert_eq!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, 2.0, None, "rev-1"),
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, 2.0, None, "rev-1")
        );
        assert_ne!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, 2.0, None, "rev-1"),
            fingerprint(&item, "vi", "Minh Quân Pro", 1.0, false, 2.0, None, "rev-1")
        );
        assert_ne!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, 2.0, None, "rev-1"),
            fingerprint(&item, "vi", "Mai Anh", 1.1, false, 2.0, None, "rev-1")
        );
        let mut changed = item.clone();
        changed.translated.push('!');
        assert_ne!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, 2.0, None, "rev-1"),
            fingerprint(&changed, "vi", "Mai Anh", 1.0, false, 2.0, None, "rev-1")
        );
    }

    #[test]
    fn output_path_is_relative_and_scoped_to_tts_directory() {
        let root = PathBuf::from("C:/app/projects/id/audio/tts/asset.wav");
        assert!(root.starts_with(PathBuf::from("C:/app/projects/id/audio/tts")));
    }

    #[test]
    fn fit_respects_tempo_limit_and_reports_remaining_overflow() {
        let (fitted_tempo, fits) = plan_audio_tempo(2.3, 2.0, 1.0, true);
        assert!((fitted_tempo - 1.15).abs() < 0.001);
        assert!(!fits);

        let (limited_tempo, still_overflows) = plan_audio_tempo(5.0, 2.0, 1.0, true);
        assert!((limited_tempo - 1.2).abs() < 0.001);
        assert!(still_overflows);

        let (short_tempo, short_overflow) = plan_audio_tempo(0.7, 1.0, 1.0, true);
        assert!((short_tempo - 1.0).abs() < 0.001);
        assert!(!short_overflow);
    }

    #[test]
    fn custom_speed_is_applied_and_natural_mode_does_not_speed_up() {
        let (natural, overflow) = plan_audio_tempo(2.3, 2.0, 1.0, false);
        assert_eq!(natural, 1.0);
        assert!(overflow);

        let (custom, custom_fits) = plan_audio_tempo(3.5, 2.0, 2.0, false);
        assert_eq!(custom, 2.0);
        assert!(!custom_fits);
    }

    #[test]
    fn wav_validation_requires_a_real_wave_header_and_minimum_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio.wav");
        let mut bytes = vec![0; 44];
        bytes[..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WAVE");
        fs::write(&path, bytes).unwrap();
        assert!(has_wave_header(&path));
        fs::write(&path, [0; 44]).unwrap();
        assert!(!has_wave_header(&path));
        fs::write(&path, b"RIFF").unwrap();
        assert!(!has_wave_header(&path));
    }

    #[test]
    fn cancellation_sets_only_the_requested_job_flag() {
        let jobs = TtsJobs::default();
        let canceled = Arc::new(AtomicBool::new(false));
        jobs.inner.lock().unwrap().insert(
            "job-1".to_string(),
            TtsJobEntry {
                target: Default::default(),
                project_id: "project-1".to_string(),
                canceled: canceled.clone(),
            },
        );
        assert!(cancel(&jobs, "job-1").unwrap());
        assert!(canceled.load(Ordering::SeqCst));
        assert!(!cancel(&jobs, "missing").unwrap());
    }

    #[test]
    fn reopening_a_project_marks_missing_audio_failed_without_removing_its_reference() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let mut project = store.create("Missing TTS file").unwrap();
        project.source.video_path = Some("media/source/video.mp4".to_string());
        project.source.checksum_sha256 = Some("c".repeat(64));
        project.source.duration = Some(4.0);
        let project = store.update_project(&project).unwrap();
        let mut segment = sample();
        segment.tts = Some(TranscriptTts {
            audio_path: Some("audio/tts/missing.wav".to_string()),
            duration: Some(1.0),
            fingerprint: Some("fingerprint".to_string()),
            status: Some(TranscriptTtsStatus::Ready),
            voice: Some("Ngọc Lan".to_string()),
            speed: Some(1.0),
            text_hash: Some("hash".to_string()),
            generated_at: Some("2026-09-23T00:00:00Z".to_string()),
            warning: None,
            provider: Some("vieneu".to_string()),
            model_id: Some(MODEL_ID.to_string()),
            model_revision: Some("revision".to_string()),
        });
        let transcript = Transcript {
            schema_version: 1,
            project_id: project.id.clone(), video_content_id: None,
            source_type: crate::project_service::TranscriptSourceType::Asr,
            source_track: None,
            language: Some("en".to_string()),
            video_checksum: project.source.checksum_sha256.clone(),
            segments: vec![segment],
        };
        transcript_service::save_transcript(&store, &project.id, transcript).unwrap();

        let loaded = get_transcript_with_audio_validation(&store, &project.id)
            .unwrap()
            .unwrap();
        let audio = loaded.segments[0].tts.as_ref().unwrap();
        assert_eq!(audio.status, Some(TranscriptTtsStatus::Failed));
        assert_eq!(audio.audio_path.as_deref(), Some("audio/tts/missing.wav"));
        assert!(audio.warning.as_deref().unwrap().contains("thiếu"));
    }
}

fn job_target(app: &AppHandle, job_id: &str) -> crate::video_content::JobTarget {
    app.try_state::<TtsJobs>()
        .and_then(|jobs| jobs.inner.lock().ok().and_then(|entries| entries.get(job_id).map(|entry| entry.target.clone())))
        .unwrap_or_default()
}
pub fn has_active_content_job(jobs: &TtsJobs, project_id: &str, content_id: Option<&str>) -> bool {
    jobs.inner.lock().map(|entries| entries.values().any(|entry| entry.project_id == project_id
        && (entry.target.video_content_id.as_deref() == content_id || entry.target.video_content_id.is_none())))
        .unwrap_or(true)
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use crate::project_service::TranscriptSourceType;

    fn wave(path: &Path, frequency: f32) {
        let mut writer = hound::WavWriter::create(path, hound::WavSpec { channels:1, sample_rate:16000,
            bits_per_sample:32, sample_format:hound::SampleFormat::Float }).unwrap();
        for index in 0..8000 {
            writer.write_sample((index as f32 * frequency * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.2).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn two_video_jobs_cancel_and_lock_independently() {
        let jobs = TtsJobs::default();
        for id in ["a", "b"] {
            jobs.inner.lock().unwrap().insert(id.into(), TtsJobEntry { project_id:"project".into(),
                target:crate::video_content::JobTarget {video_content_id:Some(id.into()),content_revision:Some(3)},
                canceled:Arc::new(AtomicBool::new(false)) });
        }
        assert!(has_active_content_job(&jobs,"project",Some("a")));
        assert!(has_active_project_job(&jobs,"project"));
        assert!(!has_active_content_job(&jobs,"project",Some("other")));
        assert!(cancel(&jobs,"a").unwrap());
        let entries=jobs.inner.lock().unwrap();
        assert!(entries["a"].canceled.load(Ordering::SeqCst));
        assert!(!entries["b"].canceled.load(Ordering::SeqCst));
    }

    #[test]
    fn generated_wavs_commit_to_their_video_after_reorder_and_reject_changed_voice() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let first=store.for_video(&project.id,Some(&a)).unwrap();
        let second=store.for_video(&project.id,Some(&b)).unwrap();
        let mut items=Vec::new();
        for (scoped, frequency) in [(&first,500.0),(&second,900.0)] {
            let view=scoped.get(&project.id).unwrap();
            let mut segment=TranscriptSegment::new(0.0,1.0,"source".into(),TranscriptSourceType::Asr);
            segment.translated="Xin chào".into();
            segment.translation_status=transcript_service::TranscriptTranslationStatus::Translated;
            let value=Transcript {schema_version:1,project_id:project.id.clone(),video_content_id:scoped.content_id().map(str::to_owned),
                source_type:TranscriptSourceType::Asr,source_track:None,language:Some("vi".into()),video_checksum:view.source.checksum_sha256.clone(),segments:vec![segment.clone()]};
            transcript_service::save_transcript(scoped,&project.id,value).unwrap();
            let directory=safe_tts_directory(scoped,&project.id).unwrap();
            let stage=directory.join("same-stage.wav");wave(&stage,frequency);
            items.push(WorkItem {segment,slot_end:1.0,fingerprint:"fingerprint".into(),text_hash:"text".into(),stage_path:stage,
                destination:directory.join("same-cache-key.wav"),reference_audio:None,model_revision:"test".into(),raw_path:directory.join("same-raw.wav"),
                processing:view.dubbing.audio_processing.clone(),source_checksum:view.source.checksum_sha256.clone(),dubbing_snapshot:view.dubbing});
        }
        store.edit_project(&project.id,|project| { let doc=project.timeline.as_mut().unwrap();
            doc.clips[0].start_us=10_000_000;doc.clips[1].start_us=0;doc.clips[1].source_in_us=100_000;Ok(()) }).unwrap();
        let canceled=AtomicBool::new(false);
        for (scoped,item) in [(&first,&items[0]),(&second,&items[1])] {
            commit_segment(scoped,&project.id,"vi",item,"Ngọc Huyền",1.0,false,&canceled).unwrap();
            let value=transcript_service::get_transcript(scoped,&project.id).unwrap().unwrap();
            assert_eq!(value.segments[0].tts.as_ref().unwrap().audio_path.as_deref(),Some(scoped.content_path("audio/tts/same-cache-key.wav").as_str()));
            assert!(audio_path(scoped,&project.id,&item.segment.id).unwrap().ends_with("same-cache-key.wav"));
        }
        assert_ne!(std::fs::read(&items[0].destination).unwrap(),std::fs::read(&items[1].destination).unwrap());
        let mut changed=second.get(&project.id).unwrap();changed.dubbing.voice=Some("Different voice".into());second.update_project(&changed).unwrap();
        wave(&items[1].stage_path,1200.0);
        let saved=std::fs::read(&items[1].destination).unwrap();
        assert!(commit_segment(&second,&project.id,"vi",&items[1],"Ngọc Huyền",1.0,false,&canceled).is_err());
        assert_eq!(std::fs::read(&items[1].destination).unwrap(),saved);
    }
}
