use crate::commands::projects_root;
use crate::project_service::ProjectStore;
use crate::transcript_service::{
    save_transcript, Transcript, TranscriptSegment, TranscriptTranslationStatus,
};
use crate::video_service::resolve_video;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::Metadata;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const MODELS: &[&str] = &["tiny", "base", "small", "medium", "large-v3"];
const WORKER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);
static MODEL_CACHE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

fn asr_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // Keep Hugging Face's generated repo/blob paths short. In packaged or
    // sandboxed Windows environments, app_data_dir can already be deeply
    // nested, and the old models/faster-whisper suffix pushed blob filenames
    // past the legacy MAX_PATH boundary.
    let cache_dir = app.path().app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu model: {error}"))?
        .join("models").join("fw");
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("Không thể tạo thư mục cache faster-whisper: {error}"))?;
    Ok(cache_dir)
}

fn legacy_asr_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("models").join("faster-whisper"))
        .map_err(|error| format!("Không thể xác định cache model cũ: {error}"))
}

#[derive(Clone, Default)]
pub struct AsrJobs {
    inner: Arc<Mutex<HashMap<String, JobEntry>>>,
    deleting: Arc<Mutex<HashSet<String>>>,
}

pub fn has_active_project_job(jobs: &AsrJobs, project_id: &str) -> bool {
    jobs.inner
        .lock()
        .map(|entries| entries.values().any(|entry| entry.project_id == project_id))
        .unwrap_or(true)
}

pub fn set_project_deleting(jobs: &AsrJobs, project_id: &str) -> Result<(), String> {
    jobs.deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái project jobs.".to_string())?
        .insert(project_id.to_string());
    Ok(())
}

#[derive(Clone)]
struct JobEntry {
    project_id: String,
    canceled: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrOptions {
    #[serde(default = "default_asr_engine")]
    pub engine: String,
    pub model: String,
    pub device: String,
    pub language: String,
    #[serde(default = "default_transcript_mode")]
    pub mode: String,
    #[serde(default = "default_ocr_fps")]
    pub ocr_fps: f64,
    #[serde(default)]
    pub ocr_region: Option<crate::project_service::OcrRegion>,
    #[serde(default = "default_ocr_model")]
    pub ocr_model: String,
    #[serde(default = "default_ocr_device")]
    pub ocr_device: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrEnvironment {
    pub worker_installed: bool,
    pub model_cached: bool,
    pub cuda_available: bool,
    pub model: String,
    pub whisper_x_installed: bool,
    pub alignment_model_cached: bool,
    pub alignment_language: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrEnvironment {
    pub worker_installed: bool,
    pub ocr_installed: bool,
    pub runtime_installed: bool,
    pub model_cached: bool,
    pub engine: String,
    pub model: String,
    pub model_profile: String,
    pub model_bytes: u64,
    pub estimated_download_bytes: u64,
    pub cuda_available: bool,
    pub active_device: String,
}

fn default_asr_engine() -> String {
    "fasterWhisper".to_string()
}

fn default_transcript_mode() -> String { "speechRecognition".to_string() }
fn default_ocr_fps() -> f64 { 2.0 }
fn default_ocr_model() -> String { "tiny".to_string() }
fn default_ocr_device() -> String { "auto".to_string() }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptionEvent {
    project_id: String,
    job_id: String,
    status: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<f64>,
}

#[derive(Deserialize)]
struct WorkerEvent {
    #[serde(rename = "type")]
    kind: String,
    stage: Option<String>,
    progress: Option<f64>,
    message: Option<String>,
    code: Option<String>,
}

#[derive(Deserialize)]
struct WorkerOutput {
    language: Option<String>,
    segments: Vec<WorkerSegment>,
}

#[derive(Deserialize)]
struct WorkerSegment {
    start: f64,
    end: f64,
    text: String,
}

struct TranscriptionArtifacts {
    audio_path: PathBuf,
    worker_output_path: PathBuf,
    keep_audio: bool,
}

impl Drop for TranscriptionArtifacts {
    fn drop(&mut self) {
        if !self.keep_audio {
            let _ = fs::remove_file(&self.audio_path);
        }
        let _ = fs::remove_file(&self.worker_output_path);
    }
}

pub fn get_asr_environment(
    app: AppHandle,
    model: String,
    alignment_language: Option<String>,
) -> Result<AsrEnvironment, String> {
    validate_model(&model)?;
    let alignment_language = alignment_language.filter(|language| language != "auto");
    if let Some(language) = alignment_language.as_deref() {
        validate_alignment_language(language)?;
    }
    let root = asr_cache_dir(&app)?;
    let legacy_root = legacy_asr_cache_dir(&app)?;
    let worker = worker_root(&app)?;
    let request = serde_json::json!({ "operation": "inspect", "model": model, "cacheDir": root, "legacyCacheDir": legacy_root, "alignmentLanguage": alignment_language });
    let response = inspect_worker(&worker, &request)?;
    Ok(AsrEnvironment {
        worker_installed: response
            .get("workerInstalled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        model_cached: response
            .get("modelCached")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        cuda_available: response
            .get("cudaAvailable")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        model,
        whisper_x_installed: response
            .get("whisperXInstalled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        alignment_model_cached: response
            .get("alignmentModelCached")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        alignment_language,
    })
}

fn ocr_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app.path().app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục model OCR: {error}"))?
        .join("models");
    fs::create_dir_all(&root).map_err(|error| format!("Không thể tạo thư mục model OCR: {error}"))?;
    Ok(root)
}

pub fn get_ocr_environment(app: AppHandle, model: String) -> Result<OcrEnvironment, String> {
    validate_ocr_options(&model, "auto")?;
    let worker = ocr_worker_root(&app)?;
    let response = inspect_worker(&worker, &serde_json::json!({
        "operation": "ocr_inspect",
        "cacheDir": ocr_cache_dir(&app)?,
        "model": model,
    }))?;
    Ok(OcrEnvironment {
        worker_installed: response.get("workerInstalled").and_then(|value| value.as_bool()).unwrap_or(false),
        ocr_installed: response.get("ocrInstalled").and_then(|value| value.as_bool()).unwrap_or(false),
        runtime_installed: response.get("runtimeInstalled").and_then(|value| value.as_bool()).unwrap_or(false),
        model_cached: response.get("modelCached").and_then(|value| value.as_bool()).unwrap_or(false),
        engine: response.get("engine").and_then(|value| value.as_str()).unwrap_or("RapidOCR · ONNX Runtime").to_string(),
        model: response.get("model").and_then(|value| value.as_str()).unwrap_or("PP-OCRv6 Tiny").to_string(),
        model_profile: response.get("modelProfile").and_then(|value| value.as_str()).unwrap_or("tiny").to_string(),
        model_bytes: response.get("modelBytes").and_then(|value| value.as_u64()).unwrap_or(0),
        estimated_download_bytes: response.get("estimatedDownloadBytes").and_then(|value| value.as_u64()).unwrap_or(7 * 1024 * 1024),
        cuda_available: response.get("cudaAvailable").and_then(|value| value.as_bool()).unwrap_or(false),
        active_device: response.get("activeDevice").and_then(|value| value.as_str()).unwrap_or("cpu").to_string(),
    })
}

/// Installs the optional OCR Python runtime for a development checkout. Release
/// workers are self-contained and must ship these modules in the bundled
/// executable, so a missing release dependency is reported as a repair/update
/// issue instead of attempting to mutate the installed application.
pub fn install_ocr_runtime(app: AppHandle) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("Bản cài này thiếu runtime OCR tích hợp. Hãy cập nhật hoặc sửa chữa JC AI để khôi phục thành phần OCR.".into());
    }
    let worker = ocr_worker_root(&app)?;
    let before = inspect_worker(&worker, &serde_json::json!({
        "operation": "ocr_inspect",
        "cacheDir": ocr_cache_dir(&app)?,
    }))?;
    if !before.get("workerInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Err("Bộ xử lý AI chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.".into());
    }
    let mut command = crate::runtime_paths::development_python_command()?;
    command
        .args(["-m", "pip", "install", "--disable-pip-version-check", "--no-cache-dir", "rapidocr==3.9.2"])
        .current_dir(&worker)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8:backslashreplace")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !before.get("runtimeInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
        command.arg("onnxruntime==1.30.0");
    }
    let output = command
        .output()
        .map_err(|error| format!("Không thể khởi chạy pip để cài runtime OCR: {error}"))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr);
        let details = details.trim();
        return Err(if details.is_empty() {
            "pip không cài được RapidOCR. Kiểm tra kết nối mạng và môi trường Python development.".into()
        } else {
            format!("Không cài được runtime RapidOCR: {}", details.chars().rev().take(1800).collect::<String>().chars().rev().collect::<String>())
        });
    }
    let environment = inspect_worker(&worker, &serde_json::json!({
        "operation": "ocr_inspect",
        "cacheDir": ocr_cache_dir(&app)?,
    }))?;
    if !environment.get("ocrInstalled").and_then(|value| value.as_bool()).unwrap_or(false)
        || !environment.get("runtimeInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Err("pip đã hoàn tất nhưng Python Worker chưa thể nạp RapidOCR. Hãy bấm Kiểm tra lại hoặc xem diagnostics.".into());
    }
    Ok(())
}

