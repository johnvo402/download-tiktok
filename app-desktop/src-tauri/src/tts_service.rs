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
use std::process::{Child, Stdio};
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
    project_id: String,
    canceled: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsJobEvent {
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
    fingerprint: String,
    text_hash: String,
    stage_path: PathBuf,
    destination: PathBuf,
    reference_audio: Option<PathBuf>,
    model_revision: String,
}

#[derive(Default)]
struct JobCounts {
    completed: usize,
    failed: usize,
}

pub fn has_active_project_job(jobs: &TtsJobs, project_id: &str) -> bool {
    jobs.inner
        .lock()
        .map(|jobs| jobs.values().any(|job| job.project_id == project_id))
        .unwrap_or(true)
}

pub fn get_environment(app: AppHandle) -> Result<TtsEnvironment, String> {
    let cache = app
        .path()
        .app_data_dir()
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
    let cache = app
        .path()
        .app_data_dir()
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
    if !active.is_empty() {
        return Err("Không thể gỡ VieNeu-TTS khi đang chạy một tác vụ giọng đọc.".to_string());
    }
    drop(active);
    let data_root = app
        .path()
        .app_data_dir()
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
    let root = app
        .path()
        .app_data_dir()
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
        .output()
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
    let root = app
        .path()
        .app_data_dir()
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
        .output()
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
    request: TtsJobRequest,
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
    let store = project_store(&app)?;
    let project = store.get(&project_id)?;
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
        project_id: project_id.clone(),
        canceled: canceled.clone(),
    };
    {
        let mut all_jobs = jobs
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái TTS jobs.".to_string())?;
        if !all_jobs.is_empty() {
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
    );
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
    resolve_project_file(store, project_id, relative, "audio/tts")
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn get_transcript_with_audio_validation(
    store: &ProjectStore,
    project_id: &str,
) -> Result<Option<Transcript>, String> {
    transcript_service::with_transcript_lock(project_id, || {
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
                .and_then(|path| resolve_project_file(store, project_id, path, "audio/tts").ok())
                .is_some_and(|path| has_wave_header(&path));
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
    let store = project_store(&app)?;
    let project = store.get(project_id)?;
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
        if request.scope == "missing"
            && is_cached(
                &store,
                project_id,
                segment,
                &project.translation.target_language,
                &voice,
                speed,
                fit,
                reference_hash.as_deref(),
                &model_revision,
            )
        {
            continue;
        }
        let fingerprint = fingerprint(
            segment,
            &project.translation.target_language,
            &voice,
            speed,
            fit,
            reference_hash.as_deref(),
            &model_revision,
        );
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
            fingerprint,
            text_hash,
            stage_path,
            destination,
            reference_audio: reference_audio.clone(),
            model_revision: model_revision.clone(),
        });
    }
    if items.is_empty() {
        emit(
            &app,
            job_id,
            project_id,
            "completed",
            "Không có bản dịch mới cần tạo giọng đọc.",
            1.0,
            0,
            0,
            None,
        );
        return Ok(());
    }

    let cache_dir = app
        .path()
        .app_data_dir()
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
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            if tx.send(line).is_err() {
                break;
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
            cleanup_staging(&items);
            emit(
                &app,
                job_id,
                project_id,
                "cancelled",
                "Đã hủy tạo giọng đọc. Các segment đã hoàn tất được giữ lại.",
                counts.completed as f64 / items.len() as f64,
                counts.completed,
                items.len(),
                None,
            );
            return Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let event: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        cleanup_staging(&items);
                        emit(
                            &app,
                            job_id,
                            project_id,
                            if counts.completed > 0 {
                                "partial"
                            } else {
                                "failed"
                            },
                            &format!("TTS worker trả dữ liệu JSON không hợp lệ: {error}"),
                            counts.completed as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            None,
                        );
                        return Ok(());
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
                            counts.completed as f64 / items.len() as f64,
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
                        match commit_segment(
                            &store,
                            project_id,
                            &project.translation.target_language,
                            item,
                            &voice,
                            speed,
                            fit,
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
                                    counts.completed as f64 / items.len() as f64,
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
                                    counts.completed as f64 / items.len() as f64,
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
                        emit(
                            &app,
                            job_id,
                            project_id,
                            "running",
                            text,
                            (counts.completed + counts.failed) as f64 / items.len() as f64,
                            counts.completed,
                            items.len(),
                            Some(id.to_string()),
                        );
                    }
                    "failed" => {
                        let text = event["message"].as_str().unwrap_or("TTS worker gặp lỗi.");
                        cleanup_staging(&items);
                        emit(
                            &app,
                            job_id,
                            project_id,
                            "failed",
                            text,
                            counts.completed as f64 / items.len() as f64,
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
                    if !status.success() && counts.completed + counts.failed < items.len() {
                        cleanup_staging(&items);
                        emit(&app, job_id, project_id, if counts.completed > 0 { "partial" } else { "failed" }, "VieNeu-TTS worker đã dừng bất thường. Audio đã tạo thành công vẫn được giữ.", counts.completed as f64 / items.len() as f64, counts.completed, items.len(), None);
                    } else {
                        cleanup_staging(&items);
                        let status = if counts.failed == 0 {
                            "completed"
                        } else if counts.completed > 0 {
                            "partial"
                        } else {
                            "failed"
                        };
                        emit(
                            &app,
                            job_id,
                            project_id,
                            status,
                            &format!(
                                "Hoàn tất: {} thành công, {} lỗi.",
                                counts.completed, counts.failed
                            ),
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
                cleanup_staging(&items);
                let all_segments_reported = counts.completed + counts.failed >= items.len();
                let terminal_status = if all_segments_reported && counts.failed == 0 {
                    "completed"
                } else if counts.completed > 0 {
                    "partial"
                } else {
                    "failed"
                };
                let message = if all_segments_reported {
                    format!("Hoàn tất: {} thành công, {} lỗi.", counts.completed, counts.failed)
                } else if exit_status.success() {
                    format!(
                        "TTS worker đã kết thúc sớm: {}/{} segment có kết quả. Audio đã tạo được giữ lại.",
                        counts.completed + counts.failed,
                        items.len()
                    )
                } else {
                    format!(
                        "TTS worker dừng bất thường ({}): {}/{} audio đã tạo được giữ lại.",
                        exit_status,
                        counts.completed,
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

fn commit_segment(
    store: &ProjectStore,
    project_id: &str,
    target_language: &str,
    item: &WorkItem,
    voice: &str,
    speed: f64,
    fit: bool,
) -> Result<(TranscriptTtsStatus, Option<String>), String> {
    if !item.stage_path.is_file() {
        return Err("VieNeu không tạo được file WAV hợp lệ.".to_string());
    }
    if !has_wave_header(&item.stage_path) {
        return Err("VieNeu tạo ra file không phải WAV hợp lệ.".to_string());
    }
    let mut warning = None;
    let mut status = TranscriptTtsStatus::Ready;
    let initial_duration = probe_duration(&item.stage_path)?;
    let mut filters = Vec::new();
    let slot_duration = item.segment.end - item.segment.start;
    let (applied_tempo, duration_overflow) =
        plan_audio_tempo(initial_duration, slot_duration, speed, fit);
    if (applied_tempo - 1.0).abs() > 0.001 {
        filters.push(format!("atempo={applied_tempo:.4}"));
    }
    if duration_overflow {
        status = TranscriptTtsStatus::DurationOverflow;
        warning = Some(if fit {
            format!(
                "Audio vẫn dài hơn khung segment {} sau khi Fit đến giới hạn 2.00×.",
                item.segment.id
            )
        } else {
            format!(
                "Audio dài hơn khung thoại segment {}. Chọn chế độ Fit hoặc rút gọn bản dịch.",
                item.segment.id
            )
        });
    }
    if !filters.is_empty() {
        let processed = item.stage_path.with_extension("processed.wav");
        let output = crate::runtime_paths::ffmpeg_command()?
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&item.stage_path)
            .args(["-filter:a", &filters.join(","), "-c:a", "pcm_s16le"])
            .arg(&processed)
            .output()
            .map_err(|error| format!("Không thể chạy FFmpeg để điều chỉnh audio TTS: {error}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            let _ = fs::remove_file(&processed);
            return Err(format!(
                "Không thể điều chỉnh tốc độ audio TTS: {}",
                detail.trim()
            ));
        }
        fs::rename(&processed, &item.stage_path)
            .map_err(|error| format!("Không thể thay audio tạm sau Fit: {error}"))?;
    }
    let duration = probe_duration(&item.stage_path)?;
    let destination_rel = format!(
        "audio/tts/{}",
        item.destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    if item.destination.exists() {
        fs::remove_file(&item.destination)
            .map_err(|error| format!("Không thể thay file audio TTS cũ: {error}"))?;
    }
    fs::rename(&item.stage_path, &item.destination)
        .map_err(|error| format!("Không thể lưu audio TTS trong dự án: {error}"))?;
    transcript_service::with_transcript_lock(project_id, || {
        let mut transcript = transcript_service::get_transcript(store, project_id)?.ok_or_else(|| "Transcript đã bị xóa khi TTS đang chạy.".to_string())?;
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
    }).map_err(|error| { let _ = fs::remove_file(&item.destination); error })?;
    let _ = target_language;
    Ok((status, warning))
}

fn plan_audio_tempo(audio_duration: f64, slot_duration: f64, speed: f64, fit: bool) -> (f64, bool) {
    let requested = if fit {
        (audio_duration / slot_duration).max(1.0)
    } else {
        speed
    };
    let applied = if fit { requested.min(2.0) } else { requested };
    (applied, audio_duration / applied > slot_duration + 0.02)
}

fn record_segment_failure(store: &ProjectStore, project_id: &str, item: &WorkItem, message: &str) {
    let _ = fs::remove_file(&item.stage_path);
    let _ = transcript_service::with_transcript_lock(project_id, || {
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
    reference_hash: Option<&str>,
    model_revision: &str,
) -> bool {
    let Some(tts) = &segment.tts else {
        return false;
    };
    if tts.fingerprint.as_deref()
        != Some(
            fingerprint(
                segment,
                target_language,
                voice,
                speed,
                fit,
                reference_hash,
                model_revision,
            )
            .as_str(),
        )
    {
        return false;
    }
    let Some(path) = tts.audio_path.as_deref() else {
        return false;
    };
    resolve_project_file(store, project_id, path, "audio/tts").is_ok()
        && tts.voice.as_deref() == Some(voice)
        && (tts.speed.unwrap_or(1.0) - speed).abs() < 0.001
        && tts.status == Some(TranscriptTtsStatus::Ready)
        && tts.text_hash.as_deref() == Some(sha256(segment.translated.as_bytes()).as_str())
}

fn fingerprint(
    segment: &TranscriptSegment,
    target_language: &str,
    voice: &str,
    speed: f64,
    fit: bool,
    reference_hash: Option<&str>,
    model_revision: &str,
) -> String {
    sha256(
        format!(
            "{MODEL_ID}\0{model_revision}\0{}\0{}\0{}\0{}\0{voice}\0{speed:.4}\0{fit}\0{}",
            segment.id,
            segment.translated,
            segment.source_revision,
            target_language,
            reference_hash.unwrap_or("preset")
        )
        .as_bytes(),
    )
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn probe_duration(path: &Path) -> Result<f64, String> {
    let output = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Không thể chạy FFprobe để kiểm tra audio TTS: {error}"))?;
    if !output.status.success() {
        return Err("Audio TTS không hợp lệ hoặc FFprobe không đọc được file.".to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "Audio TTS có thời lượng không hợp lệ.".to_string())
}

fn safe_tts_directory(store: &ProjectStore, project_id: &str) -> Result<PathBuf, String> {
    let root = store
        .project_directory(project_id)?
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục dự án: {error}"))?;
    let audio = root.join("audio");
    let tts = audio.join("tts");
    for directory in [&audio, &tts] {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("Thư mục audio trong dự án không an toàn.".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(directory)
                .map_err(|create| format!("Không thể tạo thư mục audio TTS: {create}"))?,
            Err(error) => return Err(format!("Không thể kiểm tra thư mục audio: {error}")),
        }
    }
    let canonical = tts
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục audio TTS: {error}"))?;
    if !canonical.starts_with(&root) {
        return Err("Đường dẫn audio TTS vượt khỏi thư mục dự án.".to_string());
    }
    Ok(canonical)
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
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục dự án: {error}"))?
        .join("projects");
    Ok(ProjectStore::new(root))
}

fn worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::worker_dir()
}

fn launch_worker(worker: &Path) -> Result<Child, String> {
    crate::runtime_paths::worker_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "Không thể khởi chạy Python worker. Hãy cài Python 3.10+ và dependencies: {error}"
            )
        })
}

fn worker_request(
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
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            if tx.send(line).is_err() {
                break;
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
                let value: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("JSON từ TTS worker không hợp lệ: {error}"));
                    }
                };
                match value["type"].as_str().unwrap_or("") {
                    "failed" => {
                        let message = value["message"]
                            .as_str()
                            .unwrap_or("VieNeu worker gặp lỗi.");
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
                    if status.success() {
                        return terminal
                            .ok_or_else(|| "Python worker không trả dữ liệu.".to_string());
                    }
                    return Err("Python worker đã kết thúc bất thường. Kiểm tra Python dependencies và log backend.".to_string());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.wait();
    terminal.ok_or_else(|| "Python worker đã đóng mà không trả kết quả.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, None, "rev-1"),
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, None, "rev-1")
        );
        assert_ne!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, None, "rev-1"),
            fingerprint(&item, "vi", "Minh Quân Pro", 1.0, false, None, "rev-1")
        );
        assert_ne!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, None, "rev-1"),
            fingerprint(&item, "vi", "Mai Anh", 1.1, false, None, "rev-1")
        );
        let mut changed = item.clone();
        changed.translated.push('!');
        assert_ne!(
            fingerprint(&item, "vi", "Mai Anh", 1.0, false, None, "rev-1"),
            fingerprint(&changed, "vi", "Mai Anh", 1.0, false, None, "rev-1")
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
        assert!((limited_tempo - 2.0).abs() < 0.001);
        assert!(still_overflows);
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
            project_id: project.id.clone(),
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
