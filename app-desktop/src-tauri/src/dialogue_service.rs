use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager};

const MODEL_FILE: &str = "checkpoint-multi.ckpt";
const MODEL_BYTES: u64 = 446_680_129;
const MODEL_SHA256: &str = "abcfccf65446752a057f4a302c941479a54b7560ebf8d7bca039d2ea98e64cfc";
const MODEL_URL: &str = "https://zenodo.org/api/records/12701995/files/checkpoint-multi.ckpt/content";
static MODEL_LOCK: Mutex<()> = Mutex::new(());
static MODEL_CHECK: Mutex<Option<(PathBuf, u64, SystemTime, bool)>> = Mutex::new(None);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogueEnvironment {
    pub runtime_installed: bool,
    pub cuda_available: bool,
    pub model_cached: bool,
    pub model_bytes: u64,
    pub estimated_download_bytes: u64,
}

impl DialogueEnvironment {
    pub fn ready(&self) -> bool { self.runtime_installed && self.model_cached }
}

pub fn get_environment(app: &AppHandle) -> Result<DialogueEnvironment, String> {
    let worker = crate::runtime_paths::worker_dir()?;
    let output = inspect_worker(&worker)?;
    let model = model_path(app)?;
    let model_cached = verify_model_if_present(&model)?;
    Ok(DialogueEnvironment {
        runtime_installed: output.get("runtimeInstalled").and_then(Value::as_bool).unwrap_or(false),
        cuda_available: output.get("cudaAvailable").and_then(Value::as_bool).unwrap_or(false),
        model_cached,
        model_bytes: if model_cached { MODEL_BYTES } else { 0 },
        estimated_download_bytes: MODEL_BYTES,
    })
}

pub fn install_model(app: &AppHandle) -> Result<(), String> {
    let _guard = MODEL_LOCK.lock().map_err(|_| "Không thể khóa thao tác model tách thoại.".to_string())?;
    let destination = model_path(app)?;
    if verify_model_if_present(&destination)? { return Ok(()); }
    let parent = destination.parent().ok_or("Thư mục model không hợp lệ.")?;
    fs::create_dir_all(parent).map_err(|error| format!("Không thể tạo thư mục model tách thoại: {error}"))?;
    ensure_managed_directory(parent)?;
    let partial = parent.join(format!(".{MODEL_FILE}.download"));
    if let Ok(meta) = fs::symlink_metadata(&partial) {
        if meta.file_type().is_symlink() || !meta.is_file() { return Err("Tệp tải model tạm không hợp lệ.".into()); }
        fs::remove_file(&partial).map_err(|error| format!("Không thể dọn lượt tải model lỗi trước đó: {error}"))?;
    }
    let result = (|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60 * 60))
            .build().map_err(|error| format!("Không thể khởi tạo lượt tải model: {error}"))?;
        let mut response = client.get(MODEL_URL).send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|error| format!("Không thể tải BandIt v2: {error}"))?;
        let total = response.content_length().unwrap_or(MODEL_BYTES);
        let mut file = File::create(&partial).map_err(|error| format!("Không thể tạo file model: {error}"))?;
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut downloaded = 0u64;
        loop {
            let count = response.read(&mut buffer).map_err(|error| format!("Lỗi mạng khi tải BandIt v2: {error}"))?;
            if count == 0 { break; }
            file.write_all(&buffer[..count]).map_err(|error| format!("Không thể ghi file model: {error}"))?;
            downloaded += count as u64;
            let _ = app.emit("dialogue-model-progress", json!({
                "downloadedBytes": downloaded,
                "totalBytes": total,
                "progress": (downloaded as f64 / total.max(1) as f64).clamp(0.0, 0.99),
                "message": "Đang tải BandIt v2 · tách thoại, nhạc và hiệu ứng…"
            }));
        }
        file.sync_all().map_err(|error| format!("Không thể hoàn tất ghi model: {error}"))?;
        if downloaded != MODEL_BYTES {
            return Err(format!("Kích thước model không đúng ({downloaded} byte, cần {MODEL_BYTES}). Lượt tải đã bị hủy."));
        }
        let digest = file_sha256(&partial)?;
        if digest != MODEL_SHA256 { return Err("Checksum BandIt v2 không khớp; file tải về đã bị loại bỏ.".into()); }
        fs::rename(&partial, &destination).map_err(|error| format!("Không thể hoàn tất cài model: {error}"))?;
        if let Ok(mut cache) = MODEL_CHECK.lock() { *cache = None; }
        let _ = app.emit("dialogue-model-progress", json!({"downloadedBytes": MODEL_BYTES, "totalBytes": MODEL_BYTES, "progress": 1.0, "message": "BandIt v2 đã cài xong"}));
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&partial); }
    result
}

pub fn remove_model(app: &AppHandle, export_jobs: &crate::render_service::RenderJobs) -> Result<(), String> {
    if crate::render_service::has_any_active_job(export_jobs) {
        return Err("Không thể gỡ model khi đang export hoặc tách thoại.".into());
    }
    let _guard = MODEL_LOCK.lock().map_err(|_| "Không thể khóa thao tác model tách thoại.".to_string())?;
    let model = model_path(app)?;
    match fs::symlink_metadata(&model) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Không thể kiểm tra model tách thoại: {error}")),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err("Tệp model tách thoại không hợp lệ; không xóa dữ liệu.".into()),
        Ok(_) => {
            fs::remove_file(model).map_err(|error| format!("Không thể gỡ model BandIt v2: {error}"))?;
            if let Ok(mut cache) = MODEL_CHECK.lock() { *cache = None; }
            Ok(())
        }
    }
}

