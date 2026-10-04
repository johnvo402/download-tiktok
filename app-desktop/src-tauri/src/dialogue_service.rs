use crate::worker_manager::WorkerCommandExt;
use crate::project_service::ProjectStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager};

const MODEL_FILE: &str = "checkpoint-multi.ckpt";
const MODEL_BYTES: u64 = 446_680_129;
const MODEL_SHA256: &str = "abcfccf65446752a057f4a302c941479a54b7560ebf8d7bca039d2ea98e64cfc";
const MODEL_URLS: [&str; 2] = [
    "https://zenodo.org/api/records/12701995/files/checkpoint-multi.ckpt/content",
    "https://zenodo.org/records/12701995/files/checkpoint-multi.ckpt?download=1",
];
const MODEL_DOWNLOAD_ATTEMPTS: usize = 4;
static MODEL_LOCK: Mutex<()> = Mutex::new(());
static STEM_LOCK: Mutex<()> = Mutex::new(());
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
    let worker = crate::runtime_paths::ai_worker_dir_for("bandit")?;
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
        if meta.len() > MODEL_BYTES {
            fs::remove_file(&partial).map_err(|error| format!("Không thể dọn tệp tải BandIt lỗi: {error}"))?;
        }
    }
    let result = (|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60 * 60))
            .connect_timeout(std::time::Duration::from_secs(30))
            .user_agent("JC-AI/0.1 (desktop model downloader)")
            .build().map_err(|error| format!("Không thể khởi tạo lượt tải model: {error}"))?;
        download_model_with_resume(app, &client, &partial)?;
        let file = OpenOptions::new().read(true).open(&partial)
            .map_err(|error| format!("Không thể mở file BandIt đã tải: {error}"))?;
        if file.metadata().map_err(|error| format!("Không thể kiểm tra file BandIt đã tải: {error}"))?.len() != MODEL_BYTES {
            return Err("Tệp BandIt tải về chưa đủ kích thước; phần đã tải được giữ để tiếp tục lần sau.".into());
        }
        let digest = file_sha256(&partial)?;
        if digest != MODEL_SHA256 {
            let _ = fs::remove_file(&partial);
            return Err("Checksum BandIt v2 không khớp; file tải về đã bị loại bỏ. Hãy thử tải lại.".into());
        }
        replace_invalid_model(&partial, &destination)?;
        if let Ok(mut cache) = MODEL_CHECK.lock() { *cache = None; }
        let _ = app.emit("dialogue-model-progress", json!({"downloadedBytes": MODEL_BYTES, "totalBytes": MODEL_BYTES, "progress": 1.0, "message": "BandIt v2 đã cài xong"}));
        Ok(())
    })();
    result
}

pub fn install_model_from_file(app: &AppHandle, source: &Path) -> Result<(), String> {
    let _guard = MODEL_LOCK.lock().map_err(|_| "Không thể khóa thao tác model tách thoại.".to_string())?;
    let destination = model_path(app)?;
    if verify_model_if_present(&destination)? { return Ok(()); }
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Không thể đọc checkpoint đã chọn: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != MODEL_BYTES {
        return Err(format!("File BandIt v2 phải là checkpoint-multi.ckpt hợp lệ ({MODEL_BYTES} byte)."));
    }
    if file_sha256(source)? != MODEL_SHA256 {
        return Err("Checkpoint BandIt v2 đã chọn có SHA-256 không khớp bản phát hành.".into());
    }
    let parent = destination.parent().ok_or("Thư mục model không hợp lệ.")?;
    fs::create_dir_all(parent).map_err(|error| format!("Không thể tạo thư mục Models: {error}"))?;
    ensure_managed_directory(parent)?;
    let staged = parent.join(format!(".{MODEL_FILE}.import"));
    if let Ok(metadata) = fs::symlink_metadata(&staged) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("File import BandIt tạm không hợp lệ.".into());
        }
        fs::remove_file(&staged).map_err(|error| format!("Không thể dọn file import cũ: {error}"))?;
    }
    let result = (|| {
        fs::copy(source, &staged).map_err(|error| format!("Không thể chép checkpoint vào Models: {error}"))?;
        if file_sha256(&staged)? != MODEL_SHA256 {
            return Err("Checkpoint BandIt v2 bị thay đổi khi sao chép.".into());
        }
        replace_invalid_model(&staged, &destination)?;
        if let Ok(mut cache) = MODEL_CHECK.lock() { *cache = None; }
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&staged); }
    result
}