pub fn install_ocr_model(app: AppHandle, model: String) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK.lock()
        .map_err(|_| "Không thể khóa thao tác model Speech Recognition.".to_string())?;
    validate_ocr_options(&model, "cpu")?;
    let worker = ocr_worker_root(&app)?;
    let cache_dir = ocr_cache_dir(&app)?;
    let environment = inspect_worker(&worker, &serde_json::json!({ "operation": "ocr_inspect", "cacheDir": cache_dir, "model": model }))?;
    if !environment.get("workerInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Err("Bộ đọc chữ chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.".into());
    }
    if !environment.get("ocrInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Err("Bộ đọc chữ chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.".into());
    }
    if !environment.get("runtimeInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Err("Bộ đọc chữ chưa đầy đủ. Hãy cập nhật hoặc cài lại JC AI.".into());
    }
    if environment.get("modelCached").and_then(|value| value.as_bool()).unwrap_or(false) {
        return Ok(());
    }
    run_ocr_install_worker(&app, &worker, &serde_json::json!({ "operation": "ocr_install", "cacheDir": cache_dir, "model": model }))
}

pub fn remove_ocr_model(app: AppHandle, model: String) -> Result<u64, String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK.lock()
        .map_err(|_| "Không thể khóa thao tác model Speech Recognition.".to_string())?;
    validate_ocr_options(&model, "cpu")?;
    let data_root = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let models_root = data_root.join("models");
    let model_root = models_root.join("rapidocr");
    let Ok(models_metadata) = fs::symlink_metadata(&models_root) else { return Ok(0) };
    if !models_metadata.is_dir() || models_metadata.file_type().is_symlink() || is_reparse_point(&models_metadata) {
        return Err("Thư mục Models không an toàn; không xóa model OCR.".into());
    }
    let Ok(root_metadata) = fs::symlink_metadata(&model_root) else { return Ok(0) };
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() || is_reparse_point(&root_metadata) {
        return Err("Thư mục model OCR không hợp lệ; không xóa dữ liệu.".into());
    }
    let canonical_root = models_root.canonicalize().map_err(|error| format!("Không xác minh được thư mục Models: {error}"))?;
    let canonical_model_root = model_root.canonicalize().map_err(|error| format!("Không xác minh được thư mục model OCR: {error}"))?;
    if canonical_model_root.parent() != Some(canonical_root.as_path()) || !canonical_model_root.starts_with(&canonical_root) {
        return Err("Đường dẫn model OCR nằm ngoài thư mục Models của ứng dụng; đã từ chối xóa.".into());
    }
    // New OCR profiles live in separate subfolders, so removing one profile
    // never removes another. Tiny models from earlier releases were stored
    // directly in rapidocr/; remove only their known top-level weight files.
    let profile_directory = canonical_model_root.join(&model);
    if let Ok(metadata) = fs::symlink_metadata(&profile_directory) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err("Thư mục model OCR không hợp lệ; không xóa dữ liệu.".into());
        }
        let canonical_profile = profile_directory.canonicalize().map_err(|error| format!("Không xác minh được thư mục model OCR: {error}"))?;
        if canonical_profile.parent() != Some(canonical_model_root.as_path()) || !canonical_profile.starts_with(&canonical_root) {
            return Err("Đường dẫn model OCR nằm ngoài thư mục quản lý; đã từ chối xóa.".into());
        }
        let bytes = safe_managed_directory_size(&canonical_profile)?;
        fs::remove_dir_all(&canonical_profile).map_err(|error| format!("Không thể xóa file model OCR: {error}"))?;
        return Ok(bytes);
    }
    if model != "tiny" { return Ok(0); }
    safe_managed_directory_size(&canonical_model_root)?;
    let legacy_names = [
        "PP-OCRv6_det_tiny.onnx",
        "PP-OCRv6_rec_tiny.onnx",
        "ch_ppocr_mobile_v2.0_cls_mobile.onnx",
        "ppocrv6_tiny_dict.txt",
        "PP-OCRv6_rec_tiny_dict.txt",
    ];
    let mut removed = 0_u64;
    for name in legacy_names {
        let file = canonical_model_root.join(name);
        let Ok(metadata) = fs::symlink_metadata(&file) else { continue };
        if metadata.is_file() && !metadata.file_type().is_symlink() && !is_reparse_point(&metadata) {
            removed = removed.saturating_add(metadata.len());
            fs::remove_file(file).map_err(|error| format!("Không thể xóa model OCR Tiny: {error}"))?;
        } else {
            return Err("File model OCR Tiny không hợp lệ; đã từ chối xóa.".into());
        }
    }
    Ok(removed)
}

