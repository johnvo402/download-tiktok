use crate::project_service::BlurEffect;
use crate::render_spec::{PixelRect, RenderSpecification};
use crate::project_service::TextRemovalMode;
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

const STTN_MODEL_SIZE: u64 = 66_252_587;
const STTN_MODEL_SHA256: &str = "25b0c2c30042d82efd1893bd42ec726764262d94115393a1718f8d65d2a7817b";
const STTN_MODEL_URL: &str = "https://drive.google.com/uc?export=download&id=1ZAMV8547wmZylKRt5qR_tC5VlosXD4Wv";
const WORKER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRemovalEnvironment {
    pub sttn_model_installed: bool,
    pub sttn_model_bytes: u64,
    pub sttn_estimated_download_bytes: u64,
    pub sttn_runtime_installed: bool,
    pub sttn_cuda_available: bool,
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
    intervals: Option<Vec<RemovalInterval>>,
    interval_results: Option<Vec<WorkerIntervalResult>>,
    detected_frames: Option<usize>,
    sampled_frames: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerIntervalResult {
    masked_pixels: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemovalInterval {
    start_seconds: f64,
    end_seconds: f64,
}

struct RegionScan {
    region_id: String,
    mask_mode: TextRemovalMode,
    rect: PixelRect,
    ocr_rect: PixelRect,
    work_dir: PathBuf,
    intervals: Vec<RemovalInterval>,
}

struct RemovalFrameSequence {
    interval: RemovalInterval,
    frames_dir: PathBuf,
    masks_dir: PathBuf,
    output_dir: PathBuf,
}

struct RemovalRegionWork {
    region_id: String,
    mask_mode: TextRemovalMode,
    rect: PixelRect,
    ocr_rect: PixelRect,
    sequences: Vec<RemovalFrameSequence>,
}

fn app_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("models"))
        .map_err(|error| format!("Không thể xác định thư mục mô hình: {error}"))
}

fn sttn_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_models_dir(app)?.join("text-removal").join("sttn.pth"))
}

/// Remove the retired LaMa weights from the app-managed model folder.
/// Only the two known legacy files are considered; STTN and other models remain untouched.
pub fn cleanup_legacy_lama_assets(app: &AppHandle) -> Result<u64, String> {
    let models_root = app_models_dir(app)?;
    let directory = models_root.join("text-removal");
    let root_metadata = match fs::symlink_metadata(&models_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("Không thể kiểm tra thư mục Models: {error}")),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("Thư mục Models không an toàn; không gỡ tệp cũ.".into());
    }
    let directory_metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("Không thể kiểm tra thư mục model xóa chữ: {error}")),
    };
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err("Thư mục model xóa chữ không an toàn; không gỡ tệp cũ.".into());
    }
    let canonical_root = fs::canonicalize(&models_root).map_err(|error| error.to_string())?;
    let canonical_directory = fs::canonicalize(&directory).map_err(|error| error.to_string())?;
    if !canonical_directory.starts_with(&canonical_root) {
        return Err("Thư mục model xóa chữ nằm ngoài Models của ứng dụng.".into());
    }

    let mut removed = 0_u64;
    for name in ["lama_fp32.onnx", "lama_fp32.onnx.incomplete"] {
        let target = directory.join(name);
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Không thể kiểm tra model cũ: {error}")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Tệp model LaMa cũ không an toàn; đã bỏ qua việc xóa.".into());
        }
        removed += metadata.len();
        fs::remove_file(target).map_err(|error| format!("Không thể xóa model LaMa cũ: {error}"))?;
    }
    Ok(removed)
}

pub fn environment(app: AppHandle) -> Result<TextRemovalEnvironment, String> {
    let settings = crate::settings_service::get_settings(&app)?;
    let model = settings.settings.asr.ocr_model.clone();
    let ocr = crate::asr_service::get_ocr_environment(app.clone(), model.clone());
    let ocr = ocr.ok();
    let sttn_path = sttn_model_path(&app)?;
    let sttn_model_installed = fs::metadata(&sttn_path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == STTN_MODEL_SIZE);
    let runtime = crate::optional_runtime::status();
    let ocr_cuda_available = ocr.as_ref().is_some_and(|value| value.cuda_available);
    Ok(TextRemovalEnvironment {
        sttn_model_installed,
        sttn_model_bytes: if sttn_model_installed { STTN_MODEL_SIZE } else { 0 },
        sttn_estimated_download_bytes: STTN_MODEL_SIZE,
        sttn_runtime_installed: runtime.installed_features.iter().any(|feature| feature == "sttn"),
        sttn_cuda_available: runtime.profile.as_deref() == Some("cuda") && runtime.installed_features.iter().any(|feature| feature == "sttn"),
        ocr_ready: ocr.as_ref().is_some_and(|value| {
            value.worker_installed && value.ocr_installed && value.runtime_installed && value.model_cached
        }),
        ocr_model: model,
        ocr_cuda_available,
        cuda_available: ocr_cuda_available,
    })
}