fn replace_invalid_model(staged: &Path, destination: &Path) -> Result<(), String> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("Đường dẫn model hiện tại không hợp lệ; không ghi đè.".into());
        }
        Ok(_) => fs::remove_file(destination)
            .map_err(|error| format!("Không thể thay checkpoint cũ không hợp lệ: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(format!("Không thể kiểm tra checkpoint hiện tại: {error}")),
    }
    fs::rename(staged, destination).map_err(|error| format!("Không thể hoàn tất cài model: {error}"))
}

fn download_model_with_resume(
    app: &AppHandle,
    client: &reqwest::blocking::Client,
    partial: &Path,
) -> Result<(), String> {
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut last_error = String::new();
    for attempt in 0..MODEL_DOWNLOAD_ATTEMPTS {
        let mut offset = match fs::symlink_metadata(partial) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err("Tệp tải model tạm không hợp lệ.".into());
            }
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(format!("Không thể đọc trạng thái tải BandIt: {error}")),
        };
        if offset == MODEL_BYTES { return Ok(()); }
        if offset > MODEL_BYTES { return Err("Tệp tải BandIt tạm vượt quá kích thước dự kiến.".into()); }

        let mut response = None;
        for url in MODEL_URLS {
            let mut request = client.get(url).header(reqwest::header::ACCEPT, "application/octet-stream");
            if offset > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            match request.send() {
                Ok(candidate) if candidate.status().is_success() => {
                    response = Some(candidate);
                    break;
                }
                Ok(candidate) => {
                    last_error = format!("Zenodo trả HTTP {} khi tải BandIt v2.", candidate.status());
                    if candidate.status().as_u16() == 403 {
                        let _ = app.emit("dialogue-model-progress", json!({
                            "downloadedBytes": offset,
                            "totalBytes": MODEL_BYTES,
                            "progress": (offset as f64 / MODEL_BYTES as f64).clamp(0.0, 0.99),
                            "message": "Zenodo từ chối đường tải này · đang thử đường dự phòng…"
                        }));
                    }
                }
                Err(error) => last_error = format!("Lỗi kết nối tới Zenodo: {error}"),
            }
        }
        let Some(mut response) = response else {
            if last_error.contains("HTTP 403") {
                return Err(format!("{last_error} Hãy thử tải bằng trình duyệt từ trang Zenodo rồi dùng nút ‘Cài từ file’ trong Cài đặt."));
            }
            if attempt + 1 == MODEL_DOWNLOAD_ATTEMPTS { break; }
            emit_download_retry(app, offset, attempt + 1);
            std::thread::sleep(std::time::Duration::from_secs((attempt as u64 + 1) * 2));
            continue;
        };
        let status = response.status();

        let append = if offset > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
            let range = response.headers().get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok()).unwrap_or_default();
            if !content_range_starts_at(range, offset) {
                return Err("Zenodo trả phạm vi byte không khớp khi tiếp tục tải model BandIt.".into());
            }
            true
        } else {
            if status == reqwest::StatusCode::PARTIAL_CONTENT {
                return Err("Zenodo trả một phần file dù chưa yêu cầu tiếp tục tải.".into());
            }
            offset = 0;
            false
        };
        let mut options = OpenOptions::new();
        options.write(true).create(true);
        if append { options.append(true); } else { options.truncate(true); }
        let mut file = options.open(partial)
            .map_err(|error| format!("Không thể ghi file tải BandIt: {error}"))?;
        let mut downloaded = offset;
        let mut read_error = None;
        loop {
            match response.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if downloaded.saturating_add(count as u64) > MODEL_BYTES {
                        let _ = fs::remove_file(partial);
                        return Err("Zenodo gửi dữ liệu BandIt vượt quá kích thước dự kiến; file tạm đã bị xóa.".into());
                    }
                    file.write_all(&buffer[..count])
                        .map_err(|error| format!("Không thể ghi file tải BandIt: {error}"))?;
                    downloaded += count as u64;
                    let _ = app.emit("dialogue-model-progress", json!({
                        "downloadedBytes": downloaded,
                        "totalBytes": MODEL_BYTES,
                        "progress": (downloaded as f64 / MODEL_BYTES as f64).clamp(0.0, 0.99),
                        "message": "Đang tải BandIt v2 · tách thoại, nhạc và hiệu ứng…"
                    }));
                }
                Err(error) => { read_error = Some(error); break; }
            }
        }
        file.sync_all().map_err(|error| format!("Không thể hoàn tất ghi model: {error}"))?;
        if downloaded == MODEL_BYTES { return Ok(()); }
        last_error = if let Some(error) = read_error {
            format!("Kết nối bị ngắt khi tải BandIt: {error}")
        } else {
            format!("Tải BandIt chưa hoàn tất ({downloaded}/{MODEL_BYTES} byte); phần đã tải được giữ để tiếp tục.")
        };
        if attempt + 1 == MODEL_DOWNLOAD_ATTEMPTS { break; }
        emit_download_retry(app, downloaded, attempt + 1);
        std::thread::sleep(std::time::Duration::from_secs((attempt as u64 + 1) * 2));
    }
    Err(format!("{last_error} Có thể bấm tải lại để tiếp tục từ phần đã có."))
}