fn is_reparse_point(metadata: &Metadata) -> bool {
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x0400 != 0;
    }
    #[cfg(not(windows))] { let _ = metadata; false }
}

fn safe_managed_directory_size(directory: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err("Thư mục model OCR chứa symlink/reparse point; đã từ chối xóa.".into());
    }
    if metadata.is_file() { return Ok(metadata.len()); }
    if !metadata.is_dir() { return Err("Thư mục model OCR chứa loại file không hợp lệ.".into()); }
    let mut total = 0_u64;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        total = total.saturating_add(safe_managed_directory_size(&entry.path())?);
    }
    Ok(total)
}

pub fn install_whisperx_alignment(app: AppHandle, language: String) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK
        .lock()
        .map_err(|_| "Không thể khóa thao tác model Speech Recognition.".to_string())?;
    validate_alignment_language(&language)?;
    let worker = worker_root(&app)?;
    let cache_dir = asr_cache_dir(&app)?.join("whisperx");
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("Không thể tạo thư mục model WhisperX: {error}"))?;
    let response = inspect_worker(
        &worker,
        &serde_json::json!({
            "operation": "inspect",
            "model": "small",
            "cacheDir": asr_cache_dir(&app)?,
            "alignmentLanguage": language,
        }),
    )?;
    if !response
        .get("whisperXInstalled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err("WhisperX chưa được cài. Mở Cài đặt → Nhận dạng lời thoại và cài WhisperX.".to_string());
    }
    if response
        .get("alignmentModelCached")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(());
    }
    run_install_worker(
        &worker,
        &serde_json::json!({ "operation": "whisperx_alignment_install", "language": language, "cacheDir": cache_dir }),
    )
}

pub fn install_model(app: AppHandle, model: String) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK
        .lock()
        .map_err(|_| "Không thể khóa thao tác model Speech Recognition.".to_string())?;
    validate_model(&model)?;
    let root = asr_cache_dir(&app)?;
    let legacy_root = legacy_asr_cache_dir(&app)?;
    let worker = worker_root(&app)?;
    let environment = inspect_worker(
        &worker,
        &serde_json::json!({ "operation": "inspect", "model": model, "cacheDir": root, "legacyCacheDir": legacy_root }),
    )?;
    if !environment
        .get("workerInstalled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(
            "Bộ nhận dạng lời thoại chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI."
                .to_string(),
        );
    }
    if environment
        .get("modelCached")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(());
    }
    run_install_worker(
        &worker,
        &serde_json::json!({ "operation": "asr_install", "model": model, "cacheDir": root }),
    )
}

pub fn remove_model(app: AppHandle, jobs: &AsrJobs, model: String) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK
        .lock()
        .map_err(|_| "Không thể khóa thao tác model Speech Recognition.".to_string())?;
    validate_model(&model)?;
    let active = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể kiểm tra Speech Recognition jobs.".to_string())?;
    if !active.is_empty() {
        return Err("Không thể gỡ model khi Speech Recognition đang chạy.".to_string());
    }
    drop(active);
    let root = asr_cache_dir(&app)?;
    remove_model_child(&root, &model)?;
    let legacy_root = legacy_asr_cache_dir(&app)?;
    if legacy_root.is_dir() {
        remove_model_child(&legacy_root, &model)?;
    }
    Ok(())
}

