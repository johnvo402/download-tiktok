use crate::worker_manager::WorkerCommandExt;
use crate::project_service::BlurEffect;
use crate::project_service::TextRemovalMode;
use crate::render_spec::{PixelRect, RenderSpecification};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const STTN_MODEL_SIZE: u64 = 66_252_587;
const STTN_MODEL_SHA256: &str = "25b0c2c30042d82efd1893bd42ec726764262d94115393a1718f8d65d2a7817b";
const STTN_MODEL_URL: &str =
    "https://drive.google.com/uc?export=download&id=1ZAMV8547wmZylKRt5qR_tC5VlosXD4Wv";
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
    #[cfg(test)]
    intervals: Option<Vec<RemovalInterval>>,
    interval_results: Option<Vec<WorkerIntervalResult>>,
    #[cfg(test)]
    detected_frames: Option<usize>,
    #[cfg(test)]
    sampled_frames: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerIntervalResult {
    masked_pixels: u64,
    path: Option<String>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemovalInterval {
    start_seconds: f64,
    end_seconds: f64,
}

fn app_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    crate::runtime_paths::data_dir(app)
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
    let sttn_model_installed = crate::file_integrity::verified(&sttn_path, STTN_MODEL_SIZE, STTN_MODEL_SHA256);
    let runtime = crate::optional_runtime::status();
    let ocr_cuda_available = ocr.as_ref().is_some_and(|value| value.cuda_available);
    Ok(TextRemovalEnvironment {
        sttn_model_installed,
        sttn_model_bytes: if sttn_model_installed {
            STTN_MODEL_SIZE
        } else {
            0
        },
        sttn_estimated_download_bytes: STTN_MODEL_SIZE,
        sttn_runtime_installed: runtime
            .installed_features
            .iter()
            .any(|feature| feature == "sttn"),
        sttn_cuda_available: runtime.profile.as_deref() == Some("cuda")
            && runtime
                .installed_features
                .iter()
                .any(|feature| feature == "sttn"),
        ocr_ready: ocr.as_ref().is_some_and(|value| {
            value.worker_installed
                && value.ocr_installed
                && value.runtime_installed
                && value.model_cached
        }),
        ocr_model: model,
        ocr_cuda_available,
        cuda_available: ocr_cuda_available,
    })
}

pub fn install_sttn_model(app: AppHandle) -> Result<(), String> {
    let target = sttn_model_path(&app)?;
    if crate::file_integrity::verified(&target, STTN_MODEL_SIZE, STTN_MODEL_SHA256)
    {
        return Ok(());
    }
    let directory = target
        .parent()
        .ok_or("Thư mục mô hình STTN không hợp lệ.")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Không thể tạo thư mục mô hình: {error}"))?;
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
    let mut response = client
        .get(STTN_MODEL_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("Không tải được model STTN: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length != STTN_MODEL_SIZE)
    {
        return Err("Kích thước model STTN tải về không hợp lệ.".into());
    }
    let download_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| format!("Không thể tạo tệp tải model STTN: {error}"))?;
        let mut digest = Sha256::new();
        let mut downloaded = 0_u64;
        let mut buffer = [0_u8; 256 * 1024];
        let mut last_emit = Instant::now();
        loop {
            let count = response
                .read(&mut buffer)
                .map_err(|error| format!("Tải model STTN bị gián đoạn: {error}"))?;
            if count == 0 {
                break;
            }
            if downloaded + count as u64 > STTN_MODEL_SIZE {
                return Err("Kích thước model STTN tải về vượt dự kiến.".into());
            }
            file.write_all(&buffer[..count])
                .map_err(|error| format!("Không thể ghi model STTN xuống ổ đĩa: {error}"))?;
            digest.update(&buffer[..count]);
            downloaded += count as u64;
            if last_emit.elapsed() >= Duration::from_millis(250) || downloaded == STTN_MODEL_SIZE {
                let _ = app.emit(
                    "sttn-model-progress",
                    serde_json::json!({
                        "downloadedBytes": downloaded,
                        "totalBytes": STTN_MODEL_SIZE,
                        "progress": (downloaded as f64 / STTN_MODEL_SIZE as f64).clamp(0.0, 1.0),
                        "message": "Đang tải model STTN…"
                    }),
                );
                last_emit = Instant::now();
            }
        }
        file.sync_all()
            .map_err(|error| format!("Không thể hoàn tất ghi model STTN: {error}"))?;
        if downloaded != STTN_MODEL_SIZE || format!("{:x}", digest.finalize()) != STTN_MODEL_SHA256
        {
            return Err(
                "Model STTN tải về chưa đầy đủ hoặc không vượt qua kiểm tra toàn vẹn. Hãy thử lại."
                    .into(),
            );
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
    let _ = app.emit(
        "sttn-model-progress",
        serde_json::json!({
            "downloadedBytes": STTN_MODEL_SIZE,
            "totalBytes": STTN_MODEL_SIZE,
            "progress": 1.0,
            "message": "Đã cài model STTN"
        }),
    );
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
        return Err(format!(
            "Tệp model {label} nằm ngoài thư mục Models của ứng dụng."
        ));
    }
    let bytes = metadata.len();
    fs::remove_file(&target).map_err(|error| format!("Không thể gỡ model {label}: {error}"))?;
    Ok(bytes)
}

pub fn remove_sttn_model(app: AppHandle) -> Result<u64, String> {
    remove_managed_model(&app, sttn_model_path(&app)?, "STTN")
}

pub fn require_effects_ready(
    app: &AppHandle,
    effects: &[BlurEffect],
    require_ocr: bool,
) -> Result<(), String> {
    let status = environment(app.clone())?;
    if require_ocr && !status.ocr_ready {
        return Err(format!(
            "RapidOCR chưa sẵn sàng. Hãy cài model OCR {} trong Cài đặt → Nhận dạng lời thoại.",
            status.ocr_model
        ));
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

fn expand_sttn_rect(
    rect: PixelRect,
    frame_width: u32,
    frame_height: u32,
) -> (PixelRect, PixelRect) {
    // Add local scene context, then retain the checkpoint's 432:240 aspect ratio
    // where source bounds permit. Never squash a narrow subtitle strip vertically.
    let margin = (frame_height as f64 * 0.04).round().max(8.0) as u32;
    let base_width = rect.width.saturating_add(2 * margin).min(frame_width);
    let base_height = rect.height.saturating_add(2 * margin).min(frame_height);
    let width = base_width
        .max((base_height as f64 * 432. / 240.).ceil() as u32)
        .min(frame_width);
    let height = base_height
        .max((width as f64 * 240. / 432.).ceil() as u32)
        .min(frame_height);
    let x = rect
        .x
        .saturating_add(rect.width / 2)
        .saturating_sub(width / 2)
        .min(frame_width - width);
    let y = rect
        .y
        .saturating_add(rect.height / 2)
        .saturating_sub(height / 2)
        .min(frame_height - height);
    (
        PixelRect {
            x,
            y,
            width,
            height,
        },
        PixelRect {
            x: rect.x - x,
            y: rect.y - y,
            width: rect.width,
            height: rect.height,
        },
    )
}

pub fn prepare_export_patches<F>(
    app: &AppHandle,
    spec: &RenderSpecification,
    source: &Path,
    render_dir: &Path,
    state: &AtomicU8,
    progress: F,
) -> Result<Vec<AiTextRemovalPatch>, String>
where
    F: FnMut(f64, String),
{
    prepare_patches(app, spec, source, render_dir, state, false, progress)
}

fn detection_key(
    source: &str,
    rect: PixelRect,
    start: f64,
    end: f64,
    fps: u8,
    model: &str,
    device: &str,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!({
                "version":2,"source":source,"rect":rect,"start":start,"end":end,"scanFps":fps,
                "ocrModel":model,"ocrVersion":"3.9.2","device":device
            }))
            .expect("finite validated region")
        )
    )
}
fn painting_key(mode: &str, rect: PixelRect) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!({
                "engine":"sttn-v2","contextMasks":2,"model":STTN_MODEL_SHA256,"mode":mode,"rect":rect
            }))
            .expect("validated pixel rectangle")
        )
    )
}
fn removal_region_identity(region: &crate::project_service::BlurRegion) -> serde_json::Value {
    serde_json::json!([
        region.x,
        region.y,
        region.width,
        region.height,
        region.enabled,
        region.effect,
        region.start_ms,
        region.end_ms,
        region.text_removal_scan_fps,
        region.text_removal_mode
    ])
}

