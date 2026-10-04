use crate::hardware_acceleration::{self, EncodingProfile, VideoEncoder};
use crate::project_service::{BlurEffect, BlurRegionRole, Project, ProjectStore};
use crate::render_spec::{self, ExportOptions, RenderSpecification};
use crate::transcript_service::{self, Transcript, TranscriptTtsStatus};
use crate::video_service::{resolve_project_file, resolve_video};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct RenderJobs {
    inner: Arc<Mutex<HashMap<String, RenderJobEntry>>>,
    deleting: Arc<Mutex<HashSet<String>>>,
    start_lock: Arc<Mutex<()>>,
}

#[derive(Clone)]
struct RenderJobEntry {
    project_id: String,
    state: Arc<AtomicU8>,
}

const JOB_RUNNING: u8 = 0;
const JOB_COMMITTED: u8 = 1;
const JOB_CANCELLED: u8 = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportHistoryEntry {
    pub id: String,
    pub project_id: String,
    pub file: Option<String>,
    pub created_at: String,
    pub status: String,
    pub size_bytes: Option<u64>,
    pub duration: Option<f64>,
    pub settings: ExportOptions,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJobEvent {
    pub project_id: String,
    pub job_id: String,
    pub status: String,
    pub progress: f64,
    pub elapsed_seconds: f64,
    pub total_seconds: f64,
    pub speed: Option<f64>,
    pub output: Option<String>,
    pub message: String,
    pub layout_warnings: Vec<render_spec::SubtitleLayoutWarning>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportValidation {
    pub resolved_encoder: VideoEncoder,
    pub resolved_encoder_label: String,
    pub valid: bool,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub subtitle_region_id: Option<String>,
    pub translated_segments: usize,
    pub fits_normally: usize,
    pub fit_with_smaller_font: usize,
    pub smallest_font_size: u16,
    pub warnings: Vec<render_spec::SubtitleLayoutWarning>,
    pub dubbing_segments: usize,
    pub expected_audio: bool,
    pub audio_warnings: Vec<String>,
}

pub fn has_active_project_job(jobs: &RenderJobs, project_id: &str) -> bool {
    jobs.inner
        .lock()
        .map(|jobs| jobs.values().any(|job| job.project_id == project_id))
        .unwrap_or(true)
}

pub fn has_any_active_job(jobs: &RenderJobs) -> bool {
    jobs.inner.lock().map(|jobs| !jobs.is_empty()).unwrap_or(true)
}

pub fn validate_export(
    store: &ProjectStore,
    project_id: &str,
    options: &ExportOptions,
) -> Result<ExportValidation, String> {
    validate_export_options(options)?;
    resolve_export_directory(store, project_id, options.output_directory.as_deref())?;
    let (project, transcript, spec, dubbing_count, expected_audio) =
        prepare(store, project_id, options)?;
    let normal = spec
        .subtitle
        .events
        .iter()
        .filter(|event| event.layout.font_size == spec.subtitle.configured_font_size)
        .count();
    let smaller = spec.subtitle.events.len().saturating_sub(normal);
    let smallest_font_size = spec
        .subtitle
        .events
        .iter()
        .map(|event| event.layout.font_size)
        .min()
        .unwrap_or(0);
    let voices=if options.include_dubbing {crate::audio::timeline::voices(store,project_id,&transcript,true,spec.duration)?}else{Vec::new()};
    let mut audio_warnings=crate::audio::timeline::collisions(&voices).into_iter().map(|(a,b)|format!("Giọng đọc {a} và {b} chồng thời gian; audio được giữ đầy đủ.")).collect::<Vec<_>>();
    for segment in &transcript.segments {
        if options.include_dubbing { if let Some(message)=segment.tts.as_ref().and_then(|t|t.warning.clone()){audio_warnings.push(message);} }
    }
    let _ = project;
    Ok(ExportValidation {
        resolved_encoder: spec.output.encoder,
        resolved_encoder_label: spec.output.encoder.label().into(),
        valid: spec.subtitle.warnings.is_empty(),
        width: spec.width,
        height: spec.height,
        duration: spec.duration,
        subtitle_region_id: spec.subtitle.region_id.clone(),
        translated_segments: spec.subtitle.events.len(),
        fits_normally: normal,
        fit_with_smaller_font: smaller,
        smallest_font_size,
        warnings: spec.subtitle.warnings.clone(),
        dubbing_segments: dubbing_count,
        expected_audio,
        audio_warnings,
    })
}

pub fn start_export(
    app: AppHandle,
    jobs: &RenderJobs,
    store: ProjectStore,
    project_id: String,
    options: ExportOptions,
) -> Result<String, String> {
    let _guard = jobs
        .start_lock
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái export jobs.".to_string())?;
    if jobs
        .deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái export jobs.".to_string())?
        .contains(&project_id)
    {
        return Err("Dự án đang được xóa.".into());
    }
    if has_active_project_job(jobs, &project_id) {
        return Err("Đã có export đang chạy trong dự án này.".into());
    }
    let project = store.get(&project_id)?;
    if project.source.video_path.is_none() {
        return Err("Dự án chưa có video nguồn.".into());
    }
    // prepare_export_patches validates OCR/STTN only for missing work.
    // Completed immutable patches remain exportable after model removal.
    // Resolve and validate all immutable project inputs before a background job is registered.
    resolve_export_directory(&store, &project_id, options.output_directory.as_deref())?;
    let (_, _, spec, _, _) = prepare(&store, &project_id, &options)?;
    let id = Uuid::new_v4().to_string();
    crate::job_service::register_job(
        &app,
        id.clone(),
        project_id.clone(),
        project.name.clone(),
        "export",
        if spec.output.encoder.hardware() || crate::render::RenderBackend::configured()? != crate::render::RenderBackend::Legacy { &["gpu", "cpu", "ffmpeg", "disk"] } else { &["cpu", "ffmpeg", "disk"] },
        serde_json::to_value(&options).ok(),
    );
    let state = Arc::new(AtomicU8::new(JOB_RUNNING));
    jobs.inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái export jobs.".to_string())?
        .insert(
            id.clone(),
            RenderJobEntry {
                project_id: project_id.clone(),
                state: state.clone(),
            },
        );
    emit_event(
        &app,
        event(
            &project_id,
            &id,
            "queued",
            0.0,
            0.0,
            spec.duration,
            None,
            None,
            "Đang chuẩn bị tiến trình export…".into(),
            spec.subtitle.warnings,
        ),
    );
    let jobs = jobs.clone();
    let jobs_for_error = jobs.clone();
    let job_id = id.clone();
    thread::Builder::new()
        .name(format!("video-export-{}", &id[..8]))
        .spawn(move || {
            let cpu_permit = loop {
                if state.load(Ordering::SeqCst) == JOB_CANCELLED {
                    break None;
                }
                match crate::resource_scheduler::try_cpu() {
                    Ok(guard) => break Some(guard),
                    Err(std::sync::TryLockError::Poisoned(error)) => {
                        break Some(error.into_inner())
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            };
            let ffmpeg_permit = loop {
                if state.load(Ordering::SeqCst) == JOB_CANCELLED {
                    break None;
                }
                match crate::resource_scheduler::try_ffmpeg() {
                    Ok(guard) => break Some(guard),
                    Err(std::sync::TryLockError::Poisoned(error)) => {
                        break Some(error.into_inner())
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            };
            let _ = run_export(app, &store, &project_id, &job_id, options, state);
            drop(ffmpeg_permit);
            drop(cpu_permit);
            if let Ok(mut active) = jobs.inner.lock() {
                active.remove(&job_id);
            }
        })
        .map_err(|error| {
            if let Ok(mut active) = jobs_for_error.inner.lock() {
                active.remove(&id);
            }
            format!("Không thể khởi chạy tiến trình export: {error}")
        })?;
    Ok(id)
}

pub fn cancel_export(jobs: &RenderJobs, job_id: &str) -> Result<bool, String> {
    let active = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái export jobs.".to_string())?;
    let Some(job) = active.get(job_id) else {
        return Ok(false);
    };
    Ok(job
        .state
        .compare_exchange(
            JOB_RUNNING,
            JOB_CANCELLED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_ok())
}

pub fn history(store: &ProjectStore, project_id: &str) -> Result<Vec<ExportHistoryEntry>, String> {
    let root = store.project_directory(project_id)?;
    let path = root.join("exports").join("history.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("Không thể đọc export history: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Export history bị lỗi: {error}"))
}

pub fn export_directory(store: &ProjectStore, project_id: &str) -> Result<PathBuf, String> {
    store.get(project_id)?;
    let project_root = store.project_directory(project_id)?;
    let export_dir = project_root.join("exports");
    fs::create_dir_all(&export_dir)
        .map_err(|error| format!("Không thể mở thư mục exports của dự án: {error}"))?;
    let canonical_root = fs::canonicalize(&project_root)
        .map_err(|error| format!("Không thể xác minh thư mục project: {error}"))?;
    let canonical_exports = fs::canonicalize(&export_dir)
        .map_err(|error| format!("Không thể xác minh thư mục exports: {error}"))?;
    if !canonical_exports.starts_with(&canonical_root) {
        return Err("Thư mục exports nằm ngoài phạm vi project được quản lý.".into());
    }
    Ok(canonical_exports)
}

fn resolve_export_directory(
    store: &ProjectStore,
    project_id: &str,
    requested_directory: Option<&str>,
) -> Result<PathBuf, String> {
    store.get(project_id)?;
    let Some(requested_directory) = requested_directory.filter(|path| !path.trim().is_empty())
    else {
        return export_directory(store, project_id);
    };
    let requested_path = PathBuf::from(requested_directory);
    if !requested_path.is_absolute() {
        return Err(
            "Thư mục lưu export phải là đường dẫn tuyệt đối đã chọn trong hộp thoại hệ thống."
                .into(),
        );
    }
    let canonical = fs::canonicalize(&requested_path)
        .map_err(|error| format!("Không thể truy cập thư mục lưu export đã chọn: {error}"))?;
    if !canonical.is_dir() {
        return Err("Vị trí lưu export đã chọn không phải là thư mục.".into());
    }
    // Confirm the directory is writable before starting a long render. The temporary
    // probe is removed automatically and does not touch existing user files.
    tempfile::Builder::new()
        .prefix(".avl-write-check-")
        .tempfile_in(&canonical)
        .map_err(|error| format!("Không thể ghi vào thư mục export đã chọn: {error}"))?;
    Ok(canonical)
}

pub fn open_export_folder(
    store: &ProjectStore,
    project_id: &str,
    requested_directory: Option<&str>,
) -> Result<String, String> {
    let export_dir = resolve_export_directory(store, project_id, requested_directory)?;
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command
        .arg(&export_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console_window(&mut command);
    command
        .spawn()
        .map_err(|error| format!("Không thể mở thư mục export: {error}"))?;
    Ok(export_dir.to_string_lossy().into_owned())
}

pub fn set_project_deleting(jobs: &RenderJobs, project_id: &str) -> Result<(), String> {
    jobs.deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái export jobs.".to_string())?
        .insert(project_id.to_string());
    Ok(())
}
pub fn clear_project_deleting(jobs: &RenderJobs, project_id: &str) {
    if let Ok(mut deleting) = jobs.deleting.lock() {
        deleting.remove(project_id);
    }
}
pub fn cancel_project_jobs_and_wait(jobs: &RenderJobs, project_id: &str) -> Result<(), String> {
    set_project_deleting(jobs, project_id)?;
    if let Ok(active) = jobs.inner.lock() {
        for job in active.values().filter(|job| job.project_id == project_id) {
            let _ = job.state.compare_exchange(
                JOB_RUNNING,
                JOB_CANCELLED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
    }
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let active = jobs
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái export jobs.".to_string())?
            .values()
            .any(|job| job.project_id == project_id);
        if !active {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("Đang dừng quá trình export. Thử xóa dự án lại sau vài giây.".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn prepare(
    store: &ProjectStore,
    project_id: &str,
    options: &ExportOptions,
) -> Result<(Project, Transcript, RenderSpecification, usize, bool), String> {
    validate_export_options(options)?;
    let project = store.get(project_id)?;
    let source = resolve_video(store, project_id)?;
    let duration = project
        .source
        .duration
        .ok_or("Không đọc được thời lượng video.")?;
    let transcript = transcript_service::get_transcript(store, project_id)?.unwrap_or(Transcript {
        schema_version: 1,
        project_id: project_id.to_string(),
        source_type: crate::project_service::TranscriptSourceType::Asr,
        source_track: None,
        language: None,
        video_checksum: project.source.checksum_sha256.clone(),
        segments: Vec::new(),
    });
    let mut spec = render_spec::build_render_specification(&project, &transcript, duration, options)?;
    if !options.apply_blur
        && project
            .blur_regions
            .iter()
            .any(|region| region.role == BlurRegionRole::Subtitle && region.enabled)
        && project.subtitle.enabled
        && options.burn_subtitles
    {
        // The explicit region override is still the sole way to preserve the required pair.
        return Err("Vùng blur phụ đề phải được áp dụng khi burn translated subtitle.".into());
    }
    if options.apply_logos {
        for logo in spec.logos.iter().filter(|logo| logo.enabled) {
            resolve_project_file(store, project_id, &logo.asset_path, "media/overlays")?;
        }
    }
    let dub_paths = resolve_dubbing(store, project_id, &transcript, options.include_dubbing)?;
    let expected_audio =
        spec.audio.include_original || spec.audio.include_dubbing && !dub_paths.is_empty();
    if options.include_dubbing && dub_paths.is_empty() {
        return Err("Đã chọn lồng tiếng nhưng chưa có các tệp giọng đọc hợp lệ. Hãy tạo TTS trước khi export.".into());
    }
    if options.dialogue_reduction && (!options.include_original_audio || project.source.audio_streams.is_empty()) {
        return Err("Giảm thoại gốc cần bật Âm thanh gốc và video phải có audio.".into());
    }
    let source_meta =
        fs::metadata(source).map_err(|error| format!("Không đọc được video nguồn: {error}"))?;
    if source_meta.len() == 0 {
        return Err("Video nguồn rỗng.".into());
    }
    // Resolve only after project/media validation; the specification passed to FFmpeg is concrete.
    spec.output.encoder = hardware_acceleration::resolve(options.video_codec, options.encoder)?;
    Ok((project, transcript, spec, dub_paths.len(), expected_audio))
}

fn validate_export_options(options: &ExportOptions) -> Result<(), String> {
    validate_output_file_name(&options.output_file_name)?;
    if options
        .video_bitrate_kbps
        .is_some_and(|bitrate| !(500..=50_000).contains(&bitrate))
    {
        return Err("Bitrate video phải nằm trong khoảng 500–50.000 kbps.".into());
    }
    if ![
        options.original_volume,
        options.dubbed_volume,
        options.ducking_amount,
        options.dialogue_reduction_amount,
    ]
    .into_iter()
    .all(f64::is_finite)
        || !(0.0..=1.0).contains(&options.original_volume)
        || !(0.0..=1.0).contains(&options.dubbed_volume)
        || !(0.0..=1.0).contains(&options.ducking_amount)
        || !(0.0..=1.0).contains(&options.dialogue_reduction_amount)
    {
        return Err("Mức âm lượng export phải nằm trong khoảng 0–100%.".into());
    }
    Ok(())
}

fn resolve_dubbing(
    store: &ProjectStore,
    project_id: &str,
    transcript: &Transcript,
    include_dubbing: bool,
) -> Result<Vec<(PathBuf, f64, f64, String)>, String> {
    let mut result = Vec::new();
    if include_dubbing {
        for segment in &transcript.segments {
            if segment.translated.trim().is_empty() {
                continue;
            }
            let tts = segment
                .tts
                .as_ref()
                .ok_or_else(|| format!("Segment {} chưa có voice-over.", segment.id))?;
            if !matches!(tts.status,Some(TranscriptTtsStatus::Ready | TranscriptTtsStatus::DurationOverflow)) {
                return Err(format!(
                    "Voice-over của segment {} đã cũ hoặc lỗi thời. Tạo lại TTS trước khi export.",
                    segment.id
                ));
            }
            let relative = tts.audio_path.as_deref().ok_or_else(|| {
                format!("Không tìm thấy đường dẫn TTS cho segment {}.", segment.id)
            })?;
            let path = resolve_project_file(store, project_id, relative, "audio/tts")?;
            let duration = tts
                .duration
                .filter(|d| d.is_finite() && *d > 0.0)
                .ok_or_else(|| {
                    format!("Thời lượng TTS của segment {} không hợp lệ.", segment.id)
                })?;
            if !segment.start.is_finite() || segment.start < 0.0 {
                return Err(format!(
                    "Timestamp voice-over của segment {} không hợp lệ.",
                    segment.id
                ));
            }
            result.push((path, segment.start, duration, segment.id.clone()));
        }
    }
    Ok(result)
}

fn run_export(
    app: AppHandle,
    store: &ProjectStore,
    project_id: &str,
    job_id: &str,
    options: ExportOptions,
    state: Arc<AtomicU8>,
) -> Result<(), String> {
    let started = Instant::now();
    let warnings = prepare(store, project_id, &options)
        .map(|(_, _, spec, _, _)| spec.subtitle.warnings)
        .unwrap_or_default();
    let result = if state.load(Ordering::SeqCst) == JOB_CANCELLED {
        Err("Đã hủy export trước khi bắt đầu mã hóa.".into())
    } else {
        run_export_inner(&app, store, project_id, job_id, &options, &state, started)
    };
    let status = if state.load(Ordering::SeqCst) == JOB_CANCELLED {
        "cancelled"
    } else if result.is_ok() {
        "completed"
    } else {
        "failed"
    };
    let (output, message) = match &result {
        Ok((path, _)) => (
            Some(path.clone()),
            "Export hoàn tất và đã xác minh bằng FFprobe.".to_string(),
        ),
        Err(error) => (None, error.clone()),
    };
    let total = store
        .get(project_id)
        .ok()
        .and_then(|p| p.source.duration)
        .unwrap_or(0.0);
    let history_entry = ExportHistoryEntry {
        id: job_id.into(),
        project_id: project_id.into(),
        file: output.clone(),
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        status: status.into(),
        size_bytes: output.as_ref().and_then(|stored_path| {
            let path = Path::new(stored_path);
            let resolved = if path.is_absolute() {
                Some(path.to_path_buf())
            } else {
                store
                    .project_directory(project_id)
                    .ok()
                    .map(|root| root.join(path))
            }?;
            fs::metadata(resolved).ok().map(|metadata| metadata.len())
        }),
        duration: if result.is_ok() { Some(total) } else { None },
        settings: options,
        message: Some(message.clone()),
    };
    if let Err(error) = write_history(store, project_id, history_entry) {
        emit_event(
            &app,
            event(
                project_id,
                job_id,
                "failed",
                0.0,
                started.elapsed().as_secs_f64(),
                total,
                None,
                None,
                format!("{message} Lưu lịch sử export thất bại: {error}"),
                Vec::new(),
            ),
        );
        return Err(error);
    }
    if result.is_err() {
        if let Ok(root) = store.project_directory(project_id) {
            let _ = fs::remove_dir_all(root.join("cache").join("render").join(job_id));
        }
    }
    let progress = if status == "completed" { 1.0 } else { 0.0 };
    emit_event(
        &app,
        event(
            project_id,
            job_id,
            status,
            progress,
            started.elapsed().as_secs_f64(),
            total,
            None,
            output,
            message,
            warnings,
        ),
    );
    result.map(|_| ())
}

fn run_export_inner(
    app: &AppHandle,
    store: &ProjectStore,
    project_id: &str,
    job_id: &str,
    options: &ExportOptions,
    state: &Arc<AtomicU8>,
    started: Instant,
) -> Result<(String, f64), String> {
    let (project, transcript, mut spec, _, _expected_audio) = prepare(store, project_id, options)?;
    if !spec.subtitle.warnings.is_empty() {
        let ids = spec
            .subtitle
            .warnings
            .iter()
            .map(|warning| warning.segment_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("Một số phụ đề không vừa Subtitle Blur ở cỡ tối thiểu. Segment: {ids}. Hãy mở rộng vùng blur hoặc giảm cỡ chữ/dịch ngắn hơn; không có chữ nào bị cắt."));
    }
    let project_root = store.project_directory(project_id)?;
    let source_path = resolve_video(store, project_id)?;
    let render_dir = project_root.join("cache").join("render").join(job_id);
    fs::create_dir_all(&render_dir)
        .map_err(|error| format!("Không thể tạo thư mục render tạm: {error}"))?;
    let export_dir =
        resolve_export_directory(store, project_id, options.output_directory.as_deref())?;
    let ass_path = render_dir.join("translated.ass");
    if spec.subtitle.enabled || spec.text_overlays.iter().any(|overlay| overlay.enabled && !overlay.text.trim().is_empty()) {
        fs::write(&ass_path, render_spec::generate_ass(&spec)?)
            .map_err(|error| format!("Không thể tạo phụ đề ASS: {error}"))?;
    }
    let source_duration = spec.duration;
    let ai_patches = crate::text_removal_service::prepare_export_patches(
        app,
        &spec,
        &source_path,
        &render_dir,
        state,
        |progress, message| {
            emit_event(app, event(
                project_id,
                job_id,
                "running",
                progress,
                started.elapsed().as_secs_f64(),
                source_duration,
                None,
                None,
                message,
                spec.subtitle.warnings.clone(),
            ));
        },
    )?;
    let dialogue_stem = if options.dialogue_reduction {
        prepare_dialogue_stem(app, store, project_id, &project, &source_path, &render_dir, job_id, source_duration, started, state)?
    } else {
        None
    };
    let voices = if options.include_dubbing { crate::audio::timeline::voices(store, project_id, &transcript, true, spec.duration)? } else { Vec::new() };
    let conflicts = crate::audio::timeline::collisions(&voices);
    let audio_manifest = serde_json::json!({"engineVersion":2,"dubbingSegments":voices.iter().map(|v|serde_json::json!({
        "id":v.id,"audioStart":v.start,"audioEnd":v.end,"speechStart":v.speech_start,"speechEnd":v.speech_end
    })).collect::<Vec<_>>()});
    if !conflicts.is_empty() { log::warn!("Audio speech intervals overlap: {conflicts:?}"); }
    let audio_expected = spec.audio.include_original || !voices.is_empty();
    let master_audio = if audio_expected {
        let path = render_dir.join("master-audio.wav");
        crate::audio::mixer::render(
            if spec.audio.include_original {Some(source_path.as_path())}else{None},dialogue_stem.as_deref(),voices,
            crate::audio::mixer::Mix::export(&spec.audio,project.dubbing.audio_processing.clone()),spec.duration,&path,
            ||state.load(Ordering::SeqCst)==JOB_CANCELLED,
            |p|emit_event(app,event(project_id,job_id,"running",p*0.04,started.elapsed().as_secs_f64(),source_duration,None,None,
                "Đang trộn âm thanh…".into(),Vec::new())))?;
        Some(path)
    } else {None};
    // Every visual backend receives the same processed/mixed master, encoded to AAC once.
    let dub_items: Vec<(PathBuf,f64,f64,String)> = Vec::new();
    let logo_paths = if options.apply_logos { spec.logos.iter().filter(|l|l.enabled)
        .map(|l|resolve_project_file(store,project_id,&l.asset_path,"media/overlays")).collect::<Result<Vec<_>,_>>()? } else {Vec::new()};
    let mut graph = build_video_filtergraph(&spec,options,&ass_path,0,&ai_patches);
    graph.push_str(&master_audio_filter(1+logo_paths.len()+ai_patches.len(),audio_expected));
    let filter_path = render_dir.join("filtergraph.txt");
    fs::write(&filter_path, &graph)
        .map_err(|error| format!("Không thể ghi filtergraph: {error}"))?;
    fs::write(
        render_dir.join("audio-manifest.json"),
        serde_json::to_vec_pretty(&audio_manifest).unwrap_or_default(),
    )
    .map_err(|error| format!("Không thể ghi manifest audio: {error}"))?;
    let final_path = unique_output_path(&export_dir, &options.output_file_name);
    let staging_file = tempfile::Builder::new()
        .prefix(".avl-render-")
        .suffix(".partial.mp4")
        .tempfile_in(&export_dir)
        .map_err(|error| format!("Không thể tạo tệp render tạm trong thư mục đích: {error}"))?;
    let staging_path = staging_file.into_temp_path();
    let render_backend = crate::render::RenderBackend::configured()?;
    let media_preference = crate::media_gpu::pipeline::Preference::configured()?;
    let mut native_video: Option<tempfile::TempPath> = None;
    let mut gpu_permit = None;
    let mut gpu_renderer = None;
    let mut gpu_fps = 0.0;
    if media_preference == crate::media_gpu::pipeline::Preference::ZeroCopy {
        return Err("Thiết bị hiện chưa hỗ trợ zero-copy trực tiếp. Hãy dùng Auto hoặc GPU-copy.".into());
    }
    if render_backend == crate::render::RenderBackend::Legacy && media_preference == crate::media_gpu::pipeline::Preference::GpuCopy {
        return Err("Không thể bật GPU-copy cùng với bộ render tương thích. Hãy chọn Auto.".into());
    }
    #[cfg(windows)]
    if render_backend != crate::render::RenderBackend::Legacy && media_preference != crate::media_gpu::pipeline::Preference::Cpu {
        let native_attempt = (|| -> Result<tempfile::TempPath,String> {
            if !spec.output.encoder.hardware() {return Err("Bộ mã hóa CPU cần pipeline staging.".into());}
            if !ai_patches.is_empty() {return Err("Vùng phục hồi AI hiện cần pipeline staging.".into());}
            crate::media_gpu::pipeline::preflight(&source_path)?;
            loop {
                if state.load(Ordering::SeqCst)==JOB_CANCELLED{return Err("Đã hủy export.".into());}
                match crate::resource_scheduler::ai_gpu_slot().try_lock(){
                    Ok(p)=>{gpu_permit=Some(p);break;},
                    Err(std::sync::TryLockError::Poisoned(e))=>{gpu_permit=Some(e.into_inner());break;},
                    Err(std::sync::TryLockError::WouldBlock)=>thread::sleep(Duration::from_millis(50)),
                }
            }
            let temp=tempfile::Builder::new().prefix("native-video-").suffix(".mp4").tempfile_in(&render_dir)
                .map_err(|e|format!("Không tạo được video tạm: {e}"))?.into_temp_path();
            let bitrate=crate::media_gpu::pipeline::bitrate(&source_path,&spec)?;
            let mut pipeline=crate::media_gpu::windows::NativePipeline::new(&source_path,&temp,
                crate::render::graph::RenderScene::from_spec(&spec),&logo_paths,spec.output.video_codec,bitrate,
                crate::media_gpu::capability::vendor(spec.output.encoder))?;
            let mut last=Instant::now();
            pipeline.run(state,|pts,frames|{
                if last.elapsed()<Duration::from_millis(250){return;}
                last=Instant::now();
                emit_event(app,event(project_id,job_id,"running",(pts/source_duration*0.9).clamp(0.0,0.9),
                    started.elapsed().as_secs_f64(),source_duration,None,None,
                    format!("Đang xử lý video trên GPU · {frames} khung · {}",spec.output.encoder.label()),spec.subtitle.warnings.clone()));
            })?;
            Ok(temp)
        })();
        drop(gpu_permit.take());
        match native_attempt {
            Ok(temp)=>native_video=Some(temp),
            Err(e) if media_preference==crate::media_gpu::pipeline::Preference::Auto && state.load(Ordering::SeqCst)!=JOB_CANCELLED=>{
                log::warn!("Native GPU pipeline unavailable; Tier C fallback: {e}");
            },
            Err(e)=>return Err(e),
        }
    }
    #[cfg(not(windows))]
    if media_preference == crate::media_gpu::pipeline::Preference::GpuCopy {
        return Err("Pipeline chia sẻ texture hiện chỉ hỗ trợ Windows.".into());
    }
    if native_video.is_none() && render_backend != crate::render::RenderBackend::Legacy {
        let initialized = (|| {
            gpu_fps = crate::render::stream::compatible_input(&source_path)?;
            loop {
                if state.load(Ordering::SeqCst) == JOB_CANCELLED { return Err("Đã hủy export.".into()); }
                match crate::resource_scheduler::ai_gpu_slot().try_lock() {
                    Ok(permit) => { gpu_permit = Some(permit); break; }
                    Err(std::sync::TryLockError::Poisoned(error)) => { gpu_permit = Some(error.into_inner()); break; }
                    Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(50)),
                }
            }
            crate::render::renderer::Renderer::new_preferred(crate::render::graph::RenderScene::from_spec(&spec), &logo_paths, crate::media_gpu::capability::vendor(spec.output.encoder))
        })();
        match initialized {
            Ok(renderer) => gpu_renderer = Some(renderer),
            Err(error) if render_backend == crate::render::RenderBackend::Auto && state.load(Ordering::SeqCst) != JOB_CANCELLED => {
                log::warn!("GPU visual renderer unavailable; fallback legacy: {error}");
                gpu_permit = None;
                emit_event(app, event(project_id, job_id, "running", 0.0, started.elapsed().as_secs_f64(), source_duration, None, None,
                    "Đang dùng bộ render tương thích FFmpeg.".into(), spec.subtitle.warnings.clone()));
            }
            Err(error) => return Err(error),
        }
    }
    let candidates = hardware_acceleration::fallback_candidates(options.video_codec, options.encoder, spec.output.encoder)?;
    for (attempt, backend) in candidates.iter().copied().enumerate() {
        spec.output.encoder = backend;
        if let Some(renderer) = gpu_renderer.as_mut() {
            let raw_index = 1 + dub_items.len() + usize::from(master_audio.is_some());
            let mut gpu_graph = format!("[{raw_index}:v]null[vout];");
            gpu_graph.push_str(&master_audio_filter(1,audio_expected));
            let gpu_filter = render_dir.join("audio-gpu-filtergraph.txt");
            fs::write(&gpu_filter, gpu_graph).map_err(|error| format!("Không ghi được audio graph: {error}"))?;
            let command = build_ffmpeg_command(crate::runtime_paths::ffmpeg_command()?, &source_path, &dub_items, &[], &[], master_audio.as_deref(),
                &gpu_filter, &staging_path, &spec, audio_expected, Some(gpu_fps))?;
            let rendered = crate::render::stream::run(renderer, crate::render::stream::decoder_command(&source_path)?, command, gpu_fps, &ai_patches, state, |pts, frames| {
                let fraction = (pts / source_duration).clamp(0.0, 0.995);
                emit_event(app, event(project_id, job_id, "running", if ai_patches.is_empty() { fraction } else { 0.15 + fraction * 0.845 }, started.elapsed().as_secs_f64(), source_duration, None, None,
                    format!("Đang render GPU · {frames} khung · {}", backend.label()), spec.subtitle.warnings.clone()));
            });
            match rendered {
                Ok(_) => break,
                Err(error) => {
                    log::error!("GPU export failed: {}; {}", error.message, error.details);
                    if state.load(Ordering::SeqCst) == JOB_CANCELLED { return Err("Đã hủy export.".into()); }
                    if error.encoder && options.encoder == VideoEncoder::Auto && attempt + 1 < candidates.len()
                        && hardware_acceleration::is_encoder_initialization_error(&error.details) { continue; }
                    if render_backend != crate::render::RenderBackend::Auto { return Err(error.message); }
                    emit_event(app, event(project_id, job_id, "running", 0.0, started.elapsed().as_secs_f64(), source_duration, None, None,
                        "Render GPU gặp lỗi; đang xuất lại bằng bộ render tương thích.".into(), spec.subtitle.warnings.clone()));
                    gpu_renderer = None;
                    drop(gpu_permit.take());
                }
            }
        }
        let mut compatibility_patches=ai_patches.clone();
        if native_video.is_none(){
            for patch in &mut compatibility_patches {
                patch.image_pattern=crate::text_removal_service::compatibility_patch_path(app,patch,state)?;
            }
        }
        let mut command = if let Some(video) = native_video.as_deref() {
            let audio_filter=render_dir.join("audio-native-filtergraph.txt");
            fs::write(&audio_filter,master_audio_filter(1,audio_expected)).map_err(|e|e.to_string())?;
            build_native_mux_command(&source_path,video,&dub_items,master_audio.as_deref(),&audio_filter,&staging_path,&spec,audio_expected)?
        } else { build_ffmpeg_command(
            crate::runtime_paths::ffmpeg_command()?,
            &source_path,
            &dub_items,
            &logo_paths,
            &compatibility_patches,
            master_audio.as_deref(),
            &filter_path,
            &staging_path,
            &spec,
            audio_expected,
            None,
        )? };
        if state.load(Ordering::SeqCst) == JOB_CANCELLED {
            return Err("Đã hủy export.".into());
        }
        let mut child = command.spawn().map_err(|error| {
            format!("Không thể chạy FFmpeg. Kiểm tra cài đặt FFmpeg và PATH: {error}")
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Không đọc được tiến trình FFmpeg.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Không đọc được thông báo lỗi từ FFmpeg.")?;
        let (tx, rx) = mpsc::channel::<String>();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().flatten() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let stderr_tail_reader = stderr_tail.clone();
        let stderr_reader = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                if let Ok(mut tail) = stderr_tail_reader.lock() {
                    tail.push_back(line);
                    while tail.len() > 16 {
                        tail.pop_front();
                    }
                }
            }
        });
        let mut out_seconds: f64 = 0.0;
        let mut encoded_frames = 0_u64;
        let mut last_emit = Instant::now();
        let mut speed: Option<f64> = None;
        loop {
            if state.load(Ordering::SeqCst) == JOB_CANCELLED {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                let _ = fs::remove_file(&staging_path);
                return Err("Đã hủy export.".into());
            }
            while let Ok(line) = rx.try_recv() {
                if let Some((key, value)) = line.split_once('=') {
                    if key == "frame" { encoded_frames = value.trim().parse().unwrap_or(encoded_frames); }
                    if key == "out_time_ms" {
                        out_seconds = value.parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
                    }
                    if key == "speed" {
                        speed = value.trim_end_matches('x').parse().ok();
                    }
                    if key == "progress" && last_emit.elapsed() >= Duration::from_millis(250) {
                        let encode_percent = (out_seconds / source_duration).clamp(0.0, 0.995);
                        let percent = if native_video.is_some() { 0.9 + encode_percent * 0.095 } else if ai_patches.is_empty() { encode_percent } else { 0.15 + encode_percent * 0.845 };
                        emit_event(
                            app,
                            event(
                                project_id,
                                job_id,
                                "running",
                                percent,
                                started.elapsed().as_secs_f64(),
                                source_duration,
                                speed,
                                None,
                                if native_video.is_some() { "Đang ghép âm thanh và hoàn tất MP4.".into() } else {format!("Đang mã hóa video · {}", spec.output.encoder.label())},
                                spec.subtitle.warnings.clone(),
                            ),
                        );
                        last_emit = Instant::now();
                    }
                }
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("Không thể theo dõi FFmpeg: {error}"))?
            {
                if !status.success() {
                    let _ = stderr_reader.join();
                    let _ = fs::remove_file(&staging_path);
                    let details = stderr_tail
                        .lock()
                        .map(|tail| tail.iter().rev().cloned().collect::<Vec<_>>().join(" "))
                        .unwrap_or_default();
                    if native_video.is_none() && options.encoder == VideoEncoder::Auto && encoded_frames == 0
                        && attempt + 1 < candidates.len() && hardware_acceleration::is_encoder_initialization_error(&details) {
                        log::warn!("Auto encode initialization failed on {backend:?}, trying next encoder: {details}");
                        emit_event(app, event(project_id, job_id, "running", if ai_patches.is_empty() { 0.0 } else { 0.15 }, started.elapsed().as_secs_f64(), source_duration, None, None,
                            format!("{} không khởi tạo được; đang thử {}…", backend.label(), candidates[attempt + 1].label()), spec.subtitle.warnings.clone()));
                        break;
                    }
                    return Err(hardware_acceleration::encoding_error(options.video_codec, backend, &details));
                }
                let _ = stderr_reader.join();
                break;
            }
            thread::sleep(Duration::from_millis(80));
        }
        // A successful attempt proceeds to output validation; a failed init tries the next candidate.
        if child.wait().map_err(|error| error.to_string())?.success() { break; }
    }
    if state.load(Ordering::SeqCst) == JOB_CANCELLED {
        let _ = fs::remove_file(&staging_path);
        return Err("Đã hủy export.".into());
    }
    validate_output(
        &staging_path,
        spec.width,
        spec.height,
        source_duration,
        audio_expected,
    )?;
    if state
        .compare_exchange(
            JOB_RUNNING,
            JOB_COMMITTED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        let _ = fs::remove_file(&staging_path);
        return Err("Đã hủy export.".into());
    }
    staging_path
        .persist_noclobber(&final_path)
        .map_err(|error| {
            format!(
                "Không thể lưu video export mà không ghi đè tệp hiện có: {}",
                error.error
            )
        })?;
    let _ = fs::remove_dir_all(&render_dir);
    Ok((final_path.to_string_lossy().into_owned(), source_duration))
}

fn prepare_dialogue_stem(
    app: &AppHandle,
    store: &ProjectStore,
    project_id: &str,
    project: &Project,
    source: &Path,
    render_dir: &Path,
    job_id: &str,
    duration: f64,
    started: Instant,
    state: &Arc<AtomicU8>,
) -> Result<Option<PathBuf>, String> {
    if !crate::dialogue_service::get_environment(app)?.ready() {
        return Err("Giảm thoại gốc cần runtime PyTorch và model BandIt v2 đã cài. Mở Cài đặt → Xử lý âm thanh để kiểm tra.".into());
    }
    let project_root = store.project_directory(project_id)?;
    let cache_dir = project_root.join("cache").join("audio-separation");
    fs::create_dir_all(&cache_dir).map_err(|error| format!("Không thể tạo cache tách thoại: {error}"))?;
    for path in [project_root.join("cache"), cache_dir.clone()] {
        let metadata = fs::symlink_metadata(&path).map_err(|error| format!("Không thể xác minh cache tách thoại: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Thư mục cache tách thoại không hợp lệ hoặc là symlink.".into());
        }
    }
    let source_hash = project.source.checksum_sha256.as_deref()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .map(Ok)
        .unwrap_or_else(|| file_sha256(source))?;
    let cached = cache_dir.join(format!("{source_hash}_bandit-v2-multi_speech.wav"));
    if let Ok(metadata) = fs::symlink_metadata(&cached) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Cache stem thoại không hợp lệ; không sử dụng file này.".into());
        }
        if metadata.len() > 44 && crate::audio::signal::validate_stem(&cached,duration).is_ok() { return Ok(Some(cached)); }
        fs::remove_file(&cached).map_err(|error| format!("Không thể dọn stem thoại lỗi: {error}"))?;
    }
    if state.load(Ordering::SeqCst) == JOB_CANCELLED { return Err("Đã hủy export.".into()); }
    emit_event(app, event(project_id, job_id, "running", 0.01, started.elapsed().as_secs_f64(), duration, None, None,
        "Đang chuẩn bị audio 48 kHz để tách thoại…".into(), Vec::new()));
    let input_wav = render_dir.join("dialogue-input.wav");
    let mut extract = crate::runtime_paths::ffmpeg_command()?;
    extract.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .args(["-map", "0:a:0", "-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"])
        .arg(&input_wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = extract.spawn().map_err(|error| format!("Không thể trích audio để tách thoại: {error}"))?;
    loop {
        if state.load(Ordering::SeqCst) == JOB_CANCELLED {
            let _ = child.kill(); let _ = child.wait();
            return Err("Đã hủy export.".into());
        }
        if let Some(status) = child.try_wait().map_err(|error| format!("Không thể kiểm tra FFmpeg trích audio: {error}"))? {
            if !status.success() || !input_wav.is_file() || fs::metadata(&input_wav).map(|meta| meta.len()).unwrap_or(0) <= 44 {
                return Err("FFmpeg không trích được audio stereo để tách thoại.".into());
            }
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let partial = render_dir.join("dialogue-speech.partial.wav");
    let separation = crate::dialogue_service::separate(
        app, &input_wav, &partial,
        || state.load(Ordering::SeqCst) == JOB_CANCELLED,
        |message| emit_event(app, event(project_id, job_id, "running", 0.02, started.elapsed().as_secs_f64(), duration, None, None, message, Vec::new())),
    );
    let _ = fs::remove_file(input_wav);
    if let Err(error) = separation { let _ = fs::remove_file(partial); return Err(error); }
    if let Err(e)=crate::audio::signal::validate_stem(&partial,duration) {let _=fs::remove_file(&partial);return Err(e);}
    fs::rename(&partial, &cached).map_err(|error| format!("Không thể lưu stem thoại trong cache project: {error}"))?;
    Ok(Some(cached))
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| format!("Không thể đọc video để tạo cache audio: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = std::io::Read::read(&mut file, &mut buffer).map_err(|error| format!("Không thể tính mã video: {error}"))?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn master_audio_filter(index:usize,expected:bool)->String {
    if expected {format!("[{index}:a:0]anull[aout];")} else {String::new()}
}

#[cfg(test)]
fn build_filtergraph(
    store: &ProjectStore,
    project_id: &str,
    transcript: &Transcript,
    spec: &RenderSpecification,
    options: &ExportOptions,
    ass_path: &Path,
    dialogue_stem: Option<&Path>,
    ai_patches: &[crate::text_removal_service::AiTextRemovalPatch],
) -> Result<(String, Vec<(PathBuf, f64, f64, String)>, Vec<PathBuf>, bool, PathBuf), String> {
    let voices = if options.include_dubbing {crate::audio::timeline::voices(store,project_id,transcript,true,spec.duration)?}else{Vec::new()};
    let logos: Vec<PathBuf> = if options.apply_logos {
        spec.logos
            .iter()
            .filter(|logo| logo.enabled)
            .map(|logo| resolve_project_file(store, project_id, &logo.asset_path, "media/overlays"))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    let audio_expected = spec.audio.include_original || !voices.is_empty();
    let source=resolve_video(store,project_id)?;
    let master=ass_path.with_extension("master.wav");
    crate::audio::mixer::render(if spec.audio.include_original {Some(&source)}else{None},dialogue_stem,voices,
        crate::audio::mixer::Mix::export(&spec.audio,Default::default()),spec.duration,&master,||false,|_|{})?;
    let mut graph=build_video_filtergraph(spec,options,ass_path,0,ai_patches);
    graph.push_str(&master_audio_filter(1+logos.len()+ai_patches.len(),audio_expected));
    Ok((graph,Vec::new(),logos,audio_expected,master))
}

/// Composite order is deliberately blur → translated ASS → logo. Keep the subtitle asset
/// downstream of every blur node so the translated glyphs stay crisp.
fn build_video_filtergraph(
    spec: &RenderSpecification,
    options: &ExportOptions,
    ass_path: &Path,
    dub_count: usize,
    ai_patches: &[crate::text_removal_service::AiTextRemovalPatch],
) -> String {
    let mut graph = String::new();
    let mut current = "0:v:0".to_string();
    let mut stage = 0usize;
    let logo_input_count = spec.logos.iter().filter(|logo| logo.enabled && options.apply_logos).count();
    for (index, patch) in ai_patches.iter().enumerate() {
        let output = format!("vclean{index}");
        let input_index = 1 + dub_count + logo_input_count + index;
        graph.push_str(&format!(
            "[{current}][{input_index}:v]overlay={}:{}:eof_action=pass:enable='between(t,{:.6},{:.6})'[{output}];",
            patch.rect.x, patch.rect.y, patch.start_seconds, patch.end_seconds
        ));
        current = output;
    }
    for region in spec.blur_regions.iter().filter(|region| region.enabled && region.effect == BlurEffect::Blur) {
        let rect = even_rect(region.rect, spec.width, spec.height);
        let base = format!("vbase{stage}");
        let source = format!("vsrc{stage}");
        let original_source = format!("vorigsrc{stage}");
        let blur_source = format!("vblursrc{stage}");
        let original = format!("vorig{stage}");
        let blurred = format!("vblur{stage}");
        let merged = format!("vmerged{stage}");
        let output = format!("vblurout{stage}");
        // Match the preview's CSS Gaussian blur. boxblur's radius is not equivalent
        // to CSS blur() sigma, so using the same strength value made export visibly
        // weaker and could leave hard subtitle glyphs readable. gblur uses the
        // source-pixel strength directly and is not capped by half the crop height.
        let blur_sigma = f64::from(crate::render::graph::blur_sigma(region.strength_px));
        let noise = if region.pixel_noise > 0 { format!(",noise=alls={}:allf=t+u", region.pixel_noise) } else { String::new() };
        let feather_px = crate::render::graph::feather_px(rect, region.strength_px, region.edge_blend);
        if feather_px > 0.0 {
            // Edge softness only mixes original pixels into a narrow outer feather.
            // At maximum strength remove that mix entirely so hard subtitles cannot
            // remain readable near the region boundary.
            let feather = (feather_px.round() as u32).max(1);
            let merge_original = format!("vmergeorig{stage}");
            let mask_base = format!("vmaskbase{stage}");
            let mask = format!("vmask{stage}");
            graph.push_str(&format!("[{current}]split=2[{base}][{source}];[{source}]split=2[{original_source}][{blur_source}];[{original_source}]crop={}:{}:{}:{},format=yuv420p[{original}];[{blur_source}]crop={}:{}:{}:{},format=yuv420p,gblur=sigma={:.1}:steps=2{noise}[{blurred}];", rect.width, rect.height, rect.x, rect.y, rect.width, rect.height, rect.x, rect.y, blur_sigma));
            graph.push_str(&format!("[{original}]split=2[{merge_original}][{mask_base}];[{mask_base}]format=gray,geq=lum='clip(min(min(X,W-X-1),min(Y,H-Y-1))*255/{feather},0,255)'[{mask}];[{merge_original}][{blurred}][{mask}]maskedmerge[{merged}];"));
        } else {
            graph.push_str(&format!("[{current}]split=2[{base}][{blur_source}];[{blur_source}]crop={}:{}:{}:{},format=yuv420p,gblur=sigma={:.1}:steps=2{noise}[{merged}];", rect.width, rect.height, rect.x, rect.y, blur_sigma));
        }
        graph.push_str(&format!(
            "[{base}][{merged}]overlay={}:{}:enable='{}':eof_action=pass[{output}];",
            rect.x,
            rect.y,
            enable_expr(region.start_ms, region.end_ms, spec.duration)
        ));
        current = output;
        stage += 1;
    }
    if spec.subtitle.enabled || spec.text_overlays.iter().any(|overlay| overlay.enabled && !overlay.text.trim().is_empty()) {
        let next = "vsubtitle";
        graph.push_str(&format!(
            "[{current}]subtitles=filename='{}':original_size={}x{}[{next}];",
            filter_path(ass_path),
            spec.width,
            spec.height
        ));
        current = next.to_string();
    } else {
        graph.push_str(&format!("[{current}]null[vsubtitle];"));
        current = "vsubtitle".into();
    }
    for (index, logo) in spec
        .logos
        .iter()
        .filter(|logo| logo.enabled && options.apply_logos)
        .enumerate()
    {
        let rect = even_rect(logo.rect, spec.width, spec.height);
        let input_index = 1 + dub_count + index;
        let layer = format!("logo{index}");
        let next = format!("vlogo{index}");
        if logo.maintain_aspect_ratio {
            graph.push_str(&format!("[{input_index}:v]format=rgba,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black@0,colorchannelmixer=aa={:.4}[{layer}];", rect.width, rect.height, rect.width, rect.height, logo.opacity));
        } else {
            graph.push_str(&format!(
                "[{input_index}:v]format=rgba,scale={}:{},colorchannelmixer=aa={:.4}[{layer}];",
                rect.width, rect.height, logo.opacity
            ));
        }
        graph.push_str(&format!(
            "[{current}][{layer}]overlay={}:{}:eof_action=pass:shortest=1[{next}];",
            rect.x, rect.y
        ));
        current = next;
    }
    graph.push_str(&format!("[{current}]null[vout];"));
    graph
}

/// Native video is already encoded. FFmpeg only composes audio and muxes;
/// no video filter, rawvideo pipe, video decode or second encoding pass.
fn build_native_mux_command(source:&Path,video:&Path,dubs:&[(PathBuf,f64,f64,String)],stem:Option<&Path>,filter:&Path,
 output:&Path,spec:&RenderSpecification,audio_expected:bool)->Result<Command,String>{
    let mut c=crate::runtime_paths::ffmpeg_command()?;
    c.args(["-hide_banner","-loglevel","error","-y","-i"]).arg(source);
    for (path,_,_,_) in dubs{c.arg("-i").arg(path);}
    if let Some(path)=stem{c.arg("-i").arg(path);}
    let video_index=1+dubs.len()+usize::from(stem.is_some());
    c.arg("-i").arg(video);
    if audio_expected{c.arg("-filter_complex_script").arg(filter).args(["-map","[aout]"]);}else{c.arg("-an");}
    c.arg("-map").arg(format!("{video_index}:v:0")).args(["-c:v","copy"]);
    if spec.output.video_codec==crate::hardware_acceleration::VideoCodec::Hevc{c.args(["-tag:v","hvc1"]);}
    c.args(["-c:a","aac","-b:a","192k","-movflags","+faststart","-shortest","-progress","pipe:1","-nostats"])
        .arg(output).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console_window(&mut c);Ok(c)
}

fn build_ffmpeg_command(
    mut command: Command,
    source: &Path,
    dubs: &[(PathBuf, f64, f64, String)],
    logos: &[PathBuf],
    ai_patches: &[crate::text_removal_service::AiTextRemovalPatch],
    dialogue_stem: Option<&Path>,
    filter_script: &Path,
    output: &Path,
    spec: &RenderSpecification,
    audio_expected: bool,
    raw_video_fps: Option<f64>,
) -> Result<Command, String> {
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source);
    for (path, _, _, _) in dubs {
        command.arg("-i").arg(path);
    }
    for path in logos {
        command.args(["-loop", "1", "-i"]).arg(path);
    }
    for patch in ai_patches {
        command
            .args(["-itsoffset"])
            .arg(format!("{:.6}", patch.start_seconds));
        if patch.image_pattern.extension().is_none_or(|e|e!="nut"){
            command.arg("-framerate").arg(format!("{:.6}",patch.fps));
        }
        command.arg("-i").arg(&patch.image_pattern);
    }
    if let Some(path) = dialogue_stem { command.arg("-i").arg(path); }
    if let Some(fps) = raw_video_fps {
        command.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-video_size"]).arg(format!("{}x{}", spec.width, spec.height))
            .arg("-framerate").arg(format!("{fps:.12}")).args(["-i", "pipe:0"]);
    }
    command
        .args(["-filter_complex_script"])
        .arg(filter_script)
        .args(["-map", "[vout]"]);
    if audio_expected {
        command.args(["-map", "[aout]"]);
    } else {
        command.arg("-an");
    }
    command.args(hardware_acceleration::encoder_arguments(
        spec.output.video_codec, spec.output.encoder, &spec.quality,
        spec.video_bitrate_kbps, EncodingProfile::Export,
    )?);
    command
        .arg("-c:a")
        .arg(&spec.output.audio_codec)
        .args([
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            "-shortest",
            "-progress",
            "pipe:1",
            "-nostats",
        ])
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);
    Ok(command)
}


pub(crate) fn register_auxiliary_job(
    app: &AppHandle,
    jobs: &RenderJobs,
    job_id: String,
    project_id: String,
    project_name: String,
    job_type: &str,
    resources: &[&str],
    state: Arc<AtomicU8>,
) -> Result<(), String> {
    let _guard = jobs
        .start_lock
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái tác vụ video.".to_string())?;
    if jobs
        .deleting
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái tác vụ video.".to_string())?
        .contains(&project_id)
    {
        return Err("Dự án đang được xóa.".into());
    }
    if has_active_project_job(jobs, &project_id) {
        return Err("Một tác vụ video khác đang chạy trong dự án này.".into());
    }
    jobs.inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái tác vụ video.".to_string())?
        .insert(
            job_id.clone(),
            RenderJobEntry {
                project_id: project_id.clone(),
                state,
            },
        );
    crate::job_service::register_job(
        app,
        job_id,
        project_id,
        project_name,
        job_type,
        resources,
        None,
    );
    Ok(())
}

pub(crate) fn finish_auxiliary_job(jobs: &RenderJobs, job_id: &str) {
    if let Ok(mut active) = jobs.inner.lock() {
        active.remove(job_id);
    }
}


fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn even_rect(rect: render_spec::PixelRect, width: u32, height: u32) -> render_spec::PixelRect {
    let x = rect.x / 2 * 2;
    let y = rect.y / 2 * 2;
    let max_w = width.saturating_sub(x);
    let max_h = height.saturating_sub(y);
    let w = rect.width.min(max_w) / 2 * 2;
    let h = rect.height.min(max_h) / 2 * 2;
    render_spec::PixelRect {
        x,
        y,
        width: w.max(2),
        height: h.max(2),
    }
}
fn enable_expr(start_ms: u64, end_ms: Option<u64>, duration: f64) -> String {
    format!(
        "between(t,{:.3},{:.3})",
        start_ms as f64 / 1000.0,
        end_ms
            .map(|value| value as f64 / 1000.0)
            .unwrap_or(duration + 0.1)
    )
}
fn filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn unique_output_path(directory: &Path, name: &str) -> PathBuf {
    let base = name
        .trim()
        .strip_suffix_case_insensitive(".mp4")
        .unwrap_or(name.trim());
    for index in 1..10000 {
        let file = if index == 1 {
            format!("{base}.mp4")
        } else {
            format!("{base}_{index}.mp4")
        };
        if !directory.join(&file).exists() {
            return directory.join(file);
        }
    }
    directory.join(format!("{}_{}.mp4", base, Uuid::new_v4()))
}

fn validate_output_file_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    let base = trimmed
        .strip_suffix_case_insensitive(".mp4")
        .unwrap_or(trimmed);
    if base.is_empty()
        || base == "."
        || base == ".."
        || base.chars().count() > 180
        || base.chars().any(|ch| {
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        return Err("Tên file export không hợp lệ. Hãy dùng tối đa 180 ký tự và không nhập ký tự \\/:*?\"<>|.".to_string());
    }
    let device_name = base.split('.').next().unwrap_or(base).to_ascii_uppercase();
    if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device_name.len() == 4
            && (device_name.starts_with("COM") || device_name.starts_with("LPT"))
            && device_name.as_bytes()[3].is_ascii_digit()
            && matches!(device_name.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("Tên file export này được Windows dành riêng. Hãy chọn tên khác.".to_string());
    }
    Ok(())
}

trait StripSuffixCaseInsensitive {
    fn strip_suffix_case_insensitive(&self, suffix: &str) -> Option<&str>;
}

impl StripSuffixCaseInsensitive for str {
    fn strip_suffix_case_insensitive(&self, suffix: &str) -> Option<&str> {
        self.get(..self.len().checked_sub(suffix.len())?)
            .filter(|prefix| self[prefix.len()..].eq_ignore_ascii_case(suffix))
    }
}

fn validate_output(
    path: &Path,
    width: u32,
    height: u32,
    expected_duration: f64,
    audio_expected: bool,
) -> Result<(), String> {
    if !path.is_file()
        || fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            < 2048
    {
        return Err("FFmpeg không tạo được video đầu ra hợp lệ.".into());
    }
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
        .output()
        .map_err(|error| format!("Không thể xác minh export bằng FFprobe: {error}"))?;
    if !output.status.success() {
        return Err("FFprobe không thể đọc video export.".into());
    }
    let json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Kết quả FFprobe export không hợp lệ: {error}"))?;
    let streams = json["streams"]
        .as_array()
        .ok_or("Video export không có stream.")?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .ok_or("Video export thiếu video stream.")?;
    if video["width"].as_u64() != Some(width as u64)
        || video["height"].as_u64() != Some(height as u64)
    {
        return Err("Độ phân giải video export không khớp với nguồn.".into());
    }
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            video["duration"]
                .as_str()
                .and_then(|value| value.parse::<f64>().ok())
        })
        .ok_or("Không đọc được thời lượng video export.")?;
    if duration <= 0.0 || (duration - expected_duration).abs() > expected_duration.max(2.0) * 0.03 {
        return Err("Thời lượng video export không hợp lệ hoặc lệch quá mức so với nguồn.".into());
    }
    let audio = streams.iter().any(|stream| stream["codec_type"] == "audio");
    if audio_expected && !audio {
        return Err("Video export thiếu audio stream theo cấu hình.".into());
    }
    Ok(())
}

fn event(
    project_id: &str,
    job_id: &str,
    status: &str,
    progress: f64,
    elapsed: f64,
    total: f64,
    speed: Option<f64>,
    output: Option<String>,
    message: String,
    warnings: Vec<render_spec::SubtitleLayoutWarning>,
) -> ExportJobEvent {
    ExportJobEvent {
        project_id: project_id.into(),
        job_id: job_id.into(),
        status: status.into(),
        progress,
        elapsed_seconds: elapsed,
        total_seconds: total,
        speed,
        output,
        message,
        layout_warnings: warnings,
    }
}

fn emit_event(app: &AppHandle, event: ExportJobEvent) {
    crate::job_service::sync_app_event(app, "export", &event);
    let _ = app.emit("export-progress", event);
}

fn write_history(
    store: &ProjectStore,
    project_id: &str,
    item: ExportHistoryEntry,
) -> Result<(), String> {
    let root = store.project_directory(project_id)?;
    let path = root.join("exports").join("history.json");
    let mut items = if path.exists() {
        serde_json::from_slice::<Vec<ExportHistoryEntry>>(
            &fs::read(&path).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("Export history lỗi: {error}"))?
    } else {
        Vec::new()
    };
    items.insert(0, item);
    items.truncate(100);
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Thư mục history không hợp lệ.")?)
            .map_err(|error| error.to_string())?;
    temp.write_all(&serde_json::to_vec_pretty(&items).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    temp.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temp.persist(&path)
        .map_err(|error| format!("Không thể lưu export history: {}", error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_export_uses_a_size_efficient_h264_profile() {
        let args = hardware_acceleration::encoder_arguments(crate::hardware_acceleration::VideoCodec::H264, VideoEncoder::Cpu, "balanced", None, EncodingProfile::Export).unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-preset", "medium"]));
        assert!(args.windows(2).any(|pair| pair == ["-crf", "23"]));
    }

    #[test]
    fn export_directory_is_resolved_inside_the_project() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Export Path").unwrap();
        let export_dir = export_directory(&store, &project.id).unwrap();
        assert!(export_dir.is_dir());
        let canonical_project =
            fs::canonicalize(store.project_directory(&project.id).unwrap()).unwrap();
        assert!(export_dir.starts_with(canonical_project));
    }

    #[test]
    fn output_filename_is_unique_and_never_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("My Project.mp4"), b"x").unwrap();
        assert_eq!(
            unique_output_path(temp.path(), "My Project"),
            temp.path().join("My Project_2.mp4")
        );
    }

    #[test]
    fn selected_export_directory_is_canonical_and_writable() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Export Path").unwrap();
        let chosen = temp.path().join("user-selected");
        fs::create_dir(&chosen).unwrap();
        let resolved =
            resolve_export_directory(&store, &project.id, Some(chosen.to_string_lossy().as_ref()))
                .unwrap();
        assert_eq!(resolved, fs::canonicalize(chosen).unwrap());
        assert!(resolve_export_directory(&store, &project.id, Some("relative-folder")).is_err());
    }
    #[test]
    fn filter_paths_escape_filter_delimiters_not_shell() {
        let escaped = filter_path(Path::new("C:\\Users\\A B\\x.ass"));
        assert!(escaped.contains("C\\:"));
        assert!(escaped.contains("A B"));
    }
    #[test]
    fn even_crop_is_bounded_for_portrait_and_square_video() {
        let rect = even_rect(
            render_spec::PixelRect {
                x: 85,
                y: 1497,
                width: 909,
                height: 269,
            },
            1080,
            1920,
        );
        assert_eq!(
            (rect.x % 2, rect.y % 2, rect.width % 2, rect.height % 2),
            (0, 0, 0, 0)
        );
        assert!(rect.x + rect.width <= 1080 && rect.y + rect.height <= 1920);
    }

    #[test]
    fn cancel_only_succeeds_before_output_is_committed() {
        let jobs = RenderJobs::default();
        jobs.inner.lock().unwrap().insert(
            "running".into(),
            RenderJobEntry {
                project_id: "project".into(),
                state: Arc::new(AtomicU8::new(JOB_RUNNING)),
            },
        );
        assert!(cancel_export(&jobs, "running").unwrap());
        assert!(!cancel_export(&jobs, "running").unwrap());
        jobs.inner.lock().unwrap().insert(
            "committed".into(),
            RenderJobEntry {
                project_id: "project".into(),
                state: Arc::new(AtomicU8::new(JOB_COMMITTED)),
            },
        );
        assert!(!cancel_export(&jobs, "committed").unwrap());
    }

    #[test]
    fn rejects_unbounded_audio_values() {
        let mut options = ExportOptions::default();
        options.original_volume = 1.01;
        assert!(validate_export_options(&options).is_err());
    }

    #[test]
    fn ffmpeg_renders_blur_then_region_subtitles_then_logo_and_mixes_delayed_dub() {
        verify_render(crate::hardware_acceleration::VideoCodec::H264, VideoEncoder::Cpu);
    }

    #[test]
    #[ignore = "Requires FFmpeg and performs a full export on each usable backend"]
    fn real_export_with_effects_on_available_backends() {
        let caps = hardware_acceleration::capabilities(false).unwrap();
        for codec in [crate::hardware_acceleration::VideoCodec::H264, crate::hardware_acceleration::VideoCodec::Hevc] {
            for backend in hardware_acceleration::DEFAULT_PRIORITY {
                if caps.encoders.iter().any(|item| item.id == *backend && item.supports(codec)) {
                    verify_render(codec, *backend);
                    println!("export effects {backend:?}/{codec:?}: blur/subtitle/logo/audio verified");
                }
            }
        }
    }

    fn verify_render(codec: crate::hardware_acceleration::VideoCodec, backend: VideoEncoder) {
        verify_render_path(codec, backend, false);
    }

    #[test]
    #[ignore="Requires a compatible graphics adapter and FFmpeg; exports real GPU visuals and delayed mixed audio"]
    fn gpu_exports_real_effects_and_audio_on_cpu_and_intel_qsv() {
        if crate::render::renderer::Renderer::new(crate::render::graph::RenderScene::empty(320,240), &[]).is_err() { eprintln!("Skipping GPU export: no adapter"); return; }
        verify_render_path(crate::hardware_acceleration::VideoCodec::H264, VideoEncoder::Cpu, true);
        verify_render_resolution_audio(crate::hardware_acceleration::VideoCodec::H264, VideoEncoder::Cpu, true, 320, 240, false);
        let capabilities=hardware_acceleration::capabilities(false).unwrap();
        if capabilities.encoders.iter().any(|c|c.id==VideoEncoder::Intel&&c.supports(crate::hardware_acceleration::VideoCodec::H264)) {
            verify_render_path(crate::hardware_acceleration::VideoCodec::H264, VideoEncoder::Intel, true);
        }
    }

    fn verify_render_path(codec: crate::hardware_acceleration::VideoCodec, backend: VideoEncoder, gpu:bool) {
        verify_render_resolution(codec, backend, gpu, 320, 240);
    }

    #[test]
    #[ignore = "Real 1080p visual+audio export benchmark, GPU adapter and FFmpeg required"]
    fn gpu_1080p_export_comparison() {
        use crate::hardware_acceleration::VideoCodec;
        if crate::render::renderer::Renderer::new(crate::render::graph::RenderScene::empty(1920,1080), &[]).is_err() {
            eprintln!("Skipping 1080p GPU export: no compatible adapter"); return;
        }
        verify_render_resolution(VideoCodec::H264, VideoEncoder::Cpu, false, 1920, 1080);
        verify_render_resolution(VideoCodec::H264, VideoEncoder::Cpu, true, 1920, 1080);
        let capabilities = hardware_acceleration::capabilities(false).unwrap();
        if capabilities.encoders.iter().any(|c|c.id==VideoEncoder::Intel && c.supports(VideoCodec::H264)) {
            verify_render_resolution(VideoCodec::H264, VideoEncoder::Intel, true, 1920, 1080);
        }
    }

    fn verify_render_resolution(codec: crate::hardware_acceleration::VideoCodec, backend: VideoEncoder, gpu:bool, width:u32, height:u32) {
        verify_render_resolution_audio(codec, backend, gpu, width, height, true);
    }
    fn verify_render_resolution_audio(codec: crate::hardware_acceleration::VideoCodec, backend: VideoEncoder, gpu:bool, width:u32, height:u32, original_audio:bool) {
        verify_render_resolution_audio_path(codec,backend,gpu,false,width,height,original_audio);
    }
    #[cfg(windows)]
    #[test]
    #[ignore="Opt-in real MF decode/wgpu/hardware encode, effects, Unicode paths and audio mux"]
    fn native_gpu_exports_effects_audio_and_1080p() {
        use crate::hardware_acceleration::VideoCodec;
        for codec in [VideoCodec::H264,VideoCodec::Hevc] {
            verify_render_resolution_audio_path(codec,VideoEncoder::Intel,false,true,320,240,true);
        }
        verify_render_resolution_audio_path(VideoCodec::H264,VideoEncoder::Intel,false,true,1920,1080,true);
        verify_render_resolution_audio_path(VideoCodec::H264,VideoEncoder::Intel,false,true,320,240,false);
    }
    fn verify_render_resolution_audio_path(codec: crate::hardware_acceleration::VideoCodec, backend: VideoEncoder, gpu:bool, native:bool, width:u32, height:u32, original_audio:bool) {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("Skipping integration render: FFmpeg is not installed");
            return;
        }
        let temp = tempfile::Builder::new()
            .prefix("字幕-render-integration-")
            .tempdir()
            .unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let mut project = store.create("Export integration").unwrap();
        let project_root = store.project_directory(&project.id).unwrap();
        let source = project_root.join("media/source/source.mp4");
        let generated = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=gray:s={width}x{height}:r=20:d=1"),
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            generated.status.success(),
            "fixture creation failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let metadata =
            crate::video_service::probe_video(&source, fs::metadata(&source).unwrap().len())
                .unwrap();
        project.source.video_path = Some("media/source/source.mp4".into());
        project.source.duration = metadata.duration;
        project.source.width = metadata.width;
        project.source.height = metadata.height;
        project.source.fps = metadata.fps;
        project.source.video_codec = metadata.video_codec;
        project.source.audio_codec = metadata.audio_codec;
        project.source.file_size = Some(metadata.file_size);
        project.source.audio_streams = metadata.audio_streams;
        project.source.subtitle_tracks = metadata.subtitle_tracks;
        project
            .blur_regions
            .push(crate::project_service::BlurRegion {
                id: Uuid::new_v4().to_string(),
                x: 0.1,
                y: 0.68,
                width: 0.8,
                height: 0.24,
                start_ms: 0,
                end_ms: None,
                enabled: true,
                strength: 24.0,
                effect: Default::default(),
                role: BlurRegionRole::Subtitle,
                edge_blend: 12,
                pixel_noise: 10,
                text_removal_scan_fps: 2,
                text_removal_mode: crate::project_service::TextRemovalMode::OcrGuided,
            });

        let logo = project_root.join("media/overlays/logo.png");
        let generated_logo = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=48x24:d=1",
                "-frames:v",
                "1",
                "-vf",
                "format=rgba",
            ])
            .arg(&logo)
            .output()
            .unwrap();
        assert!(generated_logo.status.success(), "logo fixture failed");
        project.logos.push(crate::project_service::ProjectLogo {
            id: Uuid::new_v4().to_string(),
            relative_path: "media/overlays/logo.png".into(),
            original_name: "logo.png".into(),
            x: 0.78,
            y: 0.05,
            width: 0.16,
            height: 0.12,
            enabled: true,
            opacity: 0.9,
            maintain_aspect_ratio: true,
            merge_only: false,
        });
        project.dubbing.enabled = true;
        project.dubbing.ducking_enabled = true;
        project.dubbing.ducking_amount = 0.75;
        project = store.update_project(&project).unwrap();

        let dub = project_root.join("audio/tts/clip.wav");
        let generated_dub = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=660:sample_rate=48000:duration=0.45",
                "-ac",
                "2",
            ])
            .arg(&dub)
            .output()
            .unwrap();
        assert!(generated_dub.status.success(), "dub fixture failed");
        let transcript = Transcript {
            schema_version: 1,
            project_id: project.id.clone(),
            source_type: crate::project_service::TranscriptSourceType::Asr,
            source_track: None,
            language: Some("en".into()),
            video_checksum: project.source.checksum_sha256.clone(),
            segments: vec![crate::transcript_service::TranscriptSegment {
                id: "seg1".into(),
                start: 0.2,
                end: 0.8,
                source: "Hello".into(),
                translated: "Xin chào mọi người".into(),
                source_revision: 0,
                translated_revision: Some(0),
                translation_meta: None,
                speaker_id: None,
                source_type: crate::project_service::TranscriptSourceType::Asr,
                translation_status:
                    crate::transcript_service::TranscriptTranslationStatus::Translated,
                tts: Some(crate::transcript_service::TranscriptTts {
                    audio_path: Some("audio/tts/clip.wav".into()),
                    duration: Some(0.45),
                    fingerprint: None,
                    status: Some(TranscriptTtsStatus::Ready),
                    voice: None,
                    speed: None,
                    text_hash: None,
                    generated_at: None,
                    warning: None,
                    provider: None,
                    model_id: None,
                    model_revision: None,
                }),
            }],
        };
        crate::transcript_service::save_transcript(&store, &project.id, transcript.clone())
            .unwrap();
        let options = ExportOptions {
            encoder: backend,
            video_codec: codec,
            include_original_audio: original_audio,
            include_dubbing: true,
            ducking: true,
            ..ExportOptions::default()
        };
        let project = store.get(&project.id).unwrap();
        let spec = render_spec::build_render_specification(
            &project,
            &transcript,
            project.source.duration.unwrap(),
            &options,
        )
        .unwrap();
        let ass = project_root.join("cache/translated.ass");
        fs::write(&ass, render_spec::generate_ass(&spec).unwrap()).unwrap();
        let (graph, dubs, logos, expected_audio, master) =
            build_filtergraph(&store, &project.id, &transcript, &spec, &options, &ass, None, &[]).unwrap();
        assert!(
            graph.find("gblur=").unwrap() < graph.find("subtitles=").unwrap(),
            "blur must precede translated subtitles"
        );
        assert!(
            graph.find("subtitles=").unwrap() < graph.rfind(&format!("overlay={}", even_rect(spec.logos[0].rect, spec.width, spec.height).x)).unwrap(),
            "logo overlay must follow translated subtitles"
        );
        assert!(
            graph.contains("maskedmerge"),
            "edge blend must feather the blur edges"
        );
        assert!(
            graph.contains("noise=alls=10:allf=t+u"),
            "pixel noise should be applied to the blurred crop"
        );
        assert!(graph.contains("anull[aout]"));
        assert!(!graph.contains("amix") && !graph.contains("adelay"));
        assert_eq!(dubs.len(), 0);
        assert_eq!(crate::audio::signal::inspect(&master).unwrap().duration,1.0);
        assert_eq!(logos.len(), 1);
        assert!(expected_audio);
        let filter = project_root.join("cache/filtergraph.txt");
        fs::write(&filter, &graph).unwrap();
        let output_path = project_root.join("exports/render-integration.mp4");
        let benchmark = Instant::now();
        if native {
            #[cfg(windows)] {
                let temp_video=project_root.join("cache/native.mp4");
                crate::media_gpu::pipeline::preflight(&source).unwrap();
                let mut pipeline=crate::media_gpu::windows::NativePipeline::new(&source,&temp_video,
                    crate::render::graph::RenderScene::from_spec(&spec),&logos,codec,4_000_000,
                    crate::media_gpu::capability::vendor(backend)).unwrap();
                let state=AtomicU8::new(JOB_RUNNING);
                let stats=pipeline.run(&state,|_,_|{}).unwrap();
                assert_eq!(stats.frames,20);assert_eq!(stats.upload_bytes,0);assert_eq!(stats.readback_bytes,0);
                drop(pipeline);
                fs::write(&filter,master_audio_filter(1,true)).unwrap();
                let mut command=build_native_mux_command(&source,&temp_video,&dubs,Some(&master),&filter,&output_path,&spec,true).unwrap();
                let rendered=command.output().unwrap();
                assert!(rendered.status.success(),"native mux: {}",String::from_utf8_lossy(&rendered.stderr));
                let pcm=crate::runtime_paths::ffmpeg_command().unwrap().args(["-v","error","-i"]).arg(&output_path)
                    .args(["-vn","-ac","1","-ar","48000","-f","f32le","pipe:1"]).output().unwrap();
                assert!(pcm.status.success());
                let samples:Vec<f64>=pcm.stdout.chunks_exact(4).map(|s|f32::from_le_bytes(s.try_into().unwrap()) as f64).collect();
                let amplitude=|frequency:f64,start:f64,end:f64|{
                    let a=(start*48000.0) as usize;let b=(end*48000.0) as usize;
                    let(mut sin,mut cos)=(0.0,0.0);
                    for (i,s) in samples[a..b].iter().enumerate(){let phase=(a+i) as f64*frequency*std::f64::consts::TAU/48000.0;sin+=s*phase.sin();cos+=s*phase.cos();}
                    2.0*(sin*sin+cos*cos).sqrt()/(b-a) as f64
                };
                let dub_before=amplitude(660.0,0.08,0.16);let dub_inside=amplitude(660.0,0.30,0.40);let dub_after=amplitude(660.0,0.80,0.90);
                assert!(dub_inside>0.02 && dub_before<dub_inside*0.15 && dub_after<dub_inside*0.15,"dub timing must be 0.2..0.65s");
                if original_audio {
                    let before=amplitude(440.0,0.08,0.16);let during=amplitude(440.0,0.30,0.40);let after=amplitude(440.0,0.80,0.90);
                    assert!(before>0.02 && during<before*0.6 && after>before*0.7,"original ducking must align with dub audio");
                }
                println!("Native Tier B {codec:?}, {width}x{height}: {:.3}s; {stats:?}",benchmark.elapsed().as_secs_f64());
                if let Ok(folder)=std::env::var("JC_GPU_TEST_ARTIFACTS") {
                    fs::create_dir_all(&folder).unwrap();
                    fs::copy(&output_path,Path::new(&folder).join(format!("native-{width}x{height}-{codec:?}-{}.mp4",if original_audio{"mixed"}else{"dubbing"}))).unwrap();
                }
            }
        } else if gpu {
            let mut renderer=crate::render::renderer::Renderer::new(crate::render::graph::RenderScene::from_spec(&spec), &logos).unwrap();
            let fps=crate::render::stream::compatible_input(&source).unwrap();
            let gpu_graph=format!("[{}:v]null[vout];{}",1+dubs.len()+1,master_audio_filter(1,true));
            assert!(!gpu_graph.contains("gblur")&&!gpu_graph.contains("subtitles=")&&!gpu_graph.contains("overlay="));
            fs::write(&filter,gpu_graph).unwrap();
            let command=build_ffmpeg_command(crate::runtime_paths::ffmpeg_command().unwrap(),&source,&dubs,&[],&[],Some(&master),&filter,&output_path,&spec,true,Some(fps)).unwrap();
            let state=Arc::new(AtomicU8::new(JOB_RUNNING));
            let stats=crate::render::stream::run(&mut renderer,crate::render::stream::decoder_command(&source).unwrap(),command,fps,&[],&state,|_,_|{}).unwrap();
            assert_eq!(stats.frames,20);assert!(stats.seconds>0.0);
            println!("wgpu export {backend:?} passed on {} / {} ({:.3}s, {:.1} fps stream)",renderer.adapter_name(),renderer.adapter_backend(),stats.seconds,stats.frames as f64/stats.seconds);
            let mut cancelled_command=build_ffmpeg_command(crate::runtime_paths::ffmpeg_command().unwrap(),&source,&dubs,&[],&[],Some(&master),&filter,&project_root.join("exports/cancelled.mp4"),&spec,true,Some(fps)).unwrap();
            cancelled_command.stdin(Stdio::piped());
            let mut slow=crate::runtime_paths::ffmpeg_command().unwrap();
            slow.args(["-hide_banner","-loglevel","info","-re","-i"]).arg(&source).args(["-map","0:v:0","-an","-vf","setpts=PTS-STARTPTS,showinfo","-fps_mode","passthrough","-pix_fmt","rgba","-f","rawvideo","pipe:1"]).stdout(Stdio::piped()).stderr(Stdio::piped());
            let state=Arc::new(AtomicU8::new(JOB_RUNNING));let cancel=state.clone();
            let trigger=thread::spawn(move||{thread::sleep(Duration::from_millis(150));cancel.store(JOB_CANCELLED,Ordering::SeqCst);});
            let before=Instant::now();
            let result=crate::render::stream::run(&mut renderer,slow,cancelled_command,fps,&[],&state,|_,_|{});
            trigger.join().unwrap();assert!(result.is_err());assert!(before.elapsed()<Duration::from_secs(5));
        } else {
        let rendered = build_ffmpeg_command(
            crate::runtime_paths::ffmpeg_command().unwrap(), &source, &dubs, &logos,
            &[], Some(&master), &filter, &output_path, &spec, expected_audio, None,
        ).unwrap().output().unwrap();
        assert!(
            rendered.status.success(),
            "FFmpeg render failed: {}\nGRAPH: {}",
            String::from_utf8_lossy(&rendered.stderr),
            graph
        );
        }
        if !gpu && !native { println!("legacy export {width}x{height} {backend:?}: {:.3}s", benchmark.elapsed().as_secs_f64()); }
        validate_output(
            &output_path,
            width,
            height,
            project.source.duration.unwrap(),
            true,
        )
        .unwrap();
        assert!(fs::metadata(&output_path).unwrap().len() > 2048);
        let metadata = crate::runtime_paths::ffprobe_command().unwrap().args(["-v","error","-select_streams","v:0","-show_entries","stream=codec_name","-of","json"]).arg(&output_path).output().unwrap();
        let json: Value = serde_json::from_slice(&metadata.stdout).unwrap();
        assert_eq!(json["streams"][0]["codec_name"],codec.name());
    }
}