pub fn install_sttn_model(app: AppHandle) -> Result<(), String> {
    let target = sttn_model_path(&app)?;
    if fs::metadata(&target).is_ok_and(|metadata| metadata.is_file() && metadata.len() == STTN_MODEL_SIZE) {
        return Ok(());
    }
    let directory = target.parent().ok_or("Thư mục mô hình STTN không hợp lệ.")?;
    fs::create_dir_all(directory).map_err(|error| format!("Không thể tạo thư mục mô hình: {error}"))?;
    let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Thư mục mô hình AI không an toàn; đã dừng tải.".into());
    }
    let partial = directory.join("sttn.pth.incomplete");
    let _ = fs::remove_file(&partial);
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30 * 60))
        .user_agent("JC-AI-Desktop/1.0")
        .build()
        .map_err(|error| format!("Không thể khởi tạo tải model STTN: {error}"))?;
    let mut response = client.get(STTN_MODEL_URL).send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("Không tải được model STTN: {error}"))?;
    if response.content_length().is_some_and(|length| length != STTN_MODEL_SIZE) {
        return Err("Kích thước model STTN tải về không hợp lệ.".into());
    }
    let download_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&partial)
            .map_err(|error| format!("Không thể tạo tệp tải model STTN: {error}"))?;
        let mut digest = Sha256::new();
        let mut downloaded = 0_u64;
        let mut buffer = [0_u8; 256 * 1024];
        let mut last_emit = Instant::now();
        loop {
            let count = response.read(&mut buffer).map_err(|error| format!("Tải model STTN bị gián đoạn: {error}"))?;
            if count == 0 { break; }
            if downloaded + count as u64 > STTN_MODEL_SIZE { return Err("Kích thước model STTN tải về vượt dự kiến.".into()); }
            file.write_all(&buffer[..count]).map_err(|error| format!("Không thể ghi model STTN xuống ổ đĩa: {error}"))?;
            digest.update(&buffer[..count]);
            downloaded += count as u64;
            if last_emit.elapsed() >= Duration::from_millis(250) || downloaded == STTN_MODEL_SIZE {
                let _ = app.emit("sttn-model-progress", serde_json::json!({
                    "downloadedBytes": downloaded,
                    "totalBytes": STTN_MODEL_SIZE,
                    "progress": (downloaded as f64 / STTN_MODEL_SIZE as f64).clamp(0.0, 1.0),
                    "message": "Đang tải model STTN…"
                }));
                last_emit = Instant::now();
            }
        }
        file.sync_all().map_err(|error| format!("Không thể hoàn tất ghi model STTN: {error}"))?;
        if downloaded != STTN_MODEL_SIZE || format!("{:x}", digest.finalize()) != STTN_MODEL_SHA256 {
            return Err("Model STTN tải về chưa đầy đủ hoặc không vượt qua kiểm tra toàn vẹn. Hãy thử lại.".into());
        }
        Ok(())
    })();
    if let Err(error) = download_result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if target.exists() {
        if let Err(error) = fs::remove_file(&target) {
            let _ = fs::remove_file(&partial);
            return Err(format!("Không thể thay model STTN cũ: {error}"));
        }
    }
    if let Err(error) = fs::rename(&partial, &target) {
        let _ = fs::remove_file(&partial);
        return Err(format!("Không thể cài model STTN: {error}"));
    }
    let _ = app.emit("sttn-model-progress", serde_json::json!({
        "downloadedBytes": STTN_MODEL_SIZE,
        "totalBytes": STTN_MODEL_SIZE,
        "progress": 1.0,
        "message": "Đã cài model STTN"
    }));
    Ok(())
}

