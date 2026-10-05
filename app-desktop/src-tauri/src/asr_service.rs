use crate::worker_manager::WorkerCommandExt;
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
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const MODELS: &[&str] = &["tiny", "base", "small", "medium", "large-v3"];
const WORKER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);
static MODEL_CACHE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

fn asr_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // Keep Hugging Face's generated repo/blob paths short. In packaged or
    // sandboxed Windows environments, app_data_dir can already be deeply
    // nested, and the old models/faster-whisper suffix pushed blob filenames
    // past the legacy MAX_PATH boundary.
    let cache_dir = crate::runtime_paths::data_dir(app)
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu model: {error}"))?
        .join("models").join("fw");
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("Không thể tạo thư mục cache faster-whisper: {error}"))?;
    Ok(cache_dir)
}

fn legacy_asr_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::data_dir(app)
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
    #[serde(default)] pub intel_runtime_installed: bool,
    #[serde(default)] pub intel_gpu_available: bool,
    #[serde(default)] pub intel_model_supported: bool,
    #[serde(default)] pub intel_model_cached: bool,
    #[serde(default)] pub ctranslate_model_cached: bool,
    #[serde(default)] pub intel_estimated_download_bytes: u64,
    #[serde(default)] pub resolved_backend: Option<String>,
    #[serde(default)] pub resolved_engine: Option<String>,
    #[serde(default)] pub resolved_label: Option<String>,
    #[serde(default)] pub selection_error: Option<String>,
    #[serde(default)] pub backends: std::collections::BTreeMap<String, bool>,
    #[serde(default)] pub word_timestamps: bool,
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
    pub onnx_runtime_version: Option<String>,
    pub onnx_runtime_device: Option<String>,
    pub execution_providers: Vec<String>,
    pub runtime_error: Option<String>,
    #[serde(default)]
    pub backends: std::collections::BTreeMap<String, OcrBackendCapability>,
    #[serde(default)]
    pub recommended_backend: String,
    #[serde(default)]
    pub resolved_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OcrBackendCapability {
    pub backend: String,
    pub available: bool,
    pub device: Option<String>,
    pub label: String,
    pub message: Option<String>,
    pub installed: bool,
    pub version: Option<String>,
}

impl OcrEnvironment {
    pub fn backend_available(&self, backend: &str) -> bool {
        self.backends.get(backend).is_some_and(|capability| capability.available)
    }
    pub fn uses_gpu(&self, requested: &str) -> bool {
        matches!(requested, "cuda" | "openvino")
            || requested == "auto" && matches!(self.recommended_backend.as_str(), "cuda" | "openvino")
    }
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
    engine: Option<String>,
    device: Option<String>,
) -> Result<AsrEnvironment, String> {
    validate_model(&model)?;
    let alignment_language = alignment_language.filter(|language| language != "auto");
    if let Some(language) = alignment_language.as_deref() {
        validate_alignment_language(language)?;
    }
    let root = asr_cache_dir(&app)?;
    let legacy_root = legacy_asr_cache_dir(&app)?;
    let worker = worker_root(&app)?;
    let request = serde_json::json!({ "operation": "inspect", "model": model, "cacheDir": root, "legacyCacheDir": legacy_root, "alignmentLanguage": alignment_language, "engine": engine.unwrap_or_else(default_asr_engine), "device": device.unwrap_or_else(|| "cpu".into()) });
    let response = inspect_worker(&worker, &request)?;
    let mut response = response;
    if request["device"] == "openvino" || request["engine"] == "openVinoWhisper" {
        let mut core_request = request.clone();
        core_request["device"] = serde_json::json!("cpu");
        core_request["engine"] = serde_json::json!("fasterWhisper");
        if let Ok(core) = inspect_worker(&worker, &core_request) {
            merge_core_asr_capabilities(&mut response, &core);
        }
    }
    // Separate packs expose only their own backend. Merge capability reports,
    // never import Intel/WhisperX into the other runtime to inspect it.
    if crate::optional_runtime::feature_installed("whisper_openvino") {
        let intel_root = crate::runtime_paths::ai_worker_dir_for("whisper_openvino")?;
        if let Ok(extra) = inspect_worker(&intel_root, &serde_json::json!({
            "operation":"inspect", "model":model, "cacheDir":root, "device":"openvino", "engine":"openVinoWhisper"
        })) {
            for (key, value) in extra.as_object().into_iter().flatten() {
                if key.starts_with("intel") { response[key] = value.clone(); }
            }
            response["backends"]["openvino"] = extra["backends"]["openvino"].clone();
        }
    }
    response["model"] = serde_json::json!(model);
    serde_json::from_value(response).map_err(|error| format!("Trạng thái nhận dạng không hợp lệ: {error}"))
}