fn content_range_starts_at(value: &str, offset: u64) -> bool {
    value.strip_prefix("bytes ")
        .and_then(|range| range.split_once('-'))
        .and_then(|(start, _)| start.parse::<u64>().ok()) == Some(offset)
}

fn emit_download_retry(app: &AppHandle, downloaded: u64, attempt: usize) {
    let _ = app.emit("dialogue-model-progress", json!({
        "downloadedBytes": downloaded,
        "totalBytes": MODEL_BYTES,
        "progress": (downloaded as f64 / MODEL_BYTES as f64).clamp(0.0, 0.99),
        "message": format!("Mạng chập chờn · đang tiếp tục tải (lần thử {})…", attempt + 1)
    }));
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

pub fn prepare_preview_stem(
    app: &AppHandle,
    store: &ProjectStore,
    project_id: &str,
) -> Result<PathBuf, String> {
    let _guard = STEM_LOCK
        .lock()
        .map_err(|_| "Không thể khóa cache tách thoại preview.".to_string())?;
    if !get_environment(app)?.ready() {
        return Err("Giảm thoại gốc cần runtime và model BandIt v2 đã cài.".into());
    }
    let source = crate::video_service::resolve_video(store, project_id)?;
    let project = store.get(project_id)?;
    let project_root = store.project_directory(project_id)?;
    let cache_root = project_root.join("cache");
    let cache_dir = cache_root.join("audio-separation");
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("Không thể tạo cache tách thoại preview: {error}"))?;
    for directory in [&cache_root, &cache_dir] {
        let metadata = fs::symlink_metadata(directory)
            .map_err(|error| format!("Không thể xác minh cache tách thoại preview: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Cache tách thoại preview không hợp lệ hoặc là symlink.".into());
        }
    }
    let source_hash = project
        .source
        .checksum_sha256
        .as_deref()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .map(Ok)
        .unwrap_or_else(|| source_file_sha256(&source))?;
    let cached = cache_dir.join(format!("{source_hash}_bandit-v2-multi_speech.wav"));
    if let Ok(metadata) = fs::symlink_metadata(&cached) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Cache stem thoại preview không hợp lệ.".into());
        }
        if metadata.len() > 44 && crate::audio::signal::validate_stem(&cached, project.source.duration.unwrap_or(0.0)).is_ok() {
            return Ok(cached);
        }
        fs::remove_file(&cached)
            .map_err(|error| format!("Không thể dọn stem preview lỗi: {error}"))?;
    }

    let input_wav = cache_dir.join(format!(".{source_hash}.preview-input.wav"));
    let partial = cache_dir.join(format!(".{source_hash}.preview-speech.partial.wav"));
    let _ = fs::remove_file(&input_wav);
    let _ = fs::remove_file(&partial);
    let _ = app.emit("dialogue-preview-progress", json!({"message":"Đang chuẩn bị audio để tách thoại preview…"}));
    let mut extract = crate::runtime_paths::ffmpeg_command()?;
    extract
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&source)
        .args(["-map", "0:a:0", "-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"])
        .arg(&input_wav)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = extract
        .output()
        .map_err(|error| format!("Không thể trích audio để preview BandIt: {error}"))?;
    if !output.status.success() || !input_wav.is_file() || fs::metadata(&input_wav).map(|meta| meta.len()).unwrap_or(0) <= 44 {
        let _ = fs::remove_file(&input_wav);
        return Err(format!(
            "FFmpeg không trích được audio cho BandIt preview: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let separation = separate(
        app,
        &input_wav,
        &partial,
        || false,
        |message| {
            let _ = app.emit("dialogue-preview-progress", json!({"message": message}));
        },
    );
    let _ = fs::remove_file(&input_wav);
    if let Err(error) = separation {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if !partial.is_file() || fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0) <= 44 {
        let _ = fs::remove_file(&partial);
        return Err("BandIt không tạo được stem thoại preview hợp lệ.".into());
    }
    if let Err(e) = crate::audio::signal::validate_stem(&partial,project.source.duration.unwrap_or(0.0)) { let _=fs::remove_file(&partial); return Err(e); }
    fs::rename(&partial, &cached)
        .map_err(|error| format!("Không thể lưu stem thoại preview: {error}"))?;
    let _ = app.emit("dialogue-preview-progress", json!({"message":"Stem BandIt preview đã sẵn sàng."}));
    Ok(cached)
}
pub fn separate(
    app: &AppHandle,
    source_wav: &Path,
    output_wav: &Path,
    canceled: impl Fn() -> bool,
    mut progress: impl FnMut(String),
) -> Result<(), String> {
    let worker = crate::runtime_paths::ai_worker_dir_for("bandit")?;
    let model = model_path(app)?;
    if !verify_model_if_present(&model)? { return Err("Chưa cài model BandIt v2. Mở Cài đặt → Xử lý âm thanh để tải model.".into()); }
    let runtime = inspect_worker(&worker)?;
    if !runtime.get("runtimeInstalled").and_then(Value::as_bool).unwrap_or(false) {
        return Err("Chưa cài bộ xử lý tách thoại. Mở Cài đặt → Nhận dạng lời thoại và cài BandIt.".into());
    }
    let request = json!({"operation":"dialogue_separate","inputPath":source_wav,"outputPath":output_wav,"modelPath":model,"device":"auto"});
    let mut child = crate::runtime_paths::worker_command(&worker)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn_worker()
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
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn_worker()
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

fn source_file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Không thể đọc video để tạo cache BandIt: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Không thể tính checksum video cho BandIt: {error}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
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

#[cfg(test)]
mod tests {
    use super::content_range_starts_at;

    #[test]
    fn validates_resumed_download_range_before_appending_bytes() {
        assert!(content_range_starts_at("bytes 1048576-446680128/446680129", 1_048_576));
        assert!(!content_range_starts_at("bytes 0-446680128/446680129", 1_048_576));
        assert!(!content_range_starts_at("items 1048576-446680128/446680129", 1_048_576));
        assert!(!content_range_starts_at("bytes */446680129", 1_048_576));
    }
}