/// Read-only lookup for native preview. Missing/stale caches never launch an AI job.
pub fn cached_preview_patches(
    app: &AppHandle,
    store: &crate::project_service::ProjectStore,
    project: &crate::project_service::Project,
) -> Result<Vec<AiTextRemovalPatch>, String> {
    let Some(source_key) = project.source.checksum_sha256.as_deref() else {
        return Ok(vec![]);
    };
    let (Some(width), Some(height), Some(duration)) = (
        project.source.width,
        project.source.height,
        project.source.duration,
    ) else {
        return Ok(vec![]);
    };
    let root = store
        .project_directory(&project.id)?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let cache = root.join("cache/text-removal/v2");
    if !cache.exists() {
        return Ok(vec![]);
    }
    let cache = cache.canonicalize().map_err(|e| e.to_string())?;
    if !cache.starts_with(&root) {
        return Err("Cache xóa chữ nằm ngoài dự án.".into());
    }
    let settings = crate::settings_service::get_settings(app)?.settings.asr;
    let mut patches = vec![];
    for region in project
        .blur_regions
        .iter()
        .filter(|r| r.enabled && r.effect == BlurEffect::Sttn)
    {
        let roi = crate::render_spec::normalized_rect_to_pixels(
            region.x,
            region.y,
            region.width,
            region.height,
            width,
            height,
        )?;
        let start = (region.start_ms as f64 / 1000.).clamp(0., duration);
        let end = region
            .end_ms
            .map(|n| n as f64 / 1000.)
            .unwrap_or(duration)
            .clamp(start, duration);
        if end <= start {
            continue;
        }
        let key = detection_key(
            source_key,
            roi,
            start,
            end,
            region.text_removal_scan_fps,
            &settings.ocr_model,
            &settings.ocr_device,
        );
        let (rect, _) = expand_sttn_rect(roi, width, height);
        let mode = if region.text_removal_mode == TextRemovalMode::FullRegion {
            "fullRegion"
        } else {
            "ocrGuided"
        };
        let detection_path = cache.join(&key).join("detection.json");
        if crate::audio::signal::regular_file(&detection_path).is_err() {
            continue;
        }
        let detection_bytes = fs::read(detection_path).map_err(|e| e.to_string())?;
        let Ok(detection) = serde_json::from_slice::<serde_json::Value>(&detection_bytes) else {
            continue;
        };
        if !valid_detection(&detection, &key, start, end) {
            continue;
        }
        let detection_fingerprint = format!("{:x}", Sha256::digest(&detection_bytes));
        let directory = cache.join(&key).join(painting_key(mode, rect));
        if !directory.exists() {
            continue;
        }
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        if !directory.starts_with(&cache) {
            return Err("Cache phục hồi không an toàn.".into());
        }
        let result = directory.join("result.json");
        if crate::audio::signal::regular_file(&result).is_err() {
            continue;
        }
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(
            &fs::read(result).map_err(|e| e.to_string())?,
        ) else {
            continue;
        };
        if json["version"] != 2
            || json["complete"] != true
            || json["detectionKey"] != key
            || json["detectionFingerprint"] != detection_fingerprint
        {
            continue;
        }
        let Some(files) = json["files"].as_array() else {
            continue;
        };
        for name in files {
            let Some(name) = name
                .as_str()
                .filter(|s| !s.contains(['/', '\\']) && s.ends_with(".pack"))
            else {
                return Err("Tên cache không hợp lệ.".into());
            };
            let path = directory.join(name);
            let Ok(reader) = crate::text_removal_cache::Reader::open(&path) else {
                continue;
            };
            if (reader.metadata.width, reader.metadata.height) != (rect.width, rect.height) {
                continue;
            }
            if let (Some(first), Some(last)) = (
                reader.metadata.frames.first(),
                reader.metadata.frames.last(),
            ) {
                patches.push(AiTextRemovalPatch {
                    image_pattern: path,
                    rect,
                    fps: project.source.fps.unwrap_or(30.),
                    start_seconds: first.pts,
                    end_seconds: last.end,
                });
            }
        }
    }
    Ok(patches)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    state: &'static str,
    intervals: usize,
    cached_frames: usize,
    cached_bytes: u64,
    sampled_frames: usize,
    ocr_backend: Option<String>,
}
fn valid_detection(value: &serde_json::Value, key: &str, start: f64, end: f64) -> bool {
    if value["version"] != 2 || value["key"] != key || value["complete"] != true {
        return false;
    }
    let Some(intervals) = value["intervals"].as_array() else {
        return false;
    };
    let mut previous = start;
    for interval in intervals {
        let (Some(a), Some(b)) = (
            interval["startSeconds"].as_f64(),
            interval["endSeconds"].as_f64(),
        ) else {
            return false;
        };
        if !a.is_finite()
            || !b.is_finite()
            || a < start
            || a < previous - 1e-7
            || a >= b
            || b > end + 1e-7
        {
            return false;
        }
        let Some(boxes) = interval["boxes"].as_array().filter(|b| !b.is_empty()) else {
            return false;
        };
        for points in boxes {
            let Some(points) = points.as_array().filter(|p| p.len() >= 4) else {
                return false;
            };
            if !points.iter().all(|p| {
                p.as_array().is_some_and(|p| {
                    p.len() == 2 && p.iter().all(|n| n.as_f64().is_some_and(f64::is_finite))
                })
            }) {
                return false;
            }
        }
        previous = b;
    }
    true
}
pub fn cache_status(
    app: &AppHandle,
    store: &crate::project_service::ProjectStore,
    project_id: &str,
    region_id: &str,
) -> Result<CacheStatus, String> {
    let mut project = store.get(project_id)?;
    let region = project
        .blur_regions
        .iter()
        .find(|r| r.id == region_id)
        .cloned()
        .ok_or("Vùng xử lý không tồn tại.")?;
    let mut status = CacheStatus {
        state: "not_detected",
        intervals: 0,
        cached_frames: 0,
        cached_bytes: 0,
        sampled_frames: 0,
        ocr_backend: None,
    };
    let (Some(source), Some(width), Some(height), Some(duration)) = (
        project.source.checksum_sha256.as_deref(),
        project.source.width,
        project.source.height,
        project.source.duration,
    ) else {
        return Ok(status);
    };
    let roi = crate::render_spec::normalized_rect_to_pixels(
        region.x,
        region.y,
        region.width,
        region.height,
        width,
        height,
    )?;
    let settings = crate::settings_service::get_settings(app)?.settings.asr;
    let start = (region.start_ms as f64 / 1000.).clamp(0., duration);
    let end = region
        .end_ms
        .map(|n| n as f64 / 1000.)
        .unwrap_or(duration)
        .clamp(start, duration);
    let key = detection_key(
        source,
        roi,
        start,
        end,
        region.text_removal_scan_fps,
        &settings.ocr_model,
        &settings.ocr_device,
    );
    let root = store
        .project_directory(project_id)?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let dir = root.join("cache/text-removal/v2").join(&key);
    if !dir.exists() {
        return Ok(status);
    }
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if !dir.starts_with(&root) {
        return Err("Cache nhận diện không an toàn.".into());
    }
    let detection = dir.join("detection.json");
    if crate::audio::signal::regular_file(&detection).is_err() {
        return Ok(status);
    }
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(detection).map_err(|e| e.to_string())?)
            .map_err(|_| "Cache nhận diện bị hỏng.")?;
    if !valid_detection(&json, &key, start, end) {
        return Ok(status);
    }
    status.intervals = json["intervals"].as_array().map_or(0, Vec::len);
    status.sampled_frames = json["sampledFrames"].as_u64().unwrap_or(0) as usize;
    status.ocr_backend = json["metrics"]["ocrBackend"].as_str().map(str::to_owned);
    status.state = "detected";
    project.blur_regions = vec![region.clone()];
    for p in cached_preview_patches(app, store, &project)? {
        if let Ok(reader) = crate::text_removal_cache::Reader::open(&p.image_pattern) {
            status.cached_frames += reader.metadata.frames.len();
            status.cached_bytes += fs::metadata(&p.image_pattern)
                .map_err(|e| e.to_string())?
                .len();
        }
    }
    if status.cached_frames > 0 {
        status.state = "removed"
    } else {
        // Cancellation preserves committed spans even before the overall manifest exists.
        let (rect, _) = expand_sttn_rect(roi, width, height);
        let mode = if region.text_removal_mode == TextRemovalMode::FullRegion {
            "fullRegion"
        } else {
            "ocrGuided"
        };
        let painted = dir.join(painting_key(mode, rect));
        if painted.exists() {
            let painted = painted.canonicalize().map_err(|e| e.to_string())?;
            if !painted.starts_with(&dir) {
                return Err("Cache phục hồi không an toàn.".into());
            }
            for entry in fs::read_dir(painted).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.extension().is_none_or(|e| e != "pack") {
                    continue;
                }
                let Ok(reader) = crate::text_removal_cache::Reader::open(&path) else {
                    continue;
                };
                if (reader.metadata.width, reader.metadata.height) != (rect.width, rect.height) {
                    continue;
                }
                if reader.metadata.frames.first().is_none_or(|f| f.pts < start)
                    || reader
                        .metadata
                        .frames
                        .last()
                        .is_none_or(|f| f.end > end + 1e-7)
                {
                    continue;
                }
                status.cached_frames += reader.metadata.frames.len();
                status.cached_bytes += fs::metadata(path).map_err(|e| e.to_string())?.len();
            }
            if status.cached_frames > 0 {
                status.state = "partial";
            }
        }
    }
    Ok(status)
}
fn prepare_patches<F>(
    app: &AppHandle,
    spec: &RenderSpecification,
    source: &Path,
    render_dir: &Path,
    state: &AtomicU8,
    detect_only: bool,
    mut progress: F,
) -> Result<Vec<AiTextRemovalPatch>, String>
where
    F: FnMut(f64, String),
{
    let regions: Vec<_> = spec
        .blur_regions
        .iter()
        .filter(|r| r.enabled && r.effect == BlurEffect::Sttn)
        .collect();
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    let root = render_dir
        .ancestors()
        .nth(3)
        .ok_or("Không xác định được thư mục dự án.")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let cached_root = root.join("cache/text-removal/v2");
    fs::create_dir_all(&cached_root).map_err(|e| e.to_string())?;
    let cached_root = cached_root.canonicalize().map_err(|e| e.to_string())?;
    if !cached_root.starts_with(&root) {
        return Err("Cache xóa chữ nằm ngoài dự án.".into());
    }
    let project: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("project.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let meta = fs::metadata(source).map_err(|e| e.to_string())?;
    let source_key = project["source"]["checksumSha256"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}-{:?}", meta.len(), meta.modified().ok()));
    let settings = crate::settings_service::get_settings(app)?.settings.asr;
    let mut patches = Vec::new();
    for (i, region) in regions.iter().enumerate() {
        if state.load(Ordering::SeqCst) == 2 {
            return Err("Đã hủy xóa chữ.".into());
        }
        let start = (region.start_ms as f64 / 1000.0).clamp(0.0, spec.duration);
        let end = (region
            .end_ms
            .map(|n| n as f64 / 1000.0)
            .unwrap_or(spec.duration))
        .clamp(start, spec.duration);
        if end <= start {
            continue;
        }
        if ![1, 2].contains(&region.text_removal_scan_fps) {
            return Err("Tốc độ quét phải là 1 hoặc 2 khung/giây.".into());
        }
        let (rect, ocr_rect) = expand_sttn_rect(region.rect, spec.width, spec.height);
        let detect_key = detection_key(
            &source_key,
            region.rect,
            start,
            end,
            region.text_removal_scan_fps,
            &settings.ocr_model,
            &settings.ocr_device,
        );
        let dir = cached_root.join(&detect_key);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if !dir
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&cached_root)
        {
            return Err("Thư mục nhận diện không an toàn.".into());
        }
        let detection = dir.join("detection.json");
        let scan = serde_json::json!({"operation":"text_removal_detect_v2","sourcePath":source,"sourceKey":source_key,
            "scanRect":region.rect,"startSeconds":start,"endSeconds":end,"scanFps":region.text_removal_scan_fps,
            "cacheDir":app_models_dir(app)?,"ocrModel":settings.ocr_model,"ocrDevice":settings.ocr_device,
            "detectionPath":detection,"detectionKey":detect_key});
        let detection_valid = crate::audio::signal::regular_file(&detection)
            .ok()
            .and_then(|_| fs::read(&detection).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .filter(|v| valid_detection(v, &detect_key, start, end));
        if detection_valid.is_none() {
            require_effects_ready(app, &[], true)?;
            run_worker(
                &crate::runtime_paths::ocr_worker_dir()?,
                &scan,
                state,
                |p, m| progress((i as f64 + p * 0.35) / regions.len() as f64 * 0.15, m),
            )?;
        } else {
            progress(
                i as f64 / regions.len() as f64 * 0.15,
                "Đã dùng lại kết quả nhận diện phụ đề.".into(),
            );
        }
        let detection_bytes = fs::read(&detection).map_err(|e| e.to_string())?;
        let detection_fingerprint = format!("{:x}", Sha256::digest(&detection_bytes));
        let detection_json: serde_json::Value =
            serde_json::from_slice(&detection_bytes).map_err(|_| "Cache nhận diện bị hỏng.")?;
        if detection_json["intervals"]
            .as_array()
            .is_none_or(|a| a.is_empty())
        {
            progress(
                (i + 1) as f64 / regions.len() as f64 * 0.15,
                "Không phát hiện phụ đề trong vùng đã chọn; giữ nguyên video.".into(),
            );
            continue;
        }
        if detect_only {
            progress(
                (i + 1) as f64 / regions.len() as f64,
                format!(
                    "Đã nhận diện {} đoạn có phụ đề.",
                    detection_json["intervals"].as_array().unwrap().len()
                ),
            );
            continue;
        }
        let mode = match region.text_removal_mode {
            TextRemovalMode::OcrGuided => "ocrGuided",
            TextRemovalMode::FullRegion => "fullRegion",
        };
        let paint_key = painting_key(mode, rect);
        let output_dir = dir.join(paint_key);
        fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
        if !output_dir
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&cached_root)
        {
            return Err("Thư mục phục hồi không an toàn.".into());
        }
        let result_path = output_dir.join("result.json");
        let completed = crate::audio::signal::regular_file(&result_path)
            .ok()
            .and_then(|_| fs::read(&result_path).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        let mut cached = Vec::new();
        if let Some(result) = completed.as_ref().filter(|v| {
            v["version"] == 2
                && v["complete"] == true
                && v["detectionKey"] == detect_key
                && v["detectionFingerprint"] == detection_fingerprint
        }) {
            if let Some(files) = result["files"].as_array() {
                for name in files {
                    let Some(name) = name
                        .as_str()
                        .filter(|s| !s.contains(['/', '\\']) && s.ends_with(".pack"))
                    else {
                        cached.clear();
                        break;
                    };
                    let path = output_dir.join(name);
                    if crate::text_removal_cache::Reader::open(&path)
                        .and_then(|mut r| r.verify())
                        .is_err()
                    {
                        cached.clear();
                        break;
                    }
                    cached.push(path);
                }
            }
        }
        let paths = if !cached.is_empty() {
            progress(
                (i + 1) as f64 / regions.len() as f64 * 0.15,
                "Đã dùng lại vùng phục hồi; không chạy STTN lại.".into(),
            );
            cached
        } else {
            require_effects_ready(app, &[BlurEffect::Sttn], false)?;
            let request = serde_json::json!({"operation":"text_removal_inpaint_v2","sourcePath":source,"sourceKey":source_key,
                "detectionPath":detection,"detectionKey":detect_key,"processingRect":rect,"ocrRect":ocr_rect,"maskMode":mode,
                "outputDir":output_dir,"modelPath":sttn_model_path(app)?,"modelHash":STTN_MODEL_SHA256,
                "videoHeight":spec.height,"videoDuration":spec.duration,"engine":"sttn","inpaintDevice":"auto","ocrDevice":"cpu"});
            let result = run_worker(
                &crate::runtime_paths::ai_worker_dir_for("sttn")?,
                &request,
                state,
                |p, m| {
                    progress(
                        (i as f64 + 0.35 + p * 0.65) / regions.len() as f64 * 0.15,
                        m,
                    )
                },
            )?;
            result
                .interval_results
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.masked_pixels > 0)
                .filter_map(|r| r.path.map(PathBuf::from))
                .collect()
        };
        for path in paths {
            let path = path.canonicalize().map_err(|_| "Thiếu cache phục hồi.")?;
            if !path.starts_with(&output_dir.canonicalize().map_err(|e| e.to_string())?) {
                return Err("Kết quả phục hồi nằm ngoài dự án.".into());
            }
            let reader = crate::text_removal_cache::Reader::open(&path)?;
            if let (Some(first), Some(last)) = (
                reader.metadata.frames.first(),
                reader.metadata.frames.last(),
            ) {
                patches.push(AiTextRemovalPatch {
                    image_pattern: path,
                    rect,
                    fps: spec.fps.unwrap_or(30.0),
                    start_seconds: first.pts,
                    end_seconds: last.end,
                });
            }
        }
    }
    Ok(patches)
}