fn remove_model_child(root: &Path, model: &str) -> Result<(), String> {
    let root_meta = fs::symlink_metadata(root)
        .map_err(|error| format!("Không thể xác minh thư mục model: {error}"))?;
    if is_reparse_or_symlink(&root_meta) || !root_meta.is_dir() {
        return Err("Thư mục model không hợp lệ hoặc là symlink.".to_string());
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let candidate = root.join(model);
    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Không thể kiểm tra thư mục model: {error}")),
    };
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err("Đường dẫn model không hợp lệ; không xóa dữ liệu.".to_string());
    }
    if candidate.parent() != Some(root.as_path()) {
        return Err("Đường dẫn model vượt khỏi thư mục cache được quản lý.".to_string());
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh model cần gỡ: {error}"))?;
    if canonical.parent() != Some(root.as_path()) {
        return Err("Model cần gỡ nằm ngoài thư mục cache được quản lý.".to_string());
    }
    fs::remove_dir_all(canonical)
        .map_err(|error| format!("Không thể xóa model faster-whisper: {error}"))
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

pub fn generate_transcript(
    app: AppHandle,
    jobs: &AsrJobs,
    project_id: String,
    mut options: AsrOptions,
) -> Result<String, String> {
    let projects_root = projects_root(&app)?;
    let store = ProjectStore::new(projects_root.clone());
    let project = store.get(&project_id)?;
    let global_asr = crate::settings_service::get_settings(&app)?.settings.asr;
    options.ocr_model = global_asr.ocr_model;
    options.ocr_device = global_asr.ocr_device;
    // The project setting is authoritative. Never let a stale or hand-crafted
    // frontend request switch OCR/ASR modes or alter the persisted crop.
    options.mode = match project.transcript_settings.mode {
        crate::project_service::TranscriptMode::Ocr => "ocr".to_string(),
        crate::project_service::TranscriptMode::SpeechRecognition => "speechRecognition".to_string(),
    };
    options.ocr_fps = project.transcript_settings.ocr_fps;
    options.ocr_region = project.transcript_settings.ocr_region.clone();
    validate_options(&options)?;
    if options.mode == "speechRecognition" && project.source.audio_streams.is_empty() {
        return Err("Video không có audio stream để nhận dạng giọng nói. Hãy chuyển chế độ dự án sang OCR để đọc subtitle trên hình.".to_string());
    }
    if options.mode == "ocr" && options.ocr_region.is_none() {
        return Err("Chưa chọn vùng subtitle để OCR. Hãy kéo chọn vùng trong khung video ở panel Transcript.".to_string());
    }
    resolve_video(&store, &project_id)?;
    let worker = if options.mode == "ocr" { ocr_worker_root(&app)? } else { worker_root(&app)? };
    if !worker.is_dir() {
        return Err("Không tìm thấy thư mục Python worker trong checkout hiện tại.".to_string());
    }
    let cache_dir = asr_cache_dir(&app)?;
    let mut cuda_available = false;
    if options.mode == "ocr" {
        let environment = inspect_worker(&worker, &serde_json::json!({
            "operation": "ocr_inspect",
            "cacheDir": app.path().app_data_dir().map_err(|error| error.to_string())?.join("models"),
            "model": options.ocr_model,
        }))?;
        if !environment.get("workerInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err("Bộ xử lý AI chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.".into());
        }
        if !environment.get("ocrInstalled").and_then(|value| value.as_bool()).unwrap_or(false)
            || !environment.get("runtimeInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err("Bộ đọc chữ chưa được cài. Mở Cài đặt → Nhận dạng lời thoại.".into());
        }
        if !environment.get("modelCached").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err(format!("Mô hình đọc chữ {} chưa được cài. Mở Cài đặt → Nhận dạng lời thoại để tải.", options.ocr_model));
        }
        if options.ocr_device == "cuda" && !environment.get("cudaAvailable").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err("Tăng tốc NVIDIA chưa sẵn sàng. Chọn Tự động hoặc CPU, hoặc cài bản NVIDIA trong Cài đặt.".into());
        }
    } else {
        let legacy_cache_dir = legacy_asr_cache_dir(&app)?;
        let alignment_language = if options.engine == "whisperX" { Some(options.language.clone()) } else { None };
        let environment = inspect_worker(&worker, &serde_json::json!({ "operation": "inspect", "model": options.model, "cacheDir": cache_dir, "legacyCacheDir": legacy_cache_dir, "alignmentLanguage": alignment_language }))?;
        cuda_available = environment.get("cudaAvailable").and_then(|value| value.as_bool()).unwrap_or(false);
        if !environment.get("workerInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err("Bộ nhận dạng lời thoại chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.".to_string());
        }
        if options.engine == "whisperX" && !environment.get("whisperXInstalled").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err("WhisperX chưa được cài. Mở Cài đặt → Nhận dạng lời thoại và cài WhisperX.".to_string());
        }
        if options.engine == "whisperX" && !environment.get("alignmentModelCached").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err(format!("WhisperX chưa có mô hình căn thời gian cho ngôn ngữ {}. Mở Cài đặt → Nhận dạng lời thoại để tải.", options.language));
        }
        if options.device == "cuda" && !cuda_available {
            return Err("Tăng tốc NVIDIA chưa sẵn sàng cho faster-whisper. Chọn Tự động hoặc CPU trong Cài đặt.".to_string());
        }
        if !environment.get("modelCached").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Err(format!("Mô hình faster-whisper '{}' chưa được cài. Mở Cài đặt → Nhận dạng lời thoại để tải.", options.model));
        }
    }

    let job_id = Uuid::new_v4().to_string();
    let canceled = Arc::new(AtomicBool::new(false));
    let deleting = jobs
        .deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái project jobs.".to_string())?;
    if deleting.contains(&project_id) {
        return Err("Dự án đang được xóa; không thể bắt đầu transcription.".to_string());
    }
    let mut entries = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái Speech Recognition.".to_string())?;
    if !entries.is_empty() {
        return Err(
            "Một tác vụ Speech Recognition khác đang chạy. Hãy đợi tác vụ đó hoàn tất.".to_string(),
        );
    }
    entries.insert(
        job_id.clone(),
        JobEntry {
            project_id: project_id.clone(),
            canceled: canceled.clone(),
        },
    );
    drop(entries);
    drop(deleting);
    let use_gpu = options.mode == "speechRecognition" && (options.device == "cuda" || options.device == "auto" && cuda_available);
    let mut resources = vec!["python-worker", "disk"];
    resources.push(if use_gpu { "gpu" } else { "cpu" });
    crate::job_service::register_job(
        &app,
        job_id.clone(),
        project_id.clone(),
        project.name.clone(),
        "transcribe",
        &resources,
        serde_json::to_value(&options).ok(),
    );
    let registry = jobs.inner.clone();
    let app_handle = app.clone();
    let worker_job_id = job_id.clone();
    thread::spawn(move || {
        let result = (|| {
            let _resource_permit = if use_gpu {
                Some(crate::resource_scheduler::wait_for_ai_gpu(&canceled)?)
            } else {
                Some(crate::resource_scheduler::wait_for_cpu(&canceled)?)
            };
            run_transcription(
                &app_handle,
                &projects_root,
                &worker,
                &cache_dir,
                &project_id,
                &worker_job_id,
                &options,
                &canceled,
            )
        })();
        if let Err(error) = result {
            let status = if canceled.load(Ordering::SeqCst) {
                "canceled"
            } else {
                "failed"
            };
            emit_status(
                &app_handle,
                &worker_job_id,
                &project_id,
                status,
                &error,
                None,
                None,
            );
        }
        if let Ok(mut entries) = registry.lock() {
            entries.remove(&worker_job_id);
        }
    });
    Ok(job_id)
}

pub fn cancel_transcription(
    jobs: tauri::State<'_, AsrJobs>,
    job_id: String,
) -> Result<bool, String> {
    let entries = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái Speech Recognition.".to_string())?;
    let Some(job) = entries.get(&job_id) else {
        return Ok(false);
    };
    job.canceled.store(true, Ordering::SeqCst);
    Ok(true)
}