fn merge_core_asr_capabilities(selected: &mut serde_json::Value, core: &serde_json::Value) {
    for key in ["cudaAvailable", "whisperXInstalled", "alignmentModelCached",
        "alignmentLanguage", "ctranslateModelCached"] {
        if let Some(value) = core.get(key) { selected[key] = value.clone(); }
    }
    for key in ["cpu", "cuda"] {
        if let Some(value) = core.get("backends").and_then(|v| v.get(key)) {
            selected["backends"][key] = value.clone();
        }
    }
}

fn ocr_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let root = crate::runtime_paths::data_dir(app)
        .map_err(|error| format!("Không thể xác định thư mục model OCR: {error}"))?
        .join("models");
    fs::create_dir_all(&root).map_err(|error| format!("Không thể tạo thư mục model OCR: {error}"))?;
    Ok(root)
}

pub fn get_ocr_environment(app: AppHandle, model: String) -> Result<OcrEnvironment, String> {
    validate_ocr_options(&model, "auto")?;
    let worker = ocr_worker_root(&app)?;
    let mut response = inspect_worker(&worker, &serde_json::json!({
        "operation": "ocr_inspect",
        "cacheDir": ocr_cache_dir(&app)?,
        "model": model,
    }))?;
    if crate::optional_runtime::feature_installed("ocr_openvino") {
        let intel = crate::runtime_paths::ai_worker_dir_for("ocr_openvino")?;
        if let Ok(extra) = inspect_worker(&intel, &serde_json::json!({"operation":"ocr_inspect","cacheDir":ocr_cache_dir(&app)?,"model":model})) {
            response["backends"]["openvino"] = extra["backends"]["openvino"].clone();
            if response["cudaAvailable"] != true && extra["backends"]["openvino"]["available"] == true {
                response["recommendedBackend"] = serde_json::json!("openvino");
                response["activeDevice"] = serde_json::json!("openvino");
                response["resolvedLabel"] = extra["resolvedLabel"].clone();
            }
        }
    }
    let providers = response.get("executionProviders").and_then(|value| value.as_array())
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect::<Vec<_>>())
        .unwrap_or_default();
    let runtime_version = response.get("onnxRuntimeVersion").and_then(|value| value.as_str());
    let runtime_device = response.get("onnxRuntimeDevice").and_then(|value| value.as_str());
    let cuda_available = response.get("cudaAvailable").and_then(|value| value.as_bool()).unwrap_or(false);
    log::info!(
        "RapidOCR environment: model={}, runtime={}, ORT device={}, providers={:?}, CUDA ready={}",
        model, runtime_version.unwrap_or("unknown"), runtime_device.unwrap_or("unknown"), providers, cuda_available
    );
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
        cuda_available,
        active_device: response.get("activeDevice").and_then(|value| value.as_str()).unwrap_or("cpu").to_string(),
        onnx_runtime_version: runtime_version.map(str::to_string),
        onnx_runtime_device: runtime_device.map(str::to_string),
        execution_providers: providers,
        runtime_error: response.get("runtimeError").and_then(|value| value.as_str()).map(str::to_string),
        backends: response.get("backends").cloned().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default(),
        recommended_backend: response.get("recommendedBackend").and_then(|value| value.as_str()).unwrap_or("cpu").into(),
        resolved_label: response.get("resolvedLabel").and_then(|value| value.as_str()).unwrap_or("CPU").into(),
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
        .output_contained(std::time::Duration::from_secs(900))
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
    let data_root = crate::runtime_paths::data_dir(&app).map_err(|error| error.to_string())?;
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