fn remove_managed_model(app: &AppHandle, target: PathBuf, label: &str) -> Result<u64, String> {
    let Some(parent) = target.parent() else {
        return Err(format!("Đường dẫn model {label} không hợp lệ."));
    };
    let metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("Không thể kiểm tra model {label}: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("Tệp model {label} không an toàn; đã dừng gỡ."));
    }
    let models_root = app_models_dir(app)?;
    let canonical_root = fs::canonicalize(&models_root)
        .map_err(|error| format!("Không thể xác minh thư mục model: {error}"))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("Không thể xác minh thư mục model {label}: {error}"))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(format!("Tệp model {label} nằm ngoài thư mục Models của ứng dụng."));
    }
    let bytes = metadata.len();
    fs::remove_file(&target).map_err(|error| format!("Không thể gỡ model {label}: {error}"))?;
    Ok(bytes)
}

pub fn remove_sttn_model(app: AppHandle) -> Result<u64, String> {
    remove_managed_model(&app, sttn_model_path(&app)?, "STTN")
}

pub fn require_effects_ready(app: &AppHandle, effects: &[BlurEffect], require_ocr: bool) -> Result<(), String> {
    let status = environment(app.clone())?;
    if require_ocr && !status.ocr_ready {
        return Err(format!("RapidOCR chưa sẵn sàng. Hãy cài model OCR {} trong Cài đặt → Nhận dạng lời thoại.", status.ocr_model));
    }
    if effects.contains(&BlurEffect::Sttn) {
        if !status.sttn_runtime_installed {
            return Err("Chưa cài bộ xử lý STTN. Mở Cài đặt ứng dụng → Nhận dạng lời thoại → STTN để cài cho CPU hoặc NVIDIA.".into());
        }
        if !status.sttn_model_installed {
            return Err("Chưa cài model STTN. Mở Cài đặt ứng dụng → Nhận dạng lời thoại → STTN để cài model.".into());
        }
    }
    Ok(())
}

fn expand_sttn_rect(rect: PixelRect, frame_width: u32, frame_height: u32) -> (PixelRect, PixelRect) {
    // STTN's released checkpoint was trained on 16:9 432x240 crops. Expand a
    // subtitle ROI to include nearby scene context while retaining the OCR ROI.
    let target_ratio = 432.0 / 240.0;
    let mut width = rect.width.max((rect.height as f64 * target_ratio).ceil() as u32).min(frame_width);
    let mut height = rect.height.max((rect.width as f64 / target_ratio).ceil() as u32).min(frame_height);
    if width < rect.width { width = rect.width.min(frame_width); }
    if height < rect.height { height = rect.height.min(frame_height); }
    if (width as f64 / height.max(1) as f64) < target_ratio && height < frame_height {
        height = ((width as f64 / target_ratio).ceil() as u32).max(rect.height).min(frame_height);
    } else if (width as f64 / height.max(1) as f64) > target_ratio && width < frame_width {
        width = ((height as f64 * target_ratio).ceil() as u32).max(rect.width).min(frame_width);
    }
    let center_x = rect.x.saturating_add(rect.width / 2);
    let center_y = rect.y.saturating_add(rect.height / 2);
    let x = center_x.saturating_sub(width / 2).min(frame_width.saturating_sub(width));
    let y = center_y.saturating_sub(height / 2).min(frame_height.saturating_sub(height));
    let local = PixelRect {
        x: rect.x.saturating_sub(x), y: rect.y.saturating_sub(y),
        width: rect.width.min(width), height: rect.height.min(height),
    };
    (PixelRect { x, y, width, height }, local)
}