pub fn cancel_project_jobs_and_wait(jobs: &AsrJobs, project_id: &str) -> Result<(), String> {
    jobs.deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái project jobs.".to_string())?
        .insert(project_id.to_string());
    let entries = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái Speech Recognition.".to_string())?;
    for entry in entries
        .values()
        .filter(|entry| entry.project_id == project_id)
    {
        entry.canceled.store(true, Ordering::SeqCst);
    }
    drop(entries);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let active = jobs
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái Speech Recognition.".to_string())?
            .values()
            .any(|entry| entry.project_id == project_id);
        if !active {
            return Ok(());
        }
        if Instant::now() >= deadline {
            clear_project_deleting(jobs, project_id);
            return Err(
                "Đang dừng Speech Recognition. Hãy thử xóa dự án lại sau giây lát.".to_string(),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn clear_project_deleting(jobs: &AsrJobs, project_id: &str) {
    if let Ok(mut deleting) = jobs.deleting.lock() {
        deleting.remove(project_id);
    }
}

fn run_transcription(
    app: &AppHandle,
    projects_root: &Path,
    worker: &Path,
    cache_dir: &Path,
    project_id: &str,
    job_id: &str,
    options: &AsrOptions,
    canceled: &AtomicBool,
) -> Result<(), String> {
    let store = ProjectStore::new(projects_root.to_path_buf());
    let project = store.get(project_id)?;
    if options.mode == "ocr" {
        return run_ocr_transcription(app, projects_root, worker, project_id, job_id, options, canceled);
    }
    let source_checksum = project.source.checksum_sha256.clone();
    let video_path = resolve_video(&store, project_id)?;
    let project_dir = store.project_directory(project_id)?;
    let extracted_dir = project_dir.join("audio").join("extracted");
    fs::create_dir_all(&extracted_dir)
        .map_err(|error| format!("Không thể tạo thư mục audio trích xuất: {error}"))?;
    let audio_path = extracted_dir.join(format!("transcription-{job_id}.wav"));
    let output_path = project_dir
        .join("cache")
        .join(format!("transcription-{job_id}.json"));
    let mut artifacts = TranscriptionArtifacts {
        audio_path: audio_path.clone(),
        worker_output_path: output_path.clone(),
        keep_audio: false,
    };

    emit_status(
        app,
        job_id,
        project_id,
        "progress",
        "Đang trích xuất audio mono 16 kHz…",
        Some("extract-audio"),
        None,
    );
    if canceled.load(Ordering::SeqCst) {
        return Err("Đã hủy transcription.".to_string());
    }
    let mut ffmpeg = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&video_path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
        ])
        .arg("-ac")
        .arg("1")
        .args(["-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&audio_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Không thể khởi chạy FFmpeg để trích xuất audio: {error}"))?;
    let ffmpeg_status = wait_killable(&mut ffmpeg, canceled)?;
    if !ffmpeg_status.success() {
        let _ = fs::remove_file(&audio_path);
        return Err(
            "FFmpeg không thể trích xuất audio. Kiểm tra file video và audio stream.".to_string(),
        );
    }
    if !audio_path.is_file()
        || fs::metadata(&audio_path)
            .map(|meta| meta.len())
            .unwrap_or(0)
            < 44
    {
        let _ = fs::remove_file(&audio_path);
        return Err("Video không tạo được audio có thể nhận dạng.".to_string());
    }
    if canceled.load(Ordering::SeqCst) {
        let _ = fs::remove_file(&audio_path);
        return Err("Đã hủy transcription.".to_string());
    }

    emit_status(
        app,
        job_id,
        project_id,
        "progress",
        if options.engine == "whisperX" { "Đang khởi tạo faster-whisper và WhisperX…" } else { "Đang khởi tạo model faster-whisper…" },
        Some("model-loading"),
        None,
    );
    let request = serde_json::json!({
        "operation": "transcribe",
        "engine": options.engine,
        "model": options.model,
        "device": options.device,
        "language": if options.language == "auto" { serde_json::Value::Null } else { serde_json::Value::String(options.language.clone()) },
        "audioPath": audio_path,
        "outputPath": output_path,
        "cacheDir": cache_dir,
        "whisperxCacheDir": asr_cache_dir(app)?.join("whisperx"),
    });
    let worker_result = run_worker_request(app, worker, &request, project_id, job_id, canceled);
    if let Err(error) = worker_result {
        let _ = fs::remove_file(&audio_path);
        let _ = fs::remove_file(&output_path);
        return Err(error);
    }

    let worker_output: WorkerOutput = fs::read(&output_path)
        .map_err(|error| format!("Worker không tạo được transcript: {error}"))
        .and_then(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("Transcript worker không hợp lệ: {error}"))
        })?;
    let latest = store
        .get(project_id)
        .map_err(|_| "Dự án đã bị xóa trong lúc transcription chạy.".to_string())?;
    if latest.source.checksum_sha256 != source_checksum {
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&audio_path);
        return Err(
            "Video nguồn đã thay đổi trong lúc transcription chạy; kết quả đã bị bỏ.".to_string(),
        );
    }
    let segments = worker_output
        .segments
        .into_iter()
        .map(|segment| TranscriptSegment {
            id: Uuid::new_v4().to_string(),
            start: segment.start,
            end: segment.end,
            source: segment.text,
            translated: String::new(),
            source_revision: 0,
            translated_revision: None,
            translation_meta: None,
            speaker_id: None,
            source_type: crate::project_service::TranscriptSourceType::Asr,
            translation_status: TranscriptTranslationStatus::Pending,
            tts: None,
        })
        .collect();
    let transcript = Transcript {
        schema_version: 1,
        project_id: project_id.to_string(),
        source_type: crate::project_service::TranscriptSourceType::Asr,
        source_track: None,
        language: worker_output.language,
        video_checksum: source_checksum,
        segments,
    };
    if let Err(error) = save_transcript(&store, project_id, transcript) {
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&audio_path);
        return Err(error);
    }
    let _ = fs::remove_file(&output_path);
    emit_status(
        app,
        job_id,
        project_id,
        "completed",
        "Transcript đã sẵn sàng.",
        Some("completed"),
        Some(1.0),
    );
    artifacts.keep_audio = true;
    Ok(())
}

struct OcrArtifacts { root: PathBuf }
impl Drop for OcrArtifacts {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}