pub fn compatibility_patch_path(
    _app: &AppHandle,
    patch: &AiTextRemovalPatch,
    state: &AtomicU8,
) -> Result<PathBuf, String> {
    if patch.image_pattern.extension().is_none_or(|e| e != "pack") {
        return Ok(patch.image_pattern.clone());
    }
    let dest = patch.image_pattern.with_extension("nut");
    if !dest.is_file() {
        run_worker(
            &crate::runtime_paths::ocr_worker_dir()?,
            &serde_json::json!({"operation":"text_removal_patch_video","patchPath":patch.image_pattern,"ocrDevice":"cpu"}),
            state,
            |_, _| {},
        )?;
    }
    crate::audio::signal::regular_file(&dest)?;
    Ok(dest)
}

pub fn start_preview(
    app: AppHandle,
    jobs: &crate::render_service::RenderJobs,
    store: crate::project_service::ProjectStore,
    project_id: String,
    region_id: String,
    current_time: f64,
    detect_only: bool,
) -> Result<String, String> {
    if !current_time.is_finite() || current_time < 0.0 {
        return Err("Thời điểm preview không hợp lệ.".into());
    }
    let project = store.get(&project_id)?;
    let duration = project
        .source
        .duration
        .ok_or("Video chưa có thời lượng hợp lệ.")?;
    let region = project
        .blur_regions
        .iter()
        .find(|region| region.id == region_id)
        .ok_or("Không tìm thấy vùng xóa chữ đang chọn.")?;
    if !region.enabled || region.effect != BlurEffect::Sttn {
        return Err("Hãy bật và chọn vùng AI Remove · STTN trước khi xử lý preview.".into());
    }
    let needs_ocr = true;
    if detect_only {
        require_effects_ready(&app, &[], true)?;
    }
    let settings = crate::settings_service::get_settings(&app)?;
    let ocr_device = settings.settings.asr.ocr_device.as_str();
    let ocr_environment = if needs_ocr {
        Some(crate::asr_service::get_ocr_environment(
            app.clone(),
            settings.settings.asr.ocr_model.clone(),
        )?)
    } else {
        None
    };
    if needs_ocr
        && matches!(ocr_device, "cuda" | "openvino")
        && !ocr_environment
            .as_ref()
            .is_some_and(|value| value.backend_available(ocr_device))
    {
        return Err("Thiết bị đọc chữ chưa sẵn sàng. Mở Cài đặt → Nhận dạng lời thoại để kiểm tra hoặc chọn Tự động/CPU.".into());
    }
    let runtime = crate::optional_runtime::status();
    let runtime_feature = "sttn";
    let ocr_uses_gpu = ocr_environment
        .as_ref()
        .is_some_and(|value| value.uses_gpu(ocr_device));
    let full_region_uses_cuda = runtime.profile.as_deref() == Some("cuda")
        && runtime
            .installed_features
            .iter()
            .any(|feature| feature == runtime_feature);
    let uses_gpu = ocr_uses_gpu || (!detect_only && full_region_uses_cuda);
    let window_length = duration.min(3.0);
    let start =
        (current_time - window_length * 0.45).clamp(0.0, (duration - window_length).max(0.0));
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
        if uses_gpu {
            &["gpu", "ffmpeg", "python", "disk"]
        } else {
            &["cpu", "ffmpeg", "python", "disk"]
        },
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
            detect_only,
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
    detect_only: bool,
) {
    use std::sync::atomic::Ordering;
    let started = Instant::now();
    let fps = store
        .get(&project_id)
        .ok()
        .and_then(|p| p.source.fps)
        .unwrap_or(30.0);
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
        detect_only,
        |progress, message| {
            emit_preview_event(
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
            )
        },
    );
    let cancelled = state.load(Ordering::SeqCst) == 2;
    let output = if cancelled {
        if let Ok(Some(path)) = &output {
            let _ = fs::remove_file(path);
        }
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
            message: if detect_only {
                "Đã nhận diện phụ đề. Có thể bắt đầu xóa mà không quét OCR lại.".into()
            } else {
                format!(
                    "Đã xử lý vùng chữ trong {:.1} giây; kết quả được lưu để dùng khi export.",
                    started.elapsed().as_secs_f64()
                )
            },
            output: path.map(|p| p.to_string_lossy().into_owned()),
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
            message: if cancelled {
                "Đã hủy xử lý preview.".into()
            } else {
                error
            },
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
    detect_only: bool,
    mut progress: F,
) -> Result<Option<PathBuf>, String>
where
    F: FnMut(f64, String),
{
    use std::sync::atomic::Ordering;
    let mut project = store.get(project_id)?;
    let duration = project
        .source
        .duration
        .ok_or("Video chưa có thời lượng hợp lệ.")?;
    let project_root = store.project_directory(project_id)?;
    let source = crate::video_service::resolve_video(store, project_id)?;
    let region = project
        .blur_regions
        .iter()
        .find(|region| region.id == region_id)
        .cloned()
        .ok_or("Không tìm thấy vùng xóa chữ đang chọn.")?;
    let original_region = region.clone();
    let source_checksum = project.source.checksum_sha256.clone();
    project.blur_regions = vec![region];
    project.subtitle.enabled = false;
    project.logos.clear();
    let transcript = crate::transcript_service::Transcript {
        schema_version: 1,
        project_id: project.id.clone(), video_content_id: None,
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
    let mut spec =
        crate::render_spec::build_render_specification(&project, &transcript, duration, &options)?;
    spec.fps = Some(fps);

    let render_dir = store.data_directory(project_id,&format!("cache/render/{job_id}"))?;
    let canonical_project = fs::canonicalize(&project_root).map_err(|error| error.to_string())?;
    let canonical_render = fs::canonicalize(&render_dir).map_err(|error| error.to_string())?;
    if !canonical_render.starts_with(&canonical_project) {
        return Err("Đường dẫn preview vượt khỏi thư mục dự án.".into());
    }
    let preview_dir = store.data_directory(project_id,"cache/preview/ai-removal")?;
    let canonical_preview = fs::canonicalize(&preview_dir).map_err(|error| error.to_string())?;
    if !canonical_preview.starts_with(&canonical_project) {
        return Err("Thư mục lưu preview nằm ngoài dự án.".into());
    }
    let output = canonical_preview.join(format!("{job_id}.mp4"));
    progress(
        0.04,
        "Đang kiểm tra kết quả nhận diện và phục hồi đã lưu…".into(),
    );
    let patches = prepare_patches(
        app,
        &spec,
        &source,
        &render_dir,
        state,
        detect_only,
        |value, message| {
            progress((value / 0.15 * 0.76).clamp(0.06, 0.80), message);
        },
    )?;
    let latest = store.get(project_id)?;
    if latest.source.checksum_sha256 != source_checksum
        || latest
            .blur_regions
            .iter()
            .find(|r| r.id == region_id)
            .is_none_or(|r| removal_region_identity(r) != removal_region_identity(&original_region))
    {
        return Err("Video hoặc vùng xử lý đã thay đổi. Kết quả cũ được giữ riêng; hãy chạy lại vùng hiện tại.".into());
    }
    if detect_only {
        return Ok(None);
    }
    if patches.is_empty() {
        return Err("RapidOCR không tìm thấy chữ trong vùng preview đã chọn.".into());
    }
    if state.load(Ordering::SeqCst) == 2 {
        return Err("Đã hủy xử lý preview.".into());
    }
    progress(0.84, "Đang ghép clip preview để xem kết quả…".into());
    render_preview_clip(
        app,
        &source,
        &patches,
        start,
        end,
        &output,
        state,
        &mut progress,
    )?;
    if !output.is_file()
        || fs::metadata(&output)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            == 0
    {
        let _ = fs::remove_file(&output);
        return Err("Clip preview xóa chữ không hợp lệ.".into());
    }
    let canonical_output = fs::canonicalize(&output)
        .map_err(|error| format!("Không thể xác minh clip preview: {error}"))?;
    if !canonical_output.starts_with(&canonical_preview) {
        return Err("Clip preview nằm ngoài phạm vi thư mục dự án.".into());
    }
    app.asset_protocol_scope()
        .allow_file(&canonical_output)
        .map_err(|error| format!("Không thể mở clip preview trong ứng dụng: {error}"))?;
    if crate::project_service::verify_tree_has_no_reparse_points(&render_dir).is_ok() {let _ = fs::remove_dir_all(&render_dir);}
    progress(0.99, "Preview đã sẵn sàng.".into());
    Ok(Some(canonical_output))
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
    let patches: Vec<_> = patches
        .iter()
        .filter(|p| p.start_seconds < end && p.end_seconds > start)
        .collect();
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
        .arg("-i")
        .arg(source);
    for patch in patches {
        let path = compatibility_patch_path(_app, patch, state)?;
        command
            .args(["-itsoffset"])
            .arg(format!("{:.6}", (patch.start_seconds - start).max(0.0)));
        if start > patch.start_seconds {
            command
                .arg("-ss")
                .arg(format!("{:.6}", start - patch.start_seconds));
        }
        if path.extension().is_none_or(|e| e != "nut") {
            command
                .arg("-framerate")
                .arg(format!("{:.6}", patch.fps))
                .args(["-start_number", "1"]);
        }
        command.arg("-i").arg(path);
    }
    command
        .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "0:a?"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
        ])
        .args([
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-progress",
            "pipe:1",
            "-nostats",
            "-t",
        ])
        .arg(format!("{duration:.6}"))
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn_contained()
        .map_err(|error| format!("Không thể tạo video preview: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Không đọc được tiến trình tạo preview.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Không đọc được lỗi FFmpeg khi tạo preview.")?;
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
    let mut out_seconds = 0.0_f64;
    let mut last_emit = Instant::now();
    loop {
        if state.load(Ordering::SeqCst) == 2 {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_thread.join();
            let _ = fs::remove_file(output);
            return Err("Đã hủy xử lý preview.".into());
        }
        while let Ok(line) = receiver.try_recv() {
            if let Some((key, value)) = line.split_once('=') {
                if key == "out_time_ms" || key == "out_time_us" {
                    out_seconds = value.parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
                }
                if key == "progress" && last_emit.elapsed() >= Duration::from_millis(250) {
                    progress(
                        0.84 + (out_seconds / duration).clamp(0.0, 1.0) * 0.14,
                        "Đang mã hóa clip preview…".into(),
                    );
                    last_emit = Instant::now();
                }
            }
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Không thể theo dõi FFmpeg preview: {error}"))?
        {
            let details = stderr_thread.join().unwrap_or_default();
            if !status.success() {
                let _ = fs::remove_file(output);
                let detail = details
                    .lines()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" ");
                return Err(if detail.is_empty() {
                    "FFmpeg không tạo được clip preview.".into()
                } else {
                    format!("FFmpeg không tạo được clip preview: {detail}")
                });
            }
            progress(0.98, "Đang hoàn tất clip preview…".into());
            return Ok(());
        }
        if last_emit.elapsed() >= Duration::from_millis(500) {
            progress(
                0.84 + (out_seconds / duration).clamp(0.0, 1.0) * 0.14,
                format!(
                    "Đang mã hóa clip preview · {:.1}s",
                    started.elapsed().as_secs_f64()
                ),
            );
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
            "Bộ xử lý AI trong bản cài này đã cũ. Hãy cập nhật JC AI để dùng tính năng xóa chữ."
                .into()
        }
        Some(message) => message.to_string(),
        None if !stderr.trim().is_empty() => format!(
            "AI xóa chữ gặp lỗi: {}",
            stderr
                .lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ")
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
    let operation = request
        .get("operation")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    // Serialize potentially accelerated OCR/STTN with ASR/TTS and the new visual renderer.
    // The permit is released before subsequent FFmpeg work; model inference is unchanged.
    let _gpu_permit = if (request.get("engine").and_then(|value| value.as_str()) == Some("sttn")
        && request
            .get("inpaintDevice")
            .and_then(|value| value.as_str())
            != Some("cpu")
        && crate::optional_runtime::status().profile.as_deref() == Some("cuda"))
        || request.get("ocrDevice").and_then(|value| value.as_str()) != Some("cpu")
    {
        Some(loop {
            if state.load(Ordering::SeqCst) == 2 {
                return Err("Đã hủy khi đang chờ GPU.".into());
            }
            match crate::resource_scheduler::ai_gpu_slot().try_lock() {
                Ok(permit) => break permit,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    thread::sleep(Duration::from_millis(50))
                }
            }
        })
    } else {
        None
    };
    log::info!(
        "Starting AI text-removal worker: operation={}, engine={}, requested_ocr_device={}, ocr_model={}",
        operation,
        request.get("engine").and_then(|value| value.as_str()).unwrap_or("unknown"),
        request.get("ocrDevice").and_then(|value| value.as_str()).unwrap_or("auto"),
        request.get("ocrModel").and_then(|value| value.as_str()).unwrap_or("unknown"),
    );
    let mut command = if matches!(
        operation,
        "text_removal_scan" | "text_removal_detect_v2" | "text_removal_patch_video"
    ) {
        crate::runtime_paths::ocr_command_for(worker, request["ocrDevice"].as_str().unwrap_or("cpu"))
    } else {
        crate::runtime_paths::worker_command(worker)
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_worker()
        .map_err(|error| format!("Không thể khởi chạy xử lý AI xóa chữ: {error}"))?;
    let mut input = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    input.push(b'\n');
    child
        .stdin
        .take()
        .ok_or("Python worker không mở stdin.")?
        .write_all(&input)
        .map_err(|error| format!("Không thể gửi yêu cầu xóa chữ: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Python worker không mở stdout.")?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or("Python worker không mở stderr.")?;
    let (sender, receiver) = mpsc::sync_channel::<String>(8);
    thread::spawn(move || {
        for line in crate::worker_manager::bounded_lines(BufReader::new(stdout),8*1024*1024).map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_thread = thread::spawn(move || {
        crate::worker_manager::read_tail(stderr_pipe,2*1024*1024).unwrap_or_default()
    });
    let started = Instant::now();
    let mut final_result = None;
    loop {
        if state.load(Ordering::SeqCst) == 2 {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_thread.join();
            return Err("Đã hủy export.".into());
        }
        if started.elapsed() > WORKER_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_thread.join();
            return Err("Quá trình xóa chữ bằng AI vượt quá giới hạn 12 giờ và đã dừng.".into());
        }
        match receiver.recv_timeout(Duration::from_millis(120)) {
            Ok(line) => {
                if let Ok(event) = serde_json::from_str::<WorkerEvent>(&line) {
                    match event.kind.as_str() {
                        "progress" => on_progress(
                            event.progress.unwrap_or(0.0),
                            event
                                .message
                                .unwrap_or_else(|| "Đang xử lý vùng chữ…".into()),
                        ),
                        "completed" | "failed" => final_result = Some(event),
                        _ => {}
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("Không thể theo dõi Python worker: {error}"))?
                {
                    if let Some(event) = final_result.take() {
                        let stderr = stderr_thread.join().unwrap_or_default();
                        log_worker_stderr(
                            operation,
                            &stderr,
                            !(status.success() && event.kind == "completed"),
                        );
                        if status.success() && event.kind == "completed" {
                            return Ok(event);
                        }
                        return Err(worker_failure_message(event.message, &stderr));
                    }
                    let stderr = stderr_thread.join().unwrap_or_default();
                    log_worker_stderr(operation, &stderr, true);
                    return Err(if stderr.trim().is_empty() {
                        "AI xóa chữ đã dừng mà không trả kết quả.".into()
                    } else {
                        format!(
                            "AI xóa chữ gặp lỗi: {}",
                            stderr
                                .lines()
                                .rev()
                                .take(8)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join(" | ")
                        )
                    });
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Không thể theo dõi Python worker: {error}"))?
        {
            while let Ok(line) = receiver.try_recv() {
                if let Ok(event) = serde_json::from_str::<WorkerEvent>(&line) {
                    if matches!(event.kind.as_str(), "completed" | "failed") {
                        final_result = Some(event);
                    }
                }
            }
            let stderr = stderr_thread.join().unwrap_or_default();
            log_worker_stderr(
                operation,
                &stderr,
                !matches!(final_result.as_ref(), Some(event) if status.success() && event.kind == "completed"),
            );
            return match final_result {
                Some(event) if status.success() && event.kind == "completed" => Ok(event),
                Some(event) => Err(worker_failure_message(event.message, &stderr)),
                None => Err(if stderr.trim().is_empty() {
                    "Python worker xử lý xóa chữ không trả kết quả.".into()
                } else {
                    format!(
                        "Python worker xử lý xóa chữ thất bại: {}",
                        stderr
                            .lines()
                            .rev()
                            .take(8)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<Vec<_>>()
                            .join(" | ")
                    )
                }),
            };
        }
    }
}

fn log_worker_stderr(operation: &str, stderr: &str, failed: bool) {
    let tail = stderr
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    if tail.is_empty() {
        return;
    }
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
    fn detection_cache_rejects_matching_key_with_invalid_intervals() {
        let valid = serde_json::json!({"version":2,"key":"k","complete":true,"intervals":[
            {"startSeconds":0.2,"endSeconds":0.8,"boxes":[[[0,0],[20,0],[20,10],[0,10]]]}]});
        assert!(valid_detection(&valid, "k", 0., 1.));
        let mut bad = valid.clone();
        bad["intervals"] = serde_json::json!("broken");
        assert!(!valid_detection(&bad, "k", 0., 1.));
        let mut bad = valid.clone();
        bad["intervals"][0]["endSeconds"] = serde_json::json!(2.);
        assert!(!valid_detection(&bad, "k", 0., 1.));
        let mut bad = valid.clone();
        bad["intervals"][0]["boxes"] = serde_json::json!([[[0, 0]]]);
        assert!(!valid_detection(&bad, "k", 0., 1.));
    }

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
    #[test]
    fn roi_keeps_mask_local_context_and_model_aspect_ratio() {
        for (width, height) in [(1920, 1080), (1080, 1920), (1080, 1080)] {
            let roi = PixelRect {
                x: 100,
                y: height - 160,
                width: width - 200,
                height: 100,
            };
            let (crop, local) = expand_sttn_rect(roi, width, height);
            assert_eq!(crop.x + local.x, roi.x);
            assert_eq!(crop.y + local.y, roi.y);
            assert!(local.x + local.width <= crop.width && local.y + local.height <= crop.height);
            assert!(crop.x + crop.width <= width && crop.y + crop.height <= height);
            assert!((crop.width as f64 / crop.height as f64 - 1.8).abs() < 0.02);
        }
    }
    #[test]
    fn cache_identity_ignores_translation_but_changes_with_roi_and_scan() {
        let rect = PixelRect {
            x: 0,
            y: 10,
            width: 100,
            height: 30,
        };
        let key = detection_key("source", rect, 0., 5., 2, "tiny", "cpu");
        assert_eq!(key, detection_key("source", rect, 0., 5., 2, "tiny", "cpu"));
        assert_ne!(key, detection_key("source", rect, 0., 5., 1, "tiny", "cpu"));
        assert_ne!(
            key,
            detection_key(
                "source",
                PixelRect { x: 1, ..rect },
                0.,
                5.,
                2,
                "tiny",
                "cpu"
            )
        );
        assert_ne!(
            painting_key("fullRegion", rect),
            painting_key("ocrGuided", rect)
        );
    }
}