pub fn install_model(app: AppHandle, model: String, backend: Option<String>) -> Result<(), String> {
    let _operation = MODEL_CACHE_OPERATION_LOCK
        .lock()
        .map_err(|_| "Không thể khóa thao tác model Speech Recognition.".to_string())?;
    validate_model(&model)?;
    let root = asr_cache_dir(&app)?;
    let backend = backend.unwrap_or_else(|| "ctranslate2".into());
    if !matches!(backend.as_str(), "ctranslate2" | "openvino") {
        return Err("Backend model không hợp lệ.".into());
    }
    let worker = worker_root(&app)?;
    if backend == "openvino" {
        if !matches!(model.as_str(), "tiny" | "base" | "small") {
            return Err("Whisper Intel hiện hỗ trợ tiny, base và small.".into());
        }
        let _gpu = crate::resource_scheduler::ai_gpu_slot().try_lock()
            .map_err(|_| "GPU đang xử lý một tác vụ khác. Hãy đợi hoàn tất rồi tải model Intel.".to_string())?;
        return run_ocr_install_worker(&app, &worker, &serde_json::json!({
            "operation": "asr_install", "model": model, "cacheDir": root, "backend": backend
        }));
    }
    let legacy_root = legacy_asr_cache_dir(&app)?;
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

pub fn remove_model(app: AppHandle, jobs: &AsrJobs, model: String, backend: Option<String>) -> Result<(), String> {
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
    if backend.as_deref() == Some("openvino") {
        if !matches!(model.as_str(), "tiny" | "base" | "small") { return Err("Model Intel không hợp lệ.".into()); }
        let intel_root = root.parent().ok_or("Thư mục model không hợp lệ.")?.join("whisper").join(&model);
        if intel_root.exists() { remove_model_child(&intel_root, "openvino")?; }
        return Ok(());
    }
    if backend.as_deref().is_some_and(|value| value != "ctranslate2") { return Err("Backend model không hợp lệ.".into()); }
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
    let cuda_available;
    let mut ocr_gpu = false;
    let mut asr_gpu = false;
    if options.mode == "ocr" {
        let environment = inspect_worker(&worker, &serde_json::json!({
            "operation": "ocr_inspect",
            "cacheDir": crate::runtime_paths::data_dir(&app).map_err(|error| error.to_string())?.join("models"),
            "model": options.ocr_model,
        }))?;
        cuda_available = environment.get("cudaAvailable").and_then(|value| value.as_bool()).unwrap_or(false);
        let provider_list = environment.get("executionProviders").map(ToString::to_string).unwrap_or_else(|| "[]".to_string());
        log::info!(
            "Starting OCR transcript: project={}, model={}, requested_device={}, ORT device={}, providers={}, CUDA ready={}",
            project_id,
            options.ocr_model,
            options.ocr_device,
            environment.get("onnxRuntimeDevice").and_then(|value| value.as_str()).unwrap_or("unknown"),
            provider_list,
            cuda_available,
        );
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
        let requested = options.ocr_device.as_str();
        if matches!(requested, "cuda" | "openvino")
            && !environment["backends"][requested]["available"].as_bool().unwrap_or(false) {
            return Err(environment["backends"][requested]["message"].as_str()
                .unwrap_or("Thiết bị đọc chữ chưa sẵn sàng. Chọn Tự động hoặc CPU.").into());
        }
        let resolved = if requested == "auto" {
            environment["recommendedBackend"].as_str().unwrap_or("cpu")
        } else { requested };
        ocr_gpu = matches!(resolved, "cuda" | "openvino");
    } else {
        let legacy_cache_dir = legacy_asr_cache_dir(&app)?;
        let alignment_language = if options.engine == "whisperX" { Some(options.language.clone()) } else { None };
        let environment = inspect_worker(&worker, &serde_json::json!({ "operation": "inspect", "model": options.model, "cacheDir": cache_dir, "legacyCacheDir": legacy_cache_dir, "alignmentLanguage": alignment_language, "engine": options.engine, "device": options.device }))?;
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
        asr_gpu = environment.get("resolvedBackend").and_then(|value| value.as_str()).is_some_and(|value| matches!(value, "cuda" | "openvino"));
        if let Some(error) = environment.get("selectionError").and_then(|value| value.as_str()) { return Err(error.into()); }
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
    let use_gpu = if options.mode == "ocr" {
        ocr_gpu
    } else {
        asr_gpu
    };
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
    ).map_err(|error| {if let Ok(mut entries)=jobs.inner.lock() {entries.remove(&job_id);} error})?;
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
    let extracted_dir = store.data_directory(project_id,"audio/extracted")?;
    let audio_path = extracted_dir.join(format!("transcription-{job_id}.wav"));
    let output_path = store.data_directory(project_id,"cache")?
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
        .spawn_contained()
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
    let segments = crate::transcript_service::merge_adjacent_duplicate_segments(segments);
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
    let temporary_root = store.data_directory(project_id,&format!("cache/ocr/{job_id}"))?;
    let artifacts = OcrArtifacts { root: temporary_root.clone() };
    let frames_dir = store.data_directory(project_id,&format!("cache/ocr/{job_id}/frames"))?;
    let output_path = temporary_root.join("transcript.json");
    emit_status(app, job_id, project_id, "progress", "Đang trích xuất frame trong vùng subtitle…", Some("ocr-extract"), Some(0.0));
    if canceled.load(Ordering::SeqCst) { return Err("Đã hủy transcription.".into()); }
    let filter = format!("crop={width}:{height}:{x}:{y},fps={}", options.ocr_fps as u8);
    let mut ffmpeg = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&video_path)
        .args(["-vf", &filter, "-vsync", "0", "-q:v", "4", "-start_number", "1"])
        .arg(frames_dir.join("frame_%08d.jpg"))
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn_contained()
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
    let segments = crate::transcript_service::merge_adjacent_duplicate_segments(segments);
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
    let operation = request.get("operation").and_then(|value| value.as_str()).unwrap_or("unknown");
    if operation == "ocr_transcribe" {
        log::info!("Starting RapidOCR worker request: model={}, requested_device={}, fps={}",
            request.get("model").and_then(|value| value.as_str()).unwrap_or("unknown"),
            request.get("device").and_then(|value| value.as_str()).unwrap_or("auto"),
            request.get("fps").and_then(|value| value.as_f64()).unwrap_or(0.0));
    }
    let mut child = worker_request_command(worker, request)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_worker()
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
    let (sender, receiver) = mpsc::sync_channel::<String>(8);
    thread::spawn(move || {
        for line in crate::worker_manager::bounded_lines(BufReader::new(stdout),8*1024*1024).map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_thread = thread::spawn(move || {
        crate::worker_manager::read_tail(stderr,2*1024*1024).unwrap_or_default()
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
        match operation {
            "ocr_transcribe" => log::error!("RapidOCR worker failed: {}; stderr={}", error, stderr_tail(&stderr, 40)),
            "transcribe" => log::error!("Speech recognition worker failed: {}; stderr={}", error, stderr_tail(&stderr, 60)),
            _ => {}
        }
        return Err(error);
    }
    if !exit_status.is_some_and(|status| status.success()) {
        log::error!("Speech recognition process stopped: {}", stderr.trim());
        return Err("Nhận dạng lời thoại đã dừng bất ngờ. Mở Cài đặt → Nhận dạng lời thoại để kiểm tra thành phần đã cài.".to_string());
    }
    if !completed {
        match operation {
            "ocr_transcribe" => log::error!("RapidOCR worker exited without completion: stderr={}", stderr_tail(&stderr, 40)),
            "transcribe" => log::error!("Speech recognition worker exited without completion: stderr={}", stderr_tail(&stderr, 60)),
            _ => {}
        }
        return Err("Nhận dạng lời thoại đã dừng trước khi hoàn tất bản chép lời.".to_string());
    }
    match operation {
        "ocr_transcribe" => log::info!("RapidOCR worker completed; diagnostics: {}", stderr_tail(&stderr, 40)),
        "transcribe" if !stderr.trim().is_empty() => log::info!("Speech recognition worker diagnostics: {}", stderr_tail(&stderr, 40)),
        _ => {}
    }
    Ok(())
}

fn stderr_tail(stderr: &str, limit: usize) -> String {
    stderr.lines().rev().take(limit).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")
}

fn inspect_worker(worker: &Path, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut child = worker_request_command(worker, request)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_worker()
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
    let mut child = worker_request_command(worker, request)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_worker()
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
        .wait_with_output_timeout(Duration::from_secs(2*3600))
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
    let mut child = worker_request_command(worker, request)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_worker()
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
    let mut non_protocol_output = Vec::new();
    let mut reader = BufReader::new(stdout);
    loop {
        let mut bytes = Vec::new();
        match reader.by_ref().take(8*1024*1024+1).read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(format!("Không thể đọc tiến trình tải model OCR: {error}"));
            }
            Ok(_) => {
                // Ignore carriage-return progress output from third-party OCR/CUDA
                // packages and parse only complete JSONL protocol messages.
                for raw_line in bytes.split(|byte| *byte == b'\r') {
                    let raw_line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
                    let line = String::from_utf8_lossy(raw_line);
                    let line = line.trim().trim_start_matches('\u{feff}').trim();
                    if line.is_empty() { continue; }
                    let envelope: serde_json::Value = match serde_json::from_str(line) {
                        Ok(value) => value,
                        Err(_) => {
                            remember_protocol_noise(&mut non_protocol_output, line);
                            continue;
                        }
                    };
                    match envelope.get("type").and_then(|value| value.as_str()) {
                        Some("progress") => { let event = if request.get("operation").and_then(|value| value.as_str()) == Some("asr_install") { "asr-model-progress" } else { "ocr-model-progress" }; let _ = app.emit(event, &envelope); }
                        Some("completed") | Some("failed") => final_message = Some(envelope),
                        _ => remember_protocol_noise(&mut non_protocol_output, line),
                    }
                }
            }
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
    if details.trim().is_empty() && non_protocol_output.is_empty() {
        Err("Python worker không hoàn tất tải model OCR. Hãy kiểm tra kết nối và thành phần OCR rồi thử lại.".into())
    } else {
        let mut diagnostic = Vec::new();
        if !details.trim().is_empty() {
            let trimmed = details.trim();
            diagnostic.push(trimmed.chars().rev().take(1800).collect::<String>().chars().rev().collect::<String>());
        }
        if !non_protocol_output.is_empty() {
            diagnostic.push(format!("đầu ra không phải JSON: {}", non_protocol_output.join(" | ")));
        }
        Err(format!("Python worker không hoàn tất tải model OCR: {}", diagnostic.join("; ")))
    }
}

fn remember_protocol_noise(lines: &mut Vec<String>, value: &str) {
    let concise: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect();
    if concise.trim().is_empty() { return; }
    if lines.len() == 6 { lines.remove(0); }
    lines.push(concise);
}

fn worker_request_command(worker: &Path, request: &serde_json::Value) -> Command {
    let operation = request.get("operation").and_then(|v| v.as_str()).unwrap_or("");
    let device = request.get("device").and_then(|v| v.as_str()).unwrap_or("cpu");
    let engine = request.get("engine").and_then(|v| v.as_str()).unwrap_or("fasterWhisper");
    if matches!(operation, "transcribe" | "inspect" | "asr_install") {
        let intel = device == "openvino" || engine == "openVinoWhisper" || request["backend"] == "openvino";
        let feature = if intel { "whisper_openvino" } else if engine == "whisperX" || crate::optional_runtime::feature_installed("whisperx") { "whisperx" } else { "acceleration" };
        let selected = crate::runtime_paths::ai_worker_dir_for(feature).unwrap_or_else(|_| worker.to_path_buf());
        return crate::runtime_paths::worker_command(&selected);
    }
    if operation == "whisperx_alignment_install" {
        let selected = crate::runtime_paths::ai_worker_dir_for("whisperx").unwrap_or_else(|_| worker.to_path_buf());
        return crate::runtime_paths::worker_command(&selected);
    }
    if request.get("operation").and_then(|value| value.as_str()).is_some_and(|value| value.starts_with("ocr_")) {
        // Inspection uses the supplied pack so independent reports can be merged.
        if operation == "ocr_inspect" { crate::runtime_paths::worker_command(worker) }
        else { crate::runtime_paths::ocr_command_for(worker, request["device"].as_str().unwrap_or("cpu")) }
    } else if request.get("operation").and_then(|value| value.as_str()).is_some_and(|value| matches!(value, "inspect" | "transcribe" | "asr_install")) {
        crate::runtime_paths::asr_worker_command(worker)
    } else {
        crate::runtime_paths::worker_command(worker)
    }
}

fn worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    if crate::optional_runtime::feature_installed("whisperx") {
        crate::runtime_paths::ai_worker_dir_for("whisperx")
    } else {
        crate::runtime_paths::ai_worker_dir()
    }
}

fn ocr_worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::ocr_worker_dir()
}

fn wait_killable(
    child: &mut Child,
    canceled: &AtomicBool,
) -> Result<std::process::ExitStatus, String> {
    let started=std::time::Instant::now();
    loop {
        if started.elapsed()>Duration::from_secs(12*60*60) {let _=child.kill();let _=child.wait();return Err("Trích xuất đã vượt thời gian chờ; dữ liệu nguồn được giữ nguyên.".into());}
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
    if !["auto", "fasterWhisper", "openVinoWhisper", "whisperX"].contains(&options.engine.as_str()) {
        return Err("Engine Speech Recognition không hợp lệ.".to_string());
    }
    validate_model(&options.model)?;
    if !["auto", "cpu", "cuda", "openvino"].contains(&options.device.as_str()) {
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
    if options.device == "openvino" && matches!(options.engine.as_str(), "fasterWhisper" | "whisperX") {
        return Err("Intel GPU cần chọn Tự động hoặc Whisper Intel.".into());
    }
    if options.engine == "openVinoWhisper" && !matches!(options.device.as_str(), "auto" | "openvino") {
        return Err("Whisper Intel chỉ hỗ trợ Intel GPU.".into());
    }
    if (options.device == "openvino" || options.engine == "openVinoWhisper") && !matches!(options.model.as_str(), "tiny" | "base" | "small") {
        return Err("Whisper Intel hiện hỗ trợ tiny, base và small.".into());
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
    if !["auto", "cpu", "cuda", "openvino"].contains(&device) {
        return Err("Thiết bị OCR phải là Tự động, CPU, NVIDIA hoặc Intel GPU.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn independent_core_capabilities_preserve_selected_intel_backend() {
        let mut selected = serde_json::json!({"resolvedBackend":"openvino", "modelCached":true,
            "intelModelCached":true, "whisperXInstalled":false, "backends":{"openvino":true}});
        let core = serde_json::json!({"resolvedBackend":"cpu", "modelCached":false,
            "whisperXInstalled":true, "cudaAvailable":true, "ctranslateModelCached":false,
            "backends":{"cpu":true,"cuda":true,"openvino":false}});
        super::merge_core_asr_capabilities(&mut selected, &core);
        assert_eq!(selected["resolvedBackend"], "openvino");
        assert_eq!(selected["modelCached"], true);
        assert_eq!(selected["whisperXInstalled"], true);
        assert_eq!(selected["backends"]["openvino"], true);
        assert_eq!(selected["backends"]["cuda"], true);
    }
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