pub fn separate(
    app: &AppHandle,
    source_wav: &Path,
    output_wav: &Path,
    canceled: impl Fn() -> bool,
    mut progress: impl FnMut(String),
) -> Result<(), String> {
    let worker = crate::runtime_paths::worker_dir()?;
    let model = model_path(app)?;
    if !verify_model_if_present(&model)? { return Err("Chưa cài model BandIt v2. Mở Cài đặt → Xử lý âm thanh để tải model.".into()); }
    let runtime = inspect_worker(&worker)?;
    if !runtime.get("runtimeInstalled").and_then(Value::as_bool).unwrap_or(false) {
        return Err("Thiếu runtime tách thoại. Trong development, chạy scripts/setup-dialogue-dev.ps1 rồi khởi động lại app.".into());
    }
    let request = json!({"operation":"dialogue_separate","inputPath":source_wav,"outputPath":output_wav,"modelPath":model,"device":"auto"});
    let mut child = crate::runtime_paths::worker_command(&worker)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Không thể chạy worker tách thoại: {error}"))?;
    let mut input = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    input.push(b'\n');
    child.stdin.take().ok_or("Worker tách thoại không mở stdin.")?.write_all(&input)
        .map_err(|error| format!("Không thể gửi yêu cầu tới worker tách thoại: {error}"))?;
    let stdout = child.stdout.take().ok_or("Worker tách thoại không mở stdout.")?;
    let stderr = child.stderr.take().ok_or("Worker tách thoại không mở stderr.")?;
    let (send, receive) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || for line in BufReader::new(stdout).lines().flatten() { if send.send(line).is_err() { break; } });
    let stderr_join = std::thread::spawn(move || { let mut data = String::new(); let _ = BufReader::new(stderr).read_to_string(&mut data); data });
    let mut terminal: Option<Value> = None;
    loop {
        if canceled() { let _ = child.kill(); let _ = child.wait(); return Err("Đã hủy bước tách thoại.".into()); }
        match receive.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(line) => if let Ok(event) = serde_json::from_str::<Value>(&line) {
                match event.get("type").and_then(Value::as_str) {
                    Some("progress") => progress(event.get("message").and_then(Value::as_str).unwrap_or("Đang tách thoại…").to_string()),
                    Some("completed") | Some("failed") => terminal = Some(event),
                    _ => {}
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {},
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = child.try_wait().map_err(|error| format!("Không thể kiểm tra worker tách thoại: {error}"))? {
                    let stderr = stderr_join.join().unwrap_or_default();
                    let event = terminal.unwrap_or(Value::Null);
                    if status.success() && event.get("type").and_then(Value::as_str) == Some("completed") && output_wav.is_file() {
                        let metadata = fs::metadata(output_wav).map_err(|error| error.to_string())?;
                        if metadata.len() > 44 { return Ok(()); }
                    }
                    return Err(event.get("message").and_then(Value::as_str).map(str::to_string)
                        .or_else(|| (!stderr.trim().is_empty()).then(|| stderr.trim().chars().rev().take(1500).collect::<String>().chars().rev().collect()))
                        .unwrap_or_else(|| "Worker tách thoại kết thúc mà không tạo được stem thoại.".into()));
                }
            }
        }
    }
}

fn inspect_worker(worker: &Path) -> Result<Value, String> {
    let mut child = crate::runtime_paths::worker_command(worker)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .map_err(|error| format!("Không thể kiểm tra runtime tách thoại: {error}"))?;
    child.stdin.take().ok_or("Worker không mở stdin.")?
        .write_all(b"{\"operation\":\"dialogue_inspect\"}\n")
        .map_err(|error| format!("Không thể yêu cầu kiểm tra runtime tách thoại: {error}"))?;
    let output = child.wait_with_output().map_err(|error| format!("Không thể chờ kiểm tra runtime tách thoại: {error}"))?;
    let value = output.stdout.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()).last()
        .ok_or_else(|| "Python worker không trả trạng thái runtime tách thoại.".to_string())?;
    let event: Value = serde_json::from_slice(value).map_err(|error| format!("Phản hồi kiểm tra runtime không hợp lệ: {error}"))?;
    if event.get("type").and_then(Value::as_str) != Some("capabilities") {
        return Err(event.get("message").and_then(Value::as_str).unwrap_or("Worker không hỗ trợ tách thoại.").to_string());
    }
    Ok(event)
}

fn model_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|error| format!("Không thể xác định thư mục Models: {error}"))?
        .join("models").join("audio-separation").join(MODEL_FILE))
}

fn verify_model_if_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Không thể kiểm tra model tách thoại: {error}")),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err("File model tách thoại không hợp lệ.".into()),
        Ok(metadata) if metadata.len() != MODEL_BYTES => Ok(false),
        Ok(metadata) => {
            if let Ok(modified) = metadata.modified() {
                if let Ok(cache) = MODEL_CHECK.lock() {
                    if let Some((checked_path, checked_size, checked_modified, valid)) = cache.as_ref() {
                        if checked_path == path
                            && *checked_size == metadata.len()
                            && *checked_modified == modified
                        {
                            return Ok(*valid);
                        }
                    }
                }
                let valid = file_sha256(path)? == MODEL_SHA256;
                if let Ok(mut cache) = MODEL_CHECK.lock() {
                    *cache = Some((path.to_path_buf(), metadata.len(), modified, valid));
                }
                Ok(valid)
            } else {
                Ok(file_sha256(path)? == MODEL_SHA256)
            }
        }
    }
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Không thể đọc model: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| format!("Không thể xác minh model: {error}"))?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn ensure_managed_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|error| format!("Không thể xác minh thư mục Models: {error}"))?;
    if meta.file_type().is_symlink() || !meta.is_dir() { return Err("Thư mục model không hợp lệ hoặc là symlink.".into()); }
    Ok(())
}