fn extract_frames(
    source: &Path,
    start: f64,
    end: f64,
    rect: PixelRect,
    fps: f64,
    frames_dir: &Path,
    state: &AtomicU8,
) -> Result<(), String> {
    if end <= start || !fps.is_finite() || fps <= 0.0 {
        return Err("Khoảng thời gian quét chữ không hợp lệ.".into());
    }
    fs::create_dir_all(frames_dir).map_err(|error| format!("Không thể tạo thư mục frame OCR: {error}"))?;
    let crop = format!("crop={}:{}:{}:{},fps={fps:.6}", rect.width, rect.height, rect.x, rect.y);
    let pattern = frames_dir.join("frame_%08d.jpg");
    let mut extraction = crate::runtime_paths::ffmpeg_command()?
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{start:.6}"))
        .arg("-i").arg(source)
        .arg("-t").arg(format!("{:.6}", end - start))
        .args(["-vf", &crop, "-vsync", "0", "-q:v", "2", "-start_number", "1"])
        .arg(&pattern)
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|error| format!("Không thể trích xuất frame để nhận diện chữ: {error}"))?;
    loop {
        if state.load(Ordering::SeqCst) == 2 {
            let _ = extraction.kill();
            let _ = extraction.wait();
            return Err("Đã hủy xử lý xóa chữ.".into());
        }
        if let Some(status) = extraction.try_wait().map_err(|error| format!("Không thể theo dõi FFmpeg trích xuất frame: {error}"))? {
            if !status.success() {
                return Err("FFmpeg không trích xuất được frame để nhận diện chữ.".into());
            }
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let has_frames = fs::read_dir(frames_dir).map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .any(|entry| entry.path().extension().is_some_and(|extension| extension == "jpg"));
    if !has_frames {
        return Err("Không có frame trong khoảng thời gian cần quét chữ.".into());
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
        .filter(|region| region.enabled && region.effect == BlurEffect::Sttn)
        .collect();
    if regions.is_empty() { return Ok(Vec::new()); }
    let effects: Vec<_> = regions.iter().map(|region| region.effect).collect();
    // RapidOCR is always used as a sparse presence scan. The selected mask mode
    // controls what gets restored inside the detected intervals, not whether
    // those intervals are scanned.
    require_effects_ready(app, &effects, true)?;
    let settings = crate::settings_service::get_settings(app)?;
    let ocr_model = settings.settings.asr.ocr_model;
    let ocr_device = settings.settings.asr.ocr_device;
    let cache_dir = app_models_dir(app)?;
    let sttn_path = sttn_model_path(app)?;
    let frame_rate = spec.fps.filter(|fps| fps.is_finite() && (1.0..=240.0).contains(fps)).unwrap_or(30.0);
    let mut scans = Vec::with_capacity(regions.len());
    for (region_index, region) in regions.iter().enumerate() {
        if state.load(Ordering::SeqCst) == 2 { return Err("Đã hủy export.".into()); }
        let start = (region.start_ms as f64 / 1000.0).clamp(0.0, spec.duration);
        let end = (region.end_ms.map(|value| value as f64 / 1000.0).unwrap_or(spec.duration)).clamp(start, spec.duration);
        if end <= start { continue; }
        let work_dir = render_dir.join(format!("text-removal-{}", region_index + 1));
        let (rect, ocr_rect) = expand_sttn_rect(region.rect, spec.width, spec.height);
        let scan_fps = match region.text_removal_scan_fps {
            1 => 1.0,
            2 => 2.0,
            _ => return Err("Tốc độ quét RapidOCR phải là 1 hoặc 2 khung hình mỗi giây.".into()),
        };
        let scan_frames_dir = work_dir.join("scan-frames");
        progress(0.01 + 0.06 * region_index as f64 / regions.len() as f64,
            format!("Đang quét RapidOCR ở {scan_fps} khung/giây · vùng {}/{}", region_index + 1, regions.len()));
        extract_frames(source, start, end, rect, scan_fps, &scan_frames_dir, state)?;
        let worker = crate::runtime_paths::ocr_worker_dir()?;
        let scan_request = serde_json::json!({
            "operation": "text_removal_scan",
            "framesDir": scan_frames_dir,
            "cacheDir": cache_dir,
            "ocrModel": ocr_model,
            "ocrDevice": ocr_device,
            "ocrRect": { "x": ocr_rect.x, "y": ocr_rect.y, "width": ocr_rect.width, "height": ocr_rect.height },
            "scanFps": scan_fps,
            "startSeconds": start,
            "endSeconds": end,
        });
        let scan_result = run_worker(&worker, &scan_request, state, |value, message| {
            let all = (region_index as f64 + value.clamp(0.0, 1.0)) / regions.len() as f64;
            progress(0.01 + 0.06 * all, message);
        })?;
        let intervals: Vec<_> = scan_result.intervals.unwrap_or_default().into_iter()
            .filter_map(|interval| {
                let span_start = interval.start_seconds.max(start);
                let span_end = interval.end_seconds.min(end);
                (span_start.is_finite() && span_end.is_finite() && span_end > span_start)
                    .then_some(RemovalInterval { start_seconds: span_start, end_seconds: span_end })
            }).collect();
        log::info!("RapidOCR sparse scan for region {} detected {} frames across {} sampled frames and {} intervals (mode={:?})",
            region.id, scan_result.detected_frames.unwrap_or(0), scan_result.sampled_frames.unwrap_or(0), intervals.len(), region.text_removal_mode);
        if !intervals.is_empty() {
            scans.push(RegionScan { region_id: region.id.clone(), mask_mode: region.text_removal_mode, rect, ocr_rect, work_dir, intervals });
        }
    }

    let interval_count = scans.iter().map(|scan| scan.intervals.len()).sum::<usize>();
    if interval_count == 0 {
        return Err("RapidOCR không phát hiện phụ đề trong khoảng thời gian của vùng đã chọn.".into());
    }

    let mut region_work = Vec::with_capacity(scans.len());
    let mut extracted_intervals = 0usize;
    for scan in scans {
        let mut sequences = Vec::with_capacity(scan.intervals.len());
        for (local_index, interval) in scan.intervals.iter().enumerate() {
            if state.load(Ordering::SeqCst) == 2 { return Err("Đã hủy export.".into()); }
            let work_dir = scan.work_dir.join(format!("interval-{:04}", local_index + 1));
            let frames_dir = work_dir.join("frames");
            let masks_dir = work_dir.join("masks");
            let output_dir = work_dir.join("clean");
            let stage = if scan.mask_mode == TextRemovalMode::FullRegion { "Đang lấy khung hình phục hồi toàn vùng tại đoạn có phụ đề" } else { "Đang lấy khung hình có chữ" };
            progress(0.07 + 0.03 * extracted_intervals as f64 / interval_count as f64,
                format!("{stage} · đoạn {}/{}", extracted_intervals + 1, interval_count));
            extract_frames(source, interval.start_seconds, interval.end_seconds, scan.rect, frame_rate, &frames_dir, state)?;
            sequences.push(RemovalFrameSequence { interval: *interval, frames_dir, masks_dir, output_dir });
            extracted_intervals += 1;
        }
        region_work.push(RemovalRegionWork {
            region_id: scan.region_id,
            mask_mode: scan.mask_mode,
            rect: scan.rect,
            ocr_rect: scan.ocr_rect,
            sequences,
        });
    }

    let mut patches = Vec::with_capacity(interval_count);
    let mut processed_intervals = 0usize;
    for work in region_work {
        let sequence_count = work.sequences.len();
        let worker_intervals: Vec<_> = work.sequences.iter().map(|sequence| serde_json::json!({
            "framesDir": sequence.frames_dir.to_string_lossy(),
            "masksDir": sequence.masks_dir.to_string_lossy(),
            "outputDir": sequence.output_dir.to_string_lossy(),
        })).collect();
        let worker = crate::runtime_paths::ai_worker_dir_for("sttn")?;
        let request = serde_json::json!({
            "operation": "text_removal_process_intervals",
            "engine": "sttn",
            "maskMode": match work.mask_mode { TextRemovalMode::OcrGuided => "ocrGuided", TextRemovalMode::FullRegion => "fullRegion" },
            "intervals": worker_intervals,
            "cacheDir": cache_dir,
            "modelPath": &sttn_path,
            "ocrModel": ocr_model,
            "ocrDevice": ocr_device,
            "ocrRect": { "x": work.ocr_rect.x, "y": work.ocr_rect.y, "width": work.ocr_rect.width, "height": work.ocr_rect.height },
        });
        let region_start = processed_intervals;
        let local_start = Instant::now();
        let output = run_worker(&worker, &request, state, |value, message| {
            let all = (region_start as f64 + value.clamp(0.0, 1.0) * sequence_count as f64) / interval_count as f64;
            progress(0.1 + 0.05 * all, message);
        })?;
        let results = output.interval_results.unwrap_or_default();
        if results.len() != sequence_count {
            return Err("Python worker không trả đủ kết quả xóa chữ cho các đoạn OCR.".into());
        }
        for (sequence, result) in work.sequences.into_iter().zip(results) {
            if result.masked_pixels > 0 {
                let image_pattern = sequence.output_dir.join("clean_%08d.png");
                let has_output = fs::read_dir(&sequence.output_dir).map_err(|error| error.to_string())?
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().file_name().is_some_and(|name| name.to_string_lossy().starts_with("clean_") && entry.path().extension().is_some_and(|extension| extension == "png")));
                if has_output {
                    patches.push(AiTextRemovalPatch {
                        image_pattern,
                        rect: work.rect,
                        fps: frame_rate,
                        start_seconds: sequence.interval.start_seconds,
                        end_seconds: sequence.interval.end_seconds,
                    });
                }
                log::info!("AI text removal processed region {} in {:.1}s", work.region_id, local_start.elapsed().as_secs_f64());
            } else {
                log::debug!("Skipping false-positive text-removal interval for region {}", work.region_id);
            }
        }
        processed_intervals += sequence_count;
    }
    if patches.is_empty() {
        return Err("Không tạo được khung hình phục hồi trong vùng đã chọn. Hãy kiểm tra thời gian và kích thước vùng.".into());
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
    let project = store.get(&project_id)?;
    let duration = project.source.duration.ok_or("Video chưa có thời lượng hợp lệ.")?;
    let region = project.blur_regions.iter()
        .find(|region| region.id == region_id)
        .ok_or("Không tìm thấy vùng xóa chữ đang chọn.")?;
    if !region.enabled || region.effect != BlurEffect::Sttn {
        return Err("Hãy bật và chọn vùng AI Remove · STTN trước khi xử lý preview.".into());
    }
    let needs_ocr = true;
    require_effects_ready(&app, &[region.effect], needs_ocr)?;
    let settings = crate::settings_service::get_settings(&app)?;
    let ocr_device = settings.settings.asr.ocr_device.as_str();
    let ocr_environment = if needs_ocr {
        Some(crate::asr_service::get_ocr_environment(
            app.clone(), settings.settings.asr.ocr_model.clone()
        )?)
    } else {
        None
    };
    if needs_ocr && matches!(ocr_device, "cuda" | "openvino")
        && !ocr_environment.as_ref().is_some_and(|value| value.backend_available(ocr_device)) {
        return Err("Thiết bị đọc chữ chưa sẵn sàng. Mở Cài đặt → Nhận dạng lời thoại để kiểm tra hoặc chọn Tự động/CPU.".into());
    }
    let runtime = crate::optional_runtime::status();
    let runtime_feature = "sttn";
    let ocr_uses_gpu = ocr_environment.as_ref().is_some_and(|value| value.uses_gpu(ocr_device));
    let full_region_uses_cuda = runtime.profile.as_deref() == Some("cuda")
        && runtime.installed_features.iter().any(|feature| feature == runtime_feature);
    let uses_gpu = ocr_uses_gpu || full_region_uses_cuda;
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
        if uses_gpu { &["gpu", "ffmpeg", "python", "disk"] } else { &["cpu", "ffmpeg", "python", "disk"] },
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
    if patches.is_empty() { return Err("RapidOCR không tìm thấy chữ trong vùng preview đã chọn.".into()); }
    if state.load(Ordering::SeqCst) == 2 { return Err("Đã hủy xử lý preview.".into()); }
    progress(0.84, "Đang ghép clip preview để xem kết quả…".into());
    render_preview_clip(app, &source, &patches, start, end, &output, state, &mut progress)?;
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
    patches: &[AiTextRemovalPatch],
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
    let mut graph = String::new();
    let mut current = "0:v:0".to_string();
    for (index, patch) in patches.iter().enumerate() {
        let next = format!("vclean{index}");
        graph.push_str(&format!(
            "[{current}][{}:v]overlay={}:{}:eof_action=pass:enable='between(t,{:.6},{:.6})'[{next}];",
            index + 1, patch.rect.x, patch.rect.y,
            (patch.start_seconds - start).max(0.0), (patch.end_seconds - start).min(duration)
        ));
        current = next;
    }
    graph.push_str(&format!("[{current}]format=yuv420p[vout]"));
    let mut command = crate::runtime_paths::ffmpeg_command()?;
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{start:.6}"))
        .arg("-i").arg(source);
    for patch in patches {
        command
            .args(["-itsoffset"])
            .arg(format!("{:.6}", (patch.start_seconds - start).max(0.0)))
            .args(["-framerate"])
            .arg(format!("{:.6}", patch.fps))
            .arg("-start_number").arg("1")
            .arg("-i").arg(&patch.image_pattern);
    }
    command
        .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "0:a?"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "23", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "96k", "-progress", "pipe:1", "-nostats", "-t"])
        .arg(format!("{duration:.6}"))
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

fn worker_failure_message(message: Option<String>, stderr: &str) -> String {
    match message.as_deref() {
        Some("Unsupported worker operation") => {
            "Bộ xử lý AI trong bản cài này đã cũ. Hãy cập nhật JC AI để dùng tính năng xóa chữ.".into()
        }
        Some(message) => message.to_string(),
        None if !stderr.trim().is_empty() => format!(
            "AI xóa chữ gặp lỗi: {}",
            stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")
        ),
        None => "Không thể xóa chữ bằng AI.".into(),
    }
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
    let operation = request.get("operation").and_then(|value| value.as_str()).unwrap_or("unknown");
    // Serialize potentially accelerated OCR/STTN with ASR/TTS and the new visual renderer.
    // The permit is released before subsequent FFmpeg work; model inference is unchanged.
    let _gpu_permit = if request.get("engine").and_then(|value| value.as_str()) == Some("sttn")
        || request.get("ocrDevice").and_then(|value| value.as_str()) != Some("cpu") {
        Some(loop {
            if state.load(Ordering::SeqCst) == 2 { return Err("Đã hủy khi đang chờ GPU.".into()); }
            match crate::resource_scheduler::ai_gpu_slot().try_lock() {
                Ok(permit) => break permit,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(50)),
            }
        })
    } else { None };
    log::info!(
        "Starting AI text-removal worker: operation={}, engine={}, requested_ocr_device={}, ocr_model={}",
        operation,
        request.get("engine").and_then(|value| value.as_str()).unwrap_or("unknown"),
        request.get("ocrDevice").and_then(|value| value.as_str()).unwrap_or("auto"),
        request.get("ocrModel").and_then(|value| value.as_str()).unwrap_or("unknown"),
    );
    let mut command = if request.get("operation").and_then(|value| value.as_str()) == Some("text_removal_scan") {
        crate::runtime_paths::ocr_worker_command(worker)
    } else { crate::runtime_paths::worker_command(worker) };
    let mut child = command
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
                        log_worker_stderr(operation, &stderr, !(status.success() && event.kind == "completed"));
                        if status.success() && event.kind == "completed" { return Ok(event); }
                        return Err(worker_failure_message(event.message, &stderr));
                    }
                    let stderr = stderr_thread.join().unwrap_or_default();
                    log_worker_stderr(operation, &stderr, true);
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
            log_worker_stderr(operation, &stderr, !matches!(final_result.as_ref(), Some(event) if status.success() && event.kind == "completed"));
            return match final_result {
                Some(event) if status.success() && event.kind == "completed" => Ok(event),
                Some(event) => Err(worker_failure_message(event.message, &stderr)),
                None => Err(if stderr.trim().is_empty() { "Python worker xử lý xóa chữ không trả kết quả.".into() } else { format!("Python worker xử lý xóa chữ thất bại: {}", stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")) }),
            };
        }
    }
}

fn log_worker_stderr(operation: &str, stderr: &str, failed: bool) {
    let tail = stderr.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
    if tail.is_empty() { return; }
    if failed {
        log::error!("AI text-removal worker ({operation}) stderr: {tail}");
    } else {
        log::info!("AI text-removal worker ({operation}) diagnostics: {tail}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_scan_and_interval_events_keep_their_typed_results() {
        let scan: WorkerEvent = serde_json::from_str(
            r#"{"type":"completed","intervals":[{"startSeconds":1.25,"endSeconds":3.5}],"detectedFrames":4,"sampledFrames":20}"#,
        ).unwrap();
        let intervals = scan.intervals.unwrap();
        let interval = intervals[0];
        assert_eq!(interval.start_seconds, 1.25);
        assert_eq!(interval.end_seconds, 3.5);
        assert_eq!(scan.detected_frames, Some(4));
        assert_eq!(scan.sampled_frames, Some(20));

        let processed: WorkerEvent = serde_json::from_str(
            r#"{"type":"completed","maskedPixels":120,"intervalResults":[{"frames":10,"maskedPixels":120,"device":"cpu"}]}"#,
        ).unwrap();
        assert_eq!(processed.interval_results.unwrap()[0].masked_pixels, 120);
    }
}
