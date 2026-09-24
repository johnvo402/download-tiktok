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
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("Không thể xác định thư mục cache model: {error}"))?
        .join("fw");
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

fn default_asr_engine() -> String {
    "fasterWhisper".to_string()
}

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
        return Err("WhisperX chưa có trong Python Worker. Hãy cài runtime WhisperX theo hướng dẫn trong Settings/README rồi kiểm tra lại.".to_string());
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
            "AI Worker thiếu thành phần Speech Recognition. Hãy sửa chữa hoặc cài lại ứng dụng."
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
    options: AsrOptions,
) -> Result<String, String> {
    validate_options(&options)?;
    let projects_root = projects_root(&app)?;
    let store = ProjectStore::new(projects_root.clone());
    let project = store.get(&project_id)?;
    if project.source.audio_streams.is_empty() {
        return Err("Video không có audio stream để nhận dạng giọng nói.".to_string());
    }
    resolve_video(&store, &project_id)?;
    let worker = worker_root(&app)?;
    if !worker.is_dir() {
        return Err("Không tìm thấy thư mục Python worker trong checkout hiện tại.".to_string());
    }
    let cache_dir = asr_cache_dir(&app)?;
    let legacy_cache_dir = legacy_asr_cache_dir(&app)?;
    let alignment_language = if options.engine == "whisperX" {
        Some(options.language.clone())
    } else {
        None
    };
    let environment = inspect_worker(
        &worker,
        &serde_json::json!({ "operation": "inspect", "model": options.model, "cacheDir": cache_dir, "legacyCacheDir": legacy_cache_dir, "alignmentLanguage": alignment_language }),
    )?;
    let cuda_available = environment
        .get("cudaAvailable")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !environment
        .get("workerInstalled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(
            "AI Worker thiếu thành phần Speech Recognition. Hãy sửa chữa hoặc cài lại ứng dụng."
                .to_string(),
        );
    }
    if options.engine == "whisperX"
        && !environment
            .get("whisperXInstalled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        return Err("WhisperX chưa có trong Python Worker. Cài runtime WhisperX trong Settings trước khi tạo transcript.".to_string());
    }
    if options.engine == "whisperX"
        && !environment
            .get("alignmentModelCached")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        return Err(format!("WhisperX chưa cài aligner cho ngôn ngữ {}. Mở Cài đặt → Speech Recognition để cài trước.", options.language));
    }
    if options.device == "cuda"
        && !environment
            .get("cudaAvailable")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        return Err("CUDA chưa khả dụng cho faster-whisper. Chọn Auto hoặc CPU trong cấu hình Speech Recognition.".to_string());
    }
    if !environment
        .get("modelCached")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(format!(
        "Model faster-whisper '{}' chưa được cài. Mở Cài đặt → Speech Recognition để tải model trước khi tạo transcript.",
            options.model
        ));
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
    let use_gpu = options.device == "cuda" || options.device == "auto" && cuda_available;
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

    let extracted_channels = if project
        .source
        .audio_streams
        .first()
        .and_then(|stream| stream.channels)
        .unwrap_or(1)
        > 1
    {
        2
    } else {
        1
    };
    emit_status(
        app,
        job_id,
        project_id,
        "progress",
        if extracted_channels == 2 {
            "Đang trích xuất audio stereo 16 kHz để giữ riêng hai kênh…"
        } else {
            "Đang trích xuất audio mono 16 kHz…"
        },
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
        .arg(extracted_channels.to_string())
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
        .map_err(|error| format!("Không thể khởi chạy Python worker: {error}"))?;
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
                            .unwrap_or_else(|| "Python worker gặp lỗi.".to_string()),
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
                                    .unwrap_or_else(|| "Python worker gặp lỗi.".to_string()),
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
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            "Python worker đã dừng bất ngờ. Kiểm tra Python và faster-whisper.".to_string()
        } else {
            format!("Python worker thất bại: {detail}")
        });
    }
    if !completed {
        return Err("Python worker đã kết thúc nhưng không trả transcript hoàn tất.".to_string());
    }
    Ok(())
}

fn inspect_worker(worker: &Path, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut child = python_command(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!("Không thể khởi chạy Python worker. Kiểm tra Python 3.10+: {error}")
        })?;
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
            let details = String::from_utf8_lossy(&output.stderr);
            format!("Python worker không phản hồi hợp lệ: {}", details.trim())
        })?;
    let envelope: serde_json::Value = serde_json::from_slice(response)
        .map_err(|error| format!("Phản hồi Python worker không hợp lệ: {error}"))?;
    if envelope.get("type").and_then(|value| value.as_str()) != Some("capabilities") {
        return Err(envelope
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Python worker không thể kiểm tra model.")
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
                if details.trim().is_empty() {
                    "Không thể tải model faster-whisper.".to_string()
                } else {
                    format!("Không thể tải model faster-whisper: {}", details.trim())
                }
            }));
    }
    Ok(())
}

fn python_command(worker: &Path) -> Command {
    crate::runtime_paths::worker_command(worker)
}

fn worker_root(_app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::worker_dir()
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
            language: "auto".into()
        })
        .is_ok());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "cuda".into(),
            language: "vi".into()
        })
        .is_ok());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "unknown".into(),
            device: "auto".into(),
            language: "auto".into()
        })
        .is_err());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "shell".into(),
            language: "auto".into()
        })
        .is_err());
        assert!(validate_options(&AsrOptions {
            engine: "fasterWhisper".into(),
            model: "small".into(),
            device: "auto".into(),
            language: "../bad".into()
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
