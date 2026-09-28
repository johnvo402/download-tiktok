use crate::project_service::BlurEffect;
use crate::render_spec::{PixelRect, RenderSpecification};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const MODEL_SIZE: u64 = 208_044_816;
const MODEL_SHA256: &str = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6";
const MODEL_URL: &str = "https://huggingface.co/edgetools/lama/resolve/d3f22ee032d32d9a68ac06dd3da40aba1f831f1d/lama_fp32.onnx";
const WORKER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRemovalEnvironment {
    pub model_installed: bool,
    pub model_bytes: u64,
    pub estimated_download_bytes: u64,
    pub ocr_ready: bool,
    pub ocr_model: String,
    pub ocr_cuda_available: bool,
    pub cuda_available: bool,
}

#[derive(Clone, Debug)]
pub struct AiTextRemovalPatch {
    pub image_pattern: PathBuf,
    pub rect: PixelRect,
    pub fps: f64,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRemovalPreviewEvent {
    pub project_id: String,
    pub job_id: String,
    pub status: String,
    pub progress: f64,
    pub message: String,
    pub output: Option<String>,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub region_id: String,
    pub fps: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerEvent {
    #[serde(rename = "type")]
    kind: String,
    progress: Option<f64>,
    message: Option<String>,
    masked_pixels: Option<u64>,
}

fn app_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("models"))
        .map_err(|error| format!("Không thể xác định thư mục mô hình: {error}"))
}

fn model_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_models_dir(app)?.join("text-removal").join("lama_fp32.onnx"))
}

fn model_valid(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == MODEL_SIZE)
}

pub fn environment(app: AppHandle) -> Result<TextRemovalEnvironment, String> {
    let settings = crate::settings_service::get_settings(&app)?;
    let model = settings.settings.asr.ocr_model.clone();
    let ocr = crate::asr_service::get_ocr_environment(app.clone(), model.clone());
    let ocr = ocr.ok();
    let path = model_path(&app)?;
    let model_installed = model_valid(&path);
    Ok(TextRemovalEnvironment {
        model_installed,
        model_bytes: if model_installed { MODEL_SIZE } else { 0 },
        estimated_download_bytes: MODEL_SIZE,
        ocr_ready: ocr.as_ref().is_some_and(|value| {
            value.worker_installed && value.ocr_installed && value.runtime_installed && value.model_cached
        }),
        ocr_model: model,
        ocr_cuda_available: ocr.as_ref().is_some_and(|value| value.cuda_available),
        cuda_available: ocr.as_ref().is_some_and(|value| value.cuda_available),
    })
}

pub fn install_model(app: AppHandle) -> Result<(), String> {
    let target = model_path(&app)?;
    if model_valid(&target) {
        return Ok(());
    }
    let directory = target.parent().ok_or("Thư mục mô hình không hợp lệ.")?;
    fs::create_dir_all(directory).map_err(|error| format!("Không thể tạo thư mục mô hình: {error}"))?;
    let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Thư mục mô hình AI không an toàn; đã dừng tải.".into());
    }
    let partial = directory.join("lama_fp32.onnx.incomplete");
    let _ = fs::remove_file(&partial);
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30 * 60))
        .user_agent("JC-AI-Desktop/1.0")
        .build()
        .map_err(|error| format!("Không thể khởi tạo tải mô hình: {error}"))?;
    let mut response = client.get(MODEL_URL).send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("Không tải được mô hình xóa chữ bằng AI: {error}"))?;
    if response.content_length().is_some_and(|length| length != MODEL_SIZE) {
        return Err("Kích thước mô hình tải về không hợp lệ.".into());
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(&partial)
        .map_err(|error| format!("Không thể tạo tệp tải mô hình: {error}"))?;
    let mut digest = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 256 * 1024];
    let mut last_emit = Instant::now();
    loop {
        let count = response.read(&mut buffer)
            .map_err(|error| format!("Tải mô hình bị gián đoạn: {error}"))?;
        if count == 0 { break; }
        file.write_all(&buffer[..count]).map_err(|error| format!("Không thể ghi mô hình xuống ổ đĩa: {error}"))?;
        digest.update(&buffer[..count]);
        downloaded += count as u64;
        if last_emit.elapsed() >= Duration::from_millis(250) || downloaded == MODEL_SIZE {
            let _ = app.emit("text-removal-model-progress", serde_json::json!({
                "downloadedBytes": downloaded,
                "totalBytes": MODEL_SIZE,
                "progress": (downloaded as f64 / MODEL_SIZE as f64).clamp(0.0, 1.0),
                "message": "Đang tải mô hình xóa chữ bằng AI…"
            }));
            last_emit = Instant::now();
        }
    }
    file.sync_all().map_err(|error| format!("Không thể hoàn tất ghi mô hình: {error}"))?;
    if downloaded != MODEL_SIZE || format!("{:x}", digest.finalize()) != MODEL_SHA256 {
        let _ = fs::remove_file(&partial);
        return Err("Mô hình tải về chưa đầy đủ hoặc không vượt qua kiểm tra toàn vẹn. Hãy thử lại.".into());
    }
    if target.exists() { fs::remove_file(&target).map_err(|error| format!("Không thể thay mô hình cũ: {error}"))?; }
    fs::rename(&partial, &target).map_err(|error| format!("Không thể cài mô hình: {error}"))?;
    let _ = app.emit("text-removal-model-progress", serde_json::json!({
        "downloadedBytes": MODEL_SIZE,
        "totalBytes": MODEL_SIZE,
        "progress": 1.0,
        "message": "Đã cài mô hình xóa chữ bằng AI"
    }));
    Ok(())
}