fn run_ocr_transcription(
    app: &AppHandle,
    projects_root: &Path,
    worker: &Path,
    project_id: &str,
    job_id: &str,
    options: &AsrOptions,
    canceled: &AtomicBool,
) -> Result<(), String> {
    let store = ProjectStore::new(projects_root.to_path_buf());
    let project = store.get(project_id)?;
    let source_checksum = project.source.checksum_sha256.clone();
    let video_path = resolve_video(&store, project_id)?;
    let region = options.ocr_region.as_ref().ok_or_else(|| "Chưa chọn vùng OCR trong cài đặt dự án.".to_string())?;
    let video_width = project.source.width.ok_or_else(|| "Không đọc được chiều rộng video cho vùng OCR.".to_string())?;
    let video_height = project.source.height.ok_or_else(|| "Không đọc được chiều cao video cho vùng OCR.".to_string())?;
    let x = (region.x * video_width as f64).round().clamp(0.0, video_width as f64) as u32;
    let y = (region.y * video_height as f64).round().clamp(0.0, video_height as f64) as u32;
    let width = (region.width * video_width as f64).round().clamp(0.0, (video_width - x) as f64) as u32;
    let height = (region.height * video_height as f64).round().clamp(0.0, (video_height - y) as f64) as u32;
    if width < 12 || height < 12 {
        return Err("Vùng OCR quá nhỏ để đọc. Hãy chọn lại vùng rộng hơn trên video.".into());
    }
    if ![1.0, 2.0, 4.0].contains(&options.ocr_fps) {
        return Err("Tốc độ lấy mẫu OCR không hợp lệ.".into());
    }
    let project_dir = store.project_directory(project_id)?;
    let temporary_root = project_dir.join("cache").join("ocr").join(job_id);
    let artifacts = OcrArtifacts { root: temporary_root.clone() };
    let frames_dir = temporary_root.join("frames");
    fs::create_dir_all(&frames_dir).map_err(|error| format!("Không thể tạo cache frame OCR: {error}"))?;
    let output_path = temporary_root.join("transcript.json");
    emit_status(app, job_id, project_id, "progress", "Đang trích xuất frame trong vùng subtitle…", Some("ocr-extract"), Some(0.0));
    if canceled.load(Ordering::SeqCst) { return Err("Đã hủy transcription.".into()); }
    let filter = format!("crop={width}:{height}:{x}:{y},fps={}", options.ocr_fps as u8);
    let mut ffmpeg = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&video_path)
        .args(["-vf", &filter, "-vsync", "0", "-q:v", "4", "-start_number", "1"])
        .arg(frames_dir.join("frame_%08d.jpg"))
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|error| format!("Không thể khởi chạy FFmpeg để lấy frame OCR: {error}"))?;
    let status = wait_killable(&mut ffmpeg, canceled)?;
    if !status.success() { return Err("FFmpeg không trích xuất được frame trong vùng OCR. Kiểm tra video và vùng đã chọn.".into()); }
    let request = serde_json::json!({
        "operation": "ocr_transcribe",
        "framesDir": frames_dir,
        "outputPath": output_path,
        "cacheDir": ocr_cache_dir(app)?,
        "fps": options.ocr_fps,
        "model": options.ocr_model,
        "device": options.ocr_device,
        "language": serde_json::Value::Null,
    });
    emit_status(app, job_id, project_id, "progress", "Đang nhận dạng chữ trong các frame…", Some("ocr-recognition"), Some(0.0));
    run_worker_request(app, worker, &request, project_id, job_id, canceled)?;
    let worker_output: WorkerOutput = fs::read(&output_path)
        .map_err(|error| format!("OCR worker không tạo được transcript: {error}"))
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| format!("Transcript OCR không hợp lệ: {error}")))?;
    if worker_output.segments.is_empty() {
        return Err("OCR không nhận dạng được subtitle trong vùng đã chọn. Hãy kiểm tra crop hoặc tăng tốc độ lấy mẫu lên 4 frame/s.".into());
    }
    let latest = store.get(project_id).map_err(|_| "Dự án đã bị xóa trong lúc OCR chạy.".to_string())?;
    if latest.source.checksum_sha256 != source_checksum {
        return Err("Video nguồn đã thay đổi trong lúc OCR chạy; kết quả đã bị bỏ.".into());
    }
    let duration = project.source.duration.unwrap_or(f64::MAX);
    let segments = worker_output.segments.into_iter().filter_map(|segment| {
        let start = segment.start.max(0.0).min(duration);
        let end = segment.end.max(start).min(duration);
        (end > start && !segment.text.trim().is_empty()).then(|| TranscriptSegment {
            id: Uuid::new_v4().to_string(), start, end, source: segment.text.trim().to_string(),
            translated: String::new(), source_revision: 0, translated_revision: None, translation_meta: None,
            speaker_id: None, source_type: crate::project_service::TranscriptSourceType::Ocr,
            translation_status: TranscriptTranslationStatus::Pending, tts: None,
        })
    }).collect::<Vec<_>>();
    if segments.is_empty() { return Err("OCR trả về timestamp không hợp lệ cho video này.".into()); }
    let transcript = Transcript {
        schema_version: 1, project_id: project_id.to_string(),
        source_type: crate::project_service::TranscriptSourceType::Ocr,
        source_track: None, language: worker_output.language, video_checksum: source_checksum, segments,
    };
    save_transcript(&store, project_id, transcript)?;
    emit_status(app, job_id, project_id, "completed", "Đã tạo transcript từ subtitle trên hình bằng OCR.", Some("completed"), Some(1.0));
    drop(artifacts);
    Ok(())
}

fn run_worker_request(
    app: &AppHandle,
    worker: &Path,
    request: &serde_json::Value,
    project_id: &str,
    job_id: &str,
    canceled: &AtomicBool,
) -> Result<(), String> {
    let mut child = python_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Không thể bắt đầu nhận dạng lời thoại: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let mut line = serde_json::to_vec(request)
            .map_err(|error| format!("Không thể mã hóa lệnh worker: {error}"))?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .map_err(|error| format!("Không thể gửi lệnh tới Python worker: {error}"))?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Python worker không mở stdout.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Python worker không mở stderr.".to_string())?;
    let (sender, receiver) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_thread = thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let mut completed = false;
    let mut worker_error: Option<String> = None;
    let mut exit_status = None;
    loop {
        if canceled.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Đã hủy transcription.".to_string());
        }
        if started.elapsed() > WORKER_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "Speech Recognition vượt quá giới hạn thời gian 12 giờ và đã dừng.".to_string(),
            );
        }
        match receiver.recv_timeout(Duration::from_millis(120)) {
            Ok(line) => match serde_json::from_str::<WorkerEvent>(&line) {
                Ok(event) if event.kind == "progress" => emit_status(
                    app,
                    job_id,
                    project_id,
                    "progress",
                    event
                        .message
                        .as_deref()
                        .unwrap_or("Đang nhận dạng giọng nói…"),
                    event.stage.as_deref(),
                    event.progress,
                ),
                Ok(event) if event.kind == "completed" => completed = true,
                Ok(event) if event.kind == "failed" => {
                    worker_error = Some(format!(
                        "{}{}",
                        event
                            .message
                            .unwrap_or_else(|| "Không thể xử lý lời thoại.".to_string()),
                        event
                            .code
                            .map(|code| format!(" ({code})"))
                            .unwrap_or_default()
                    ));
                }
                Ok(_) => {}
                Err(_) => eprintln!("Ignoring invalid Python worker JSONL output"),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("Không thể kiểm tra Python worker: {error}"))?
                {
                    exit_status = Some(status);
                    break;
                }
            }
        }
        if exit_status.is_none() {
            exit_status = child
                .try_wait()
                .map_err(|error| format!("Không thể kiểm tra Python worker: {error}"))?;
            if exit_status.is_some() {
                while let Ok(line) = receiver.try_recv() {
                    if let Ok(event) = serde_json::from_str::<WorkerEvent>(&line) {
                        if event.kind == "failed" {
                            worker_error = Some(
                                event
                                    .message
                                    .unwrap_or_else(|| "Không thể xử lý lời thoại.".to_string()),
                            );
                        } else if event.kind == "completed" {
                            completed = true;
                        }
                    }
                }
                break;
            }
        }
    }
    let stderr = stderr_thread.join().unwrap_or_default();
    if let Some(error) = worker_error {
        return Err(error);
    }
    if !exit_status.is_some_and(|status| status.success()) {
        log::error!("Speech recognition process stopped: {}", stderr.trim());
        return Err("Nhận dạng lời thoại đã dừng bất ngờ. Mở Cài đặt → Nhận dạng lời thoại để kiểm tra thành phần đã cài.".to_string());
    }
    if !completed {
        return Err("Nhận dạng lời thoại đã dừng trước khi hoàn tất bản chép lời.".to_string());
    }
    Ok(())
}

fn inspect_worker(worker: &Path, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut child = python_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Không thể kiểm tra bộ nhận dạng lời thoại: {error}"))?;
    let mut input = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    input.push(b'\n');
    child
        .stdin
        .take()
        .ok_or_else(|| "Python worker không mở stdin.".to_string())?
        .write_all(&input)
        .map_err(|error| format!("Không thể gửi lệnh kiểm tra Python worker: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Không thể đọc trạng thái Python worker: {error}"))?;
    let response = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .last()
        .ok_or_else(|| {
            log::error!("Speech recognition inspection returned no data: {}", String::from_utf8_lossy(&output.stderr));
            "Bộ nhận dạng lời thoại không phản hồi. Hãy cập nhật hoặc cài lại JC AI.".to_string()
        })?;
    let envelope: serde_json::Value = serde_json::from_slice(response)
        .map_err(|error| format!("Không đọc được trạng thái bộ nhận dạng lời thoại: {error}"))?;
    if envelope.get("type").and_then(|value| value.as_str()) != Some("capabilities") {
        return Err(envelope
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Không thể kiểm tra mô hình nhận dạng.")
            .to_string());
    }
    Ok(envelope)
}

fn run_install_worker(worker: &Path, request: &serde_json::Value) -> Result<(), String> {
    let mut child = python_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Không thể khởi chạy Python worker để cài model: {error}"))?;
    let mut input = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    input.push(b'\n');
    child
        .stdin
        .take()
        .ok_or_else(|| "Python worker không mở stdin.".to_string())?
        .write_all(&input)
        .map_err(|error| format!("Không thể gửi yêu cầu cài model: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Không thể chờ Python worker cài model: {error}"))?;
    let response = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .last()
        .ok_or_else(|| {
            let details = String::from_utf8_lossy(&output.stderr);
            format!(
                "Python worker không phản hồi khi cài model: {}",
                details.trim()
            )
        })?;
    let envelope: serde_json::Value = serde_json::from_slice(response)
        .map_err(|error| format!("Phản hồi cài model không hợp lệ: {error}"))?;
    if !output.status.success()
        || envelope.get("type").and_then(|value| value.as_str()) != Some("completed")
    {
        return Err(envelope
            .get("message")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                let details = String::from_utf8_lossy(&output.stderr);
                let operation = request.get("operation").and_then(|value| value.as_str()).unwrap_or("");
                let component = if operation.starts_with("ocr_") { "model OCR RapidOCR" } else if operation == "whisperx_alignment_install" { "WhisperX aligner" } else { "model faster-whisper" };
                if details.trim().is_empty() {
                    format!("Không thể tải {component}.")
                } else {
                    format!("Không thể tải {component}: {}", details.trim())
                }
            }));
    }
    Ok(())
}

fn run_ocr_install_worker(
    app: &AppHandle,
    worker: &Path,
    request: &serde_json::Value,
) -> Result<(), String> {
    let mut child = python_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Không thể khởi chạy Python worker để tải model OCR: {error}"))?;
    let mut input = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    input.push(b'\n');
    child.stdin.take()
        .ok_or_else(|| "Python worker không mở stdin.".to_string())?
        .write_all(&input)
        .map_err(|error| format!("Không thể gửi lệnh tải model OCR: {error}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "Python worker không mở stdout.".to_string())?;
    let mut stderr = child.stderr.take().ok_or_else(|| "Python worker không mở stderr.".to_string())?;
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let mut final_message = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|error| format!("Không thể đọc tiến trình tải model OCR: {error}"))?;
        if line.trim().is_empty() { continue; }
        let envelope: serde_json::Value = serde_json::from_str(&line)
            .map_err(|error| format!("Python worker trả tiến trình OCR không hợp lệ: {error}"))?;
        match envelope.get("type").and_then(|value| value.as_str()) {
            Some("progress") => { let _ = app.emit("ocr-model-progress", &envelope); }
            Some("completed") | Some("failed") => final_message = Some(envelope),
            _ => {}
        }
    }
    let status = child.wait().map_err(|error| format!("Không thể chờ Python worker tải model OCR: {error}"))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    let response = final_message.unwrap_or(serde_json::Value::Null);
    if status.success() && response.get("type").and_then(|value| value.as_str()) == Some("completed") {
        return Ok(());
    }
    if let Some(message) = response.get("message").and_then(|value| value.as_str()) {
        return Err(message.to_string());
    }
    let details = String::from_utf8_lossy(&stderr);
    if details.trim().is_empty() {
        Err("Python worker không hoàn tất tải model OCR.".into())
    } else {
        let trimmed = details.trim();
        let tail = trimmed.chars().rev().take(1800).collect::<String>().chars().rev().collect::<String>();
        Err(format!("Python worker không hoàn tất tải model OCR: {tail}"))
    }
}