pub fn require_ready(app: &AppHandle) -> Result<(), String> {
    let status = environment(app.clone())?;
    if !status.model_installed {
        return Err("Chưa cài mô hình xóa chữ bằng AI. Mở thuộc tính vùng và tải mô hình khoảng 198 MiB.".into());
    }
    if !status.ocr_ready {
        return Err(format!("RapidOCR chưa sẵn sàng. Hãy cài model OCR {} trong Cài đặt → Nhận dạng lời thoại.", status.ocr_model));
    }
    Ok(())
}

pub fn prepare_export_patches<F>(
    app: &AppHandle,
    spec: &RenderSpecification,
    source: &Path,
    render_dir: &Path,
    state: &AtomicU8,
    mut progress: F,
) -> Result<Vec<AiTextRemovalPatch>, String>
where
    F: FnMut(f64, String),
{
    let regions: Vec<_> = spec.blur_regions.iter()
        .filter(|region| region.enabled && region.effect == BlurEffect::AiTextRemoval)
        .collect();
    if regions.is_empty() { return Ok(Vec::new()); }
    require_ready(app)?;
    let settings = crate::settings_service::get_settings(app)?;
    let ocr_model = settings.settings.asr.ocr_model;
    let ocr_device = settings.settings.asr.ocr_device;
    let cache_dir = app_models_dir(app)?;
    let model_path = model_path(app)?;
    let worker = crate::runtime_paths::ai_worker_dir()?;
    let frame_rate = spec.fps.filter(|fps| fps.is_finite() && (1.0..=240.0).contains(fps)).unwrap_or(30.0);
    let mut patches = Vec::with_capacity(regions.len());
    for (region_index, region) in regions.iter().enumerate() {
        if state.load(Ordering::SeqCst) == 2 { return Err("Đã hủy export.".into()); }
        let start = (region.start_ms as f64 / 1000.0).clamp(0.0, spec.duration);
        let end = (region.end_ms.map(|value| value as f64 / 1000.0).unwrap_or(spec.duration)).clamp(start, spec.duration);
        if end <= start { continue; }
        let work_dir = render_dir.join(format!("text-removal-{}", region_index + 1));
        let frames_dir = work_dir.join("frames");
        let masks_dir = work_dir.join("masks");
        let output_dir = work_dir.join("clean");
        fs::create_dir_all(&frames_dir).map_err(|error| format!("Không thể tạo frame tạm để xóa chữ: {error}"))?;
        let rect = region.rect;
        let crop = format!("crop={}:{}:{}:{},fps={frame_rate:.6}", rect.width, rect.height, rect.x, rect.y);
        let pattern = frames_dir.join("frame_%08d.jpg");
        progress(0.025 + 0.12 * region_index as f64 / regions.len() as f64,
            format!("Đang trích xuất vùng chữ · vùng {}/{}", region_index + 1, regions.len()));
        let mut extraction = crate::runtime_paths::ffmpeg_command()?
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
            .arg(format!("{start:.6}"))
            .arg("-i").arg(source)
            .args(["-t"]).arg(format!("{:.6}", end - start))
            .args(["-vf", &crop, "-vsync", "0", "-q:v", "2", "-start_number", "1"])
            .arg(&pattern)
            .stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map_err(|error| format!("Không thể trích xuất frame vùng chữ: {error}"))?;
        loop {
            if state.load(Ordering::SeqCst) == 2 {
                let _ = extraction.kill(); let _ = extraction.wait();
                return Err("Đã hủy export.".into());
            }
            if let Some(status) = extraction.try_wait().map_err(|error| format!("Không thể theo dõi trích xuất frame: {error}"))? {
                if !status.success() { return Err("FFmpeg không trích xuất được vùng chữ để xóa.".into()); }
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let first = fs::read_dir(&frames_dir).map_err(|error| error.to_string())?
            .filter_map(Result::ok).any(|entry| entry.path().extension().is_some_and(|ext| ext == "jpg"));
        if !first { return Err("Không có frame trong thời gian của vùng xóa chữ.".into()); }
        progress(0.03 + 0.12 * region_index as f64 / regions.len() as f64,
            format!("Đang xử lý vùng chữ bằng OCR và AI · vùng {}/{}", region_index + 1, regions.len()));
        let request = serde_json::json!({
            "operation": "text_removal_process",
            "framesDir": frames_dir,
            "masksDir": masks_dir,
            "outputDir": output_dir,
            "cacheDir": cache_dir,
            "modelPath": model_path,
            "ocrModel": ocr_model,
            "ocrDevice": ocr_device,
        });
        let local_start = Instant::now();
        let output = run_worker(&worker, &request, state, |value, message| {
            let all = (region_index as f64 + value.clamp(0.0, 1.0)) / regions.len() as f64;
            progress(0.03 + 0.12 * all, message);
        })?;
        if output.masked_pixels.unwrap_or(0) == 0 {
            return Err("OCR không tìm thấy chữ trong vùng đã chọn. Hãy chỉnh khung hoặc kiểm tra cấu hình OCR.".into());
        }
        log::info!("AI text removal processed region {} in {:.1}s", region.id, local_start.elapsed().as_secs_f64());
        let image_pattern = output_dir.join("clean_%08d.png");
        patches.push(AiTextRemovalPatch {
            image_pattern,
            rect,
            fps: frame_rate,
            start_seconds: start,
            end_seconds: end,
        });
    }
    Ok(patches)
}

pub fn start_preview(
    app: AppHandle,
    jobs: &crate::render_service::RenderJobs,
    store: crate::project_service::ProjectStore,
    project_id: String,
    region_id: String,
    current_time: f64,
) -> Result<String, String> {
    if !current_time.is_finite() || current_time < 0.0 {
        return Err("Thời điểm preview không hợp lệ.".into());
    }
    require_ready(&app)?;
    let project = store.get(&project_id)?;
    let duration = project.source.duration.ok_or("Video chưa có thời lượng hợp lệ.")?;
    let region = project.blur_regions.iter()
        .find(|region| region.id == region_id)
        .ok_or("Không tìm thấy vùng xóa chữ đang chọn.")?;
    if !region.enabled || region.effect != BlurEffect::AiTextRemoval {
        return Err("Hãy bật và chọn vùng Xóa chữ bằng AI trước khi xử lý preview.".into());
    }
    let window_length = duration.min(3.0);
    let start = (current_time - window_length * 0.45).clamp(0.0, (duration - window_length).max(0.0));
    let end = (start + window_length).min(duration);
    if end <= start {
        return Err("Không thể xác định đoạn video để preview.".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let state = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    crate::render_service::register_auxiliary_job(
        &app,
        jobs,
        id.clone(),
        project_id.clone(),
        project.name.clone(),
        "text_removal_preview",
        &["cpu", "ffmpeg", "python", "disk"],
        state.clone(),
    )?;
    let worker_jobs = jobs.clone();
    let worker_app = app.clone();
    let worker_store = store.clone();
    let worker_project_id = project_id.clone();
    let worker_region_id = region_id.clone();
    let worker_job_id = id.clone();
    thread::spawn(move || {
        run_preview_job(
            worker_app,
            &worker_jobs,
            worker_store,
            worker_project_id,
            worker_region_id,
            worker_job_id,
            start,
            end,
            state,
        );
    });
    Ok(id)
}

fn run_preview_job(
    app: AppHandle,
    jobs: &crate::render_service::RenderJobs,
    store: crate::project_service::ProjectStore,
    project_id: String,
    region_id: String,
    job_id: String,
    start: f64,
    end: f64,
    state: std::sync::Arc<std::sync::atomic::AtomicU8>,
) {
    use std::sync::atomic::Ordering;
    let started = Instant::now();
    let fps = 8.0;
    let output = run_preview_pipeline(
        &app,
        &store,
        &project_id,
        &region_id,
        &job_id,
        start,
        end,
        fps,
        &state,
        |progress, message| emit_preview_event(
            &app,
            TextRemovalPreviewEvent {
                project_id: project_id.clone(),
                job_id: job_id.clone(),
                status: "running".into(),
                progress: progress.clamp(0.0, 0.99),
                message,
                output: None,
                start_seconds: start,
                end_seconds: end,
                region_id: region_id.clone(),
                fps,
            },
        ),
    );
    let cancelled = state.load(Ordering::SeqCst) == 2;
    let output = if cancelled {
        if let Ok(path) = &output { let _ = fs::remove_file(path); }
        Err("Đã hủy xử lý preview.".to_string())
    } else {
        output
    };
    let event = match output {
        Ok(path) => TextRemovalPreviewEvent {
            project_id: project_id.clone(),
            job_id: job_id.clone(),
            status: "completed".into(),
            progress: 1.0,
            message: format!("Đã xử lý preview vùng chữ trong {:.1} giây.", started.elapsed().as_secs_f64()),
            output: Some(path.to_string_lossy().into_owned()),
            start_seconds: start,
            end_seconds: end,
            region_id: region_id.clone(),
            fps,
        },
        Err(error) => TextRemovalPreviewEvent {
            project_id: project_id.clone(),
            job_id: job_id.clone(),
            status: if cancelled { "cancelled" } else { "failed" }.into(),
            progress: 0.0,
            message: if cancelled { "Đã hủy xử lý preview.".into() } else { error },
            output: None,
            start_seconds: start,
            end_seconds: end,
            region_id,
            fps,
        },
    };
    if let Ok(project_root) = store.project_directory(&project_id) {
        let _ = fs::remove_dir_all(project_root.join("cache").join("render").join(&job_id));
    }
    emit_preview_event(&app, event);
    crate::render_service::finish_auxiliary_job(jobs, &job_id);
}

fn run_preview_pipeline<F>(
    app: &AppHandle,
    store: &crate::project_service::ProjectStore,
    project_id: &str,
    region_id: &str,
    job_id: &str,
    start: f64,
    end: f64,
    fps: f64,
    state: &std::sync::Arc<std::sync::atomic::AtomicU8>,
    mut progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(f64, String),
{
    use std::sync::atomic::Ordering;
    let mut project = store.get(project_id)?;
    let duration = project.source.duration.ok_or("Video chưa có thời lượng hợp lệ.")?;
    let project_root = store.project_directory(project_id)?;
    let source = crate::video_service::resolve_video(store, project_id)?;
    let mut region = project.blur_regions.iter()
        .find(|region| region.id == region_id)
        .cloned()
        .ok_or("Không tìm thấy vùng xóa chữ đang chọn.")?;
    region.start_ms = (start * 1000.0).round() as u64;
    region.end_ms = Some((end * 1000.0).round() as u64);
    project.blur_regions = vec![region];
    project.subtitle.enabled = false;
    project.logos.clear();
    let transcript = crate::transcript_service::Transcript {
        schema_version: 1,
        project_id: project.id.clone(),
        source_type: crate::project_service::TranscriptSourceType::Asr,
        source_track: None,
        language: None,
        video_checksum: project.source.checksum_sha256.clone(),
        segments: Vec::new(),
    };
    let mut options = crate::render_spec::ExportOptions::default();
    options.burn_subtitles = false;
    options.include_original_audio = false;
    options.include_dubbing = false;
    options.apply_logos = false;
    let mut spec = crate::render_spec::build_render_specification(&project, &transcript, duration, &options)?;
    spec.fps = Some(fps);

    let render_dir = project_root.join("cache").join("render").join(job_id);
    fs::create_dir_all(&render_dir).map_err(|error| format!("Không thể tạo thư mục preview tạm: {error}"))?;
    let canonical_project = fs::canonicalize(&project_root).map_err(|error| error.to_string())?;
    let canonical_render = fs::canonicalize(&render_dir).map_err(|error| error.to_string())?;
    if !canonical_render.starts_with(&canonical_project) {
        return Err("Đường dẫn preview vượt khỏi thư mục dự án.".into());
    }
    let preview_dir = project_root.join("cache").join("preview").join("ai-removal");
    fs::create_dir_all(&preview_dir).map_err(|error| format!("Không thể tạo thư mục lưu preview: {error}"))?;
    let canonical_preview = fs::canonicalize(&preview_dir).map_err(|error| error.to_string())?;
    if !canonical_preview.starts_with(&canonical_project) {
        return Err("Thư mục lưu preview nằm ngoài dự án.".into());
    }
    let output = canonical_preview.join(format!("{job_id}.mp4"));
    progress(0.04, "Đang lấy khung hình quanh vị trí phát…".into());
    let patches = prepare_export_patches(app, &spec, &source, &render_dir, state, |value, message| {
        progress((value / 0.15 * 0.76).clamp(0.06, 0.80), message);
    })?;
    let patch = patches.first().ok_or("Không tạo được ảnh đã xóa chữ cho vùng đang chọn.")?;
    if state.load(Ordering::SeqCst) == 2 { return Err("Đã hủy xử lý preview.".into()); }
    progress(0.84, "Đang ghép clip preview để xem kết quả…".into());
    render_preview_clip(app, &source, patch, start, end, &output, state, &mut progress)?;
    if !output.is_file() || fs::metadata(&output).map(|metadata| metadata.len()).unwrap_or(0) == 0 {
        let _ = fs::remove_file(&output);
        return Err("Clip preview xóa chữ không hợp lệ.".into());
    }
    let canonical_output = fs::canonicalize(&output).map_err(|error| format!("Không thể xác minh clip preview: {error}"))?;
    if !canonical_output.starts_with(&canonical_preview) {
        return Err("Clip preview nằm ngoài phạm vi thư mục dự án.".into());
    }
    app.asset_protocol_scope().allow_file(&canonical_output)
        .map_err(|error| format!("Không thể mở clip preview trong ứng dụng: {error}"))?;
    let _ = fs::remove_dir_all(&render_dir);
    progress(0.99, "Preview đã sẵn sàng.".into());
    Ok(canonical_output)
}

fn render_preview_clip<F>(
    _app: &AppHandle,
    source: &Path,
    patch: &AiTextRemovalPatch,
    start: f64,
    end: f64,
    output: &Path,
    state: &std::sync::Arc<std::sync::atomic::AtomicU8>,
    progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(f64, String),
{
    use std::sync::atomic::Ordering;
    let duration = end - start;
    let graph = format!(
        "[0:v:0][1:v:0]overlay={}:{}:eof_action=pass:shortest=1,format=yuv420p[vout]",
        patch.rect.x, patch.rect.y
    );
    let mut command = crate::runtime_paths::ffmpeg_command()?;
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{start:.6}"))
        .arg("-i").arg(source)
        .arg("-t").arg(format!("{duration:.6}"))
        .arg("-framerate").arg(format!("{:.6}", patch.fps))
        .arg("-start_number").arg("1")
        .arg("-i").arg(&patch.image_pattern)
        .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "0:a?"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "23", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "96k", "-shortest", "-progress", "pipe:1", "-nostats"])
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| format!("Không thể tạo video preview: {error}"))?;
    let stdout = child.stdout.take().ok_or("Không đọc được tiến trình tạo preview.")?;
    let stderr = child.stderr.take().ok_or("Không đọc được lỗi FFmpeg khi tạo preview.")?;
    let (sender, receiver) = mpsc::channel::<String>();
    thread::spawn(move || for line in BufReader::new(stdout).lines().flatten() { if sender.send(line).is_err() { break; } });
    let stderr_thread = thread::spawn(move || {
        let mut message = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut message);
        message
    });
    let started = Instant::now();
    let mut out_seconds = 0.0_f64;
    let mut last_emit = Instant::now();
    loop {
        if state.load(Ordering::SeqCst) == 2 {
            let _ = child.kill(); let _ = child.wait(); let _ = stderr_thread.join();
            let _ = fs::remove_file(output);
            return Err("Đã hủy xử lý preview.".into());
        }
        while let Ok(line) = receiver.try_recv() {
            if let Some((key, value)) = line.split_once('=') {
                if key == "out_time_ms" || key == "out_time_us" {
                    out_seconds = value.parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
                }
                if key == "progress" && last_emit.elapsed() >= Duration::from_millis(250) {
                    progress(0.84 + (out_seconds / duration).clamp(0.0, 1.0) * 0.14, "Đang mã hóa clip preview…".into());
                    last_emit = Instant::now();
                }
            }
        }
        if let Some(status) = child.try_wait().map_err(|error| format!("Không thể theo dõi FFmpeg preview: {error}"))? {
            let details = stderr_thread.join().unwrap_or_default();
            if !status.success() {
                let _ = fs::remove_file(output);
                let detail = details.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
                return Err(if detail.is_empty() { "FFmpeg không tạo được clip preview.".into() } else { format!("FFmpeg không tạo được clip preview: {detail}") });
            }
            progress(0.98, "Đang hoàn tất clip preview…".into());
            return Ok(());
        }
        if last_emit.elapsed() >= Duration::from_millis(500) {
            progress(0.84 + (out_seconds / duration).clamp(0.0, 1.0) * 0.14, format!("Đang mã hóa clip preview · {:.1}s", started.elapsed().as_secs_f64()));
            last_emit = Instant::now();
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn emit_preview_event(app: &AppHandle, event: TextRemovalPreviewEvent) {
    crate::job_service::sync_app_event(app, "text_removal_preview", &event);
    let _ = app.emit("text-removal-preview-progress", event);
}

fn run_worker<F>(
    worker: &Path,
    request: &serde_json::Value,
    state: &AtomicU8,
    mut on_progress: F,
) -> Result<WorkerEvent, String>
where
    F: FnMut(f64, String),
{
    let mut child = crate::runtime_paths::worker_command(worker)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| format!("Không thể khởi chạy xử lý AI xóa chữ: {error}"))?;
    let mut input = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    input.push(b'\n');
    child.stdin.take().ok_or("Python worker không mở stdin.")?
        .write_all(&input).map_err(|error| format!("Không thể gửi yêu cầu xóa chữ: {error}"))?;
    let stdout = child.stdout.take().ok_or("Python worker không mở stdout.")?;
    let stderr_pipe = child.stderr.take().ok_or("Python worker không mở stderr.")?;
    let (sender, receiver) = mpsc::channel::<String>();
    thread::spawn(move || for line in BufReader::new(stdout).lines().flatten() { if sender.send(line).is_err() { break; } });
    let stderr_thread = thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr_pipe).read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let mut final_result = None;
    loop {
        if state.load(Ordering::SeqCst) == 2 {
            let _ = child.kill(); let _ = child.wait(); let _ = stderr_thread.join();
            return Err("Đã hủy export.".into());
        }
        if started.elapsed() > WORKER_TIMEOUT {
            let _ = child.kill(); let _ = child.wait(); let _ = stderr_thread.join();
            return Err("Quá trình xóa chữ bằng AI vượt quá giới hạn 12 giờ và đã dừng.".into());
        }
        match receiver.recv_timeout(Duration::from_millis(120)) {
            Ok(line) => if let Ok(event) = serde_json::from_str::<WorkerEvent>(&line) {
                match event.kind.as_str() {
                    "progress" => on_progress(event.progress.unwrap_or(0.0), event.message.unwrap_or_else(|| "Đang xử lý vùng chữ…".into())),
                    "completed" | "failed" => final_result = Some(event),
                    _ => {}
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = child.try_wait().map_err(|error| format!("Không thể theo dõi Python worker: {error}"))? {
                    if let Some(event) = final_result.take() {
                        let stderr = stderr_thread.join().unwrap_or_default();
                        if status.success() && event.kind == "completed" { return Ok(event); }
                        return Err(event.message.unwrap_or_else(|| format!("AI xóa chữ thất bại: {}", stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | "))));
                    }
                    let stderr = stderr_thread.join().unwrap_or_default();
                    return Err(if stderr.trim().is_empty() { "AI xóa chữ đã dừng mà không trả kết quả.".into() } else { format!("AI xóa chữ gặp lỗi: {}", stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")) });
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(status) = child.try_wait().map_err(|error| format!("Không thể theo dõi Python worker: {error}"))? {
            while let Ok(line) = receiver.try_recv() {
                if let Ok(event) = serde_json::from_str::<WorkerEvent>(&line) {
                    if matches!(event.kind.as_str(), "completed" | "failed") { final_result = Some(event); }
                }
            }
            let stderr = stderr_thread.join().unwrap_or_default();
            return match final_result {
                Some(event) if status.success() && event.kind == "completed" => Ok(event),
                Some(event) => Err(event.message.unwrap_or_else(|| "Không thể xóa chữ bằng AI.".into())),
                None => Err(if stderr.trim().is_empty() { "Python worker xử lý xóa chữ không trả kết quả.".into() } else { format!("Python worker xử lý xóa chữ thất bại: {}", stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")) }),
            };
        }
    }
}