fn python_command(worker: &Path) -> Command {
    crate::runtime_paths::worker_command(worker)
}

fn worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    if crate::optional_runtime::feature_installed("whisperx") {
        crate::runtime_paths::ai_worker_dir_for("whisperx")
    } else {
        crate::runtime_paths::ai_worker_dir()
    }
}

fn ocr_worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::ai_worker_dir()
}

fn wait_killable(
    child: &mut Child,
    canceled: &AtomicBool,
) -> Result<std::process::ExitStatus, String> {
    loop {
        if canceled.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Đã hủy transcription.".to_string());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Không thể kiểm tra FFmpeg: {error}"))?
        {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn emit_status(
    app: &AppHandle,
    job_id: &str,
    project_id: &str,
    status: &str,
    message: &str,
    stage: Option<&str>,
    progress: Option<f64>,
) {
    let event = TranscriptionEvent {
        project_id: project_id.to_string(),
        job_id: job_id.to_string(),
        status: status.to_string(),
        message: message.to_string(),
        stage: stage.map(str::to_string),
        progress,
    };
    crate::job_service::sync_app_event(app, "transcription", &event);
    let _ = app.emit("transcription-progress", event);
}

fn validate_model(model: &str) -> Result<(), String> {
    if MODELS.contains(&model) {
        Ok(())
    } else {
        Err("Model Speech Recognition không hợp lệ.".to_string())
    }
}

fn validate_alignment_language(language: &str) -> Result<(), String> {
    if ["en", "vi", "ja", "ko", "zh", "fr", "de", "es"].contains(&language) {
        Ok(())
    } else {
        Err("WhisperX alignment hiện hỗ trợ chọn một trong các ngôn ngữ en, vi, ja, ko, zh, fr, de, es.".to_string())
    }
}

fn validate_options(options: &AsrOptions) -> Result<(), String> {
    if !["speechRecognition", "ocr"].contains(&options.mode.as_str()) {
        return Err("Phương thức lấy transcript của dự án không hợp lệ.".into());
    }
    if ![1.0, 2.0, 4.0].contains(&options.ocr_fps) {
        return Err("Tốc độ lấy mẫu OCR chỉ hỗ trợ 1, 2 hoặc 4 frame/s.".into());
    }
    validate_ocr_options(&options.ocr_model, &options.ocr_device)?;
    if let Some(region) = options.ocr_region.as_ref() {
        if [region.x, region.y, region.width, region.height].iter().any(|value| !value.is_finite())
            || region.x < 0.0 || region.y < 0.0 || region.width <= 0.0 || region.height <= 0.0
            || region.x + region.width > 1.000001 || region.y + region.height > 1.000001
        { return Err("Vùng OCR của dự án không hợp lệ.".into()); }
    }
    if !["fasterWhisper", "whisperX"].contains(&options.engine.as_str()) {
        return Err("Engine Speech Recognition không hợp lệ.".to_string());
    }
    validate_model(&options.model)?;
    if !["auto", "cpu", "cuda"].contains(&options.device.as_str()) {
        return Err("Device Speech Recognition không hợp lệ.".to_string());
    }
    if options.language != "auto"
        && (options.language.len() < 2
            || options.language.len() > 16
            || !options
                .language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    {
        return Err("Mã ngôn ngữ Speech Recognition không hợp lệ.".to_string());
    }
    if options.engine == "whisperX" {
        if options.language == "auto" {
            return Err("WhisperX cần chọn ngôn ngữ nguồn cụ thể để cài đúng aligner. Hãy chọn ngôn ngữ trong Cài đặt → Speech Recognition.".to_string());
        }
        validate_alignment_language(&options.language)?;
    }
    Ok(())
}

fn validate_ocr_options(model: &str, device: &str) -> Result<(), String> {
    if !["tiny", "small", "medium"].contains(&model) {
        return Err("Model OCR phải là tiny, small hoặc medium.".into());
    }
    if !["auto", "cpu", "cuda"].contains(&device) {
        return Err("Thiết bị OCR phải là Auto, CPU hoặc CUDA.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validates_asr_model_device_and_language_options() {
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "auto".into(),
            language: "auto".into(), mode: "speechRecognition".into(), ocr_fps: 2.0, ocr_region: None, ocr_model: "tiny".into(), ocr_device: "auto".into()
        })
        .is_ok());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "cuda".into(),
            language: "vi".into(), mode: "speechRecognition".into(), ocr_fps: 2.0, ocr_region: None, ocr_model: "tiny".into(), ocr_device: "auto".into()
        })
        .is_ok());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "unknown".into(),
            device: "auto".into(),
            language: "auto".into(), mode: "speechRecognition".into(), ocr_fps: 2.0, ocr_region: None, ocr_model: "tiny".into(), ocr_device: "auto".into()
        })
        .is_err());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "shell".into(),
            language: "auto".into(), mode: "speechRecognition".into(), ocr_fps: 2.0, ocr_region: None, ocr_model: "tiny".into(), ocr_device: "auto".into()
        })
        .is_err());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "auto".into(),
            language: "../bad".into(), mode: "speechRecognition".into(), ocr_fps: 2.0, ocr_region: None, ocr_model: "tiny".into(), ocr_device: "auto".into()
        })
        .is_err());
    }

    #[test]
    fn removing_a_model_deletes_only_the_managed_model_child() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("models");
        let model = root.join("small");
        let outside = temp.path().join("keep.txt");
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join("model.bin"), b"fixture").unwrap();
        fs::write(&outside, b"preserve").unwrap();

        remove_model_child(&root, "small").unwrap();

        assert!(!model.exists());
        assert_eq!(fs::read(outside).unwrap(), b"preserve");
    }

    #[test]
    fn supported_model_validation_rejects_path_traversal() {
        let temp = tempdir().unwrap();
        let outside = temp.path().join("keep.txt");
        fs::write(&outside, b"preserve").unwrap();
        assert!(validate_model("../keep.txt").is_err());
        assert_eq!(fs::read(outside).unwrap(), b"preserve");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_remove_symlinked_model_directory() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("models");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"preserve").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("small")).unwrap();

        assert!(remove_model_child(&root, "small").is_err());
        assert!(outside.join("keep.txt").is_file());
    }
}
