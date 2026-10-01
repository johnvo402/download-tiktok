use crate::project_service::ProjectStore;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v", "avi"];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MergeFrameMode {
    #[default]
    Source,
    Portrait,
    Landscape,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MergeVideoCodec {
    #[default]
    #[serde(rename = "h264")]
    H264,
    #[serde(rename = "hevc")]
    Hevc,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MergeEncoder {
    #[default]
    Cpu,
    Nvidia,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeLogoSettings {
    pub logo_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub opacity: f64,
    pub maintain_aspect_ratio: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeTextSettings {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub opacity: f64,
    pub font_family: String,
    pub font_size: u16,
    pub font_weight: u16,
    pub color: String,
    pub alignment: String,
}

#[derive(Clone, Debug)]
struct PreparedMergeLogo {
    path: PathBuf,
    settings: MergeLogoSettings,
}

#[derive(Clone, Default)]
pub struct VideoToolsJobs {
    inner: Arc<Mutex<HashMap<String, VideoToolJobEntry>>>,
    deleting: Arc<Mutex<HashSet<String>>>,
    start_lock: Arc<Mutex<()>>,
}

#[derive(Clone)]
struct VideoToolJobEntry {
    project_id: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeVideoInfo {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u16>,
    pub sample_rate: Option<u32>,
    pub pixel_format: Option<String>,
    stream_signature: Vec<StreamSignature>,
    other_stream_count: usize,
    has_audio: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedMergeVideo {
    pub file_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeScanResult {
    pub videos: Vec<MergeVideoInfo>,
    pub skipped: Vec<SkippedMergeVideo>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TikTokCookieInfo {
    pub file_name: String,
    pub total_cookies: usize,
    pub tiktok_cookies: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StreamSignature {
    codec_type: String,
    codec_name: String,
    width: Option<u32>,
    height: Option<u32>,
    pixel_format: Option<String>,
    profile: Option<String>,
    level: Option<i32>,
    frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u16>,
    channel_layout: Option<String>,
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    profile: Option<String>,
    level: Option<i32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u16>,
    channel_layout: Option<String>,
}

#[derive(Default, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoToolEvent {
    project_id: String,
    job_id: String,
    status: String,
    progress: f64,
    message: String,
    stage: Option<String>,
    details: Option<Value>,
    output: Option<String>,
}

impl VideoToolsJobs {
    pub fn has_active_project_job(&self, project_id: &str) -> bool {
        self.inner
            .lock()
            .map(|jobs| jobs.values().any(|job| job.project_id == project_id))
            .unwrap_or(true)
    }

    pub fn set_project_deleting(&self, project_id: &str) -> Result<(), String> {
        self.deleting
            .lock()
            .map_err(|_| "Không thể truy cập trạng thái video tools jobs.".to_string())?
            .insert(project_id.to_string());
        Ok(())
    }

    pub fn clear_project_deleting(&self, project_id: &str) {
        if let Ok(mut deleting) = self.deleting.lock() {
            deleting.remove(project_id);
        }
    }

    pub fn cancel_project_jobs_and_wait(&self, project_id: &str) -> Result<(), String> {
        let entries = self
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập danh sách video tools jobs.".to_string())?
            .values()
            .filter(|job| job.project_id == project_id)
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            entry.cancelled.store(true, Ordering::SeqCst);
        }
        for _ in 0..200 {
            if !self.has_active_project_job(project_id) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err("Không thể dừng tác vụ video trước khi xóa dự án.".to_string())
    }
}

pub fn cancel(jobs: &VideoToolsJobs, job_id: &str) -> Result<bool, String> {
    let active = jobs
        .inner
        .lock()
        .map_err(|_| "Không thể truy cập trạng thái video tools jobs.".to_string())?;
    if let Some(job) = active.get(job_id) {
        job.cancelled.store(true, Ordering::SeqCst);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn default_drama_directory(store: &ProjectStore, project_id: &str) -> Result<String, String> {
    store.get(project_id)?;
    let project_root = store.project_directory(project_id)?;
    let directory = project_root.join("media").join("tiktok-drama");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Không thể tạo thư mục TikTok trong dự án: {error}"))?;
    ensure_child_directory(&project_root, &directory)
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn inspect_tiktok_cookie_file(cookie_file: &str) -> Result<TikTokCookieInfo, String> {
    const MAX_COOKIE_FILE_BYTES: u64 = 64 * 1024 * 1024;
    let raw_path = PathBuf::from(cookie_file.trim());
    if !raw_path.is_absolute() {
        return Err("Hãy chọn file cookies bằng hộp thoại hệ thống.".into());
    }
    let metadata = fs::symlink_metadata(&raw_path)
        .map_err(|error| format!("Không thể đọc file cookies: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("File cookies phải là file thường, không phải shortcut hoặc liên kết.".into());
    }
    if metadata.len() > MAX_COOKIE_FILE_BYTES {
        return Err("File cookies quá lớn để kiểm tra (giới hạn 64 MB).".into());
    }
    let contents = fs::read_to_string(&raw_path)
        .map_err(|error| format!("Không đọc được file cookies dạng UTF-8: {error}"))?;
    let mut lines = contents.lines();
    let header = lines
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .trim_start_matches('\u{feff}')
        .trim();
    if header != "# Netscape HTTP Cookie File" && header != "# HTTP Cookie File" {
        return Err("File không đúng định dạng cookies Netscape/Mozilla. Hãy xuất lại file cookies.txt từ trình duyệt.".into());
    }

    let mut total_cookies = 0;
    let mut tiktok_cookies = 0;
    for raw_line in lines {
        let mut line = raw_line.trim();
        if line.is_empty() { continue; }
        let http_only = line.starts_with("#HttpOnly_");
        if http_only {
            line = &line["#HttpOnly_".len()..];
        } else if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 7 || fields[5].is_empty() { continue; }
        total_cookies += 1;
        if fields[0].to_ascii_lowercase().contains("tiktok.com") {
            tiktok_cookies += 1;
        }
    }

    Ok(TikTokCookieInfo {
        file_name: raw_path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        total_cookies,
        tiktok_cookies,
    })
}

fn tiktok_browser_profile_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("browser-profiles").join("tiktok"))
        .map_err(|error| format!("Không xác định được nơi lưu phiên TikTok: {error}"))
}

pub fn has_tiktok_browser_session(app: &AppHandle) -> Result<bool, String> {
    let marker = tiktok_browser_profile_directory(app)?.join(".jc-ai-tiktok-session.json");
    match fs::symlink_metadata(marker) {
        Ok(metadata) => Ok(metadata.is_file() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Không kiểm tra được phiên đăng nhập TikTok: {error}")),
    }
}

pub fn start_tiktok_browser_login(
    app: AppHandle,
    jobs: &VideoToolsJobs,
    store: ProjectStore,
    project_id: String,
) -> Result<String, String> {
    let _start_guard = jobs.start_lock.lock().map_err(|_| "Không thể khóa trạng thái video jobs.".to_string())?;
    ensure_project_job_can_start(jobs, &project_id)?;
    let project = store.get(&project_id)?;
    let profile_directory = tiktok_browser_profile_directory(&app)?;
    fs::create_dir_all(&profile_directory)
        .map_err(|error| format!("Không thể chuẩn bị hồ sơ trình duyệt TikTok: {error}"))?;
    let worker = crate::runtime_paths::worker_dir()?;
    let request = json!({
        "operation": "tiktok_browser_login",
        "profileDirectory": profile_directory,
    });
    let job_id = Uuid::new_v4().to_string();
    let cancelled = Arc::new(AtomicBool::new(false));
    insert_job(jobs, &job_id, &project_id, cancelled.clone())?;
    crate::job_service::register_job(&app, job_id.clone(), project_id.clone(), project.name, "tiktok_cookie_login", &["network", "browser"], None);
    emit_video_tool_event(&app, VideoToolEvent {
        project_id: project_id.clone(), job_id: job_id.clone(), status: "queued".into(), progress: 0.0,
        message: "Đang chuẩn bị cửa sổ đăng nhập TikTok…".into(), stage: Some("queued".into()), details: None, output: None,
    });

    let jobs_for_thread = jobs.clone();
    let app_for_thread = app.clone();
    let job_for_thread = job_id.clone();
    let project_for_thread = project_id.clone();
    thread::Builder::new().name(format!("tiktok-login-{}", &job_id[..8])).spawn(move || {
        if let Err(error) = run_tiktok_worker(&app_for_thread, &worker, &request, &job_for_thread, &project_for_thread, &cancelled) {
            let cancelled_now = cancelled.load(Ordering::SeqCst);
            emit_video_tool_event(&app_for_thread, VideoToolEvent {
                project_id: project_for_thread.clone(), job_id: job_for_thread.clone(),
                status: if cancelled_now { "cancelled" } else { "failed" }.into(), progress: 0.0,
                message: if cancelled_now { "Đã hủy kết nối TikTok.".into() } else { error },
                stage: Some("terminal".into()), details: None, output: None,
            });
        }
        remove_job(&jobs_for_thread, &job_for_thread);
    }).map_err(|error| {
        remove_job(jobs, &job_id);
        format!("Không thể mở đăng nhập TikTok: {error}")
    })?;
    Ok(job_id)
}

pub fn default_merge_directory(store: &ProjectStore, project_id: &str) -> Result<String, String> {
    crate::render_service::export_directory(store, project_id)
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn scan_merge_videos(source_directory: &str) -> Result<MergeScanResult, String> {
    let root = canonical_directory(source_directory, "Thư mục video nguồn")?;
    let entries = fs::read_dir(&root)
        .map_err(|error| format!("Không thể đọc thư mục video: {error}"))?;
    let mut paths = Vec::new();
    let mut skipped = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("Không thể đọc một mục trong thư mục: {error}"))?;
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                skipped.push(SkippedMergeVideo { file_name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(), reason: error.to_string() });
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() || !is_supported_video(&path) {
            continue;
        }
        paths.push(path);
    }
    paths.sort_by(|left, right| {
        left.file_name().unwrap_or_default().to_string_lossy().to_lowercase()
            .cmp(&right.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
    });

    let mut videos = Vec::new();
    for path in paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match probe_merge_video(&path) {
            Ok(info) => videos.push(info),
            Err(error) => skipped.push(SkippedMergeVideo { file_name: name, reason: error }),
        }
    }
    Ok(MergeScanResult { videos, skipped })
}

fn probe_merge_video(path: &Path) -> Result<MergeVideoInfo, String> {
    let output = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v", "error", "-show_entries",
            "format=duration:stream=codec_type,codec_name,width,height,pix_fmt,profile,level,avg_frame_rate,r_frame_rate,sample_rate,channels,channel_layout",
            "-of", "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Không chạy được FFprobe: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() { "FFprobe không đọc được video.".into() } else { detail });
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("FFprobe trả metadata không hợp lệ: {error}"))?;
    let video = probe.streams.iter().find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| "Tệp không có video stream.".to_string())?;
    let duration = probe.format.duration.as_deref().and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "Không đọc được thời lượng video.".to_string())?;
    let width = video.width.filter(|value| *value > 0).ok_or_else(|| "Không đọc được chiều rộng video.".to_string())?;
    let height = video.height.filter(|value| *value > 0).ok_or_else(|| "Không đọc được chiều cao video.".to_string())?;
    let audio = probe.streams.iter().find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let size_bytes = fs::metadata(path).map_err(|error| format!("Không đọc được kích thước video: {error}"))?.len();
    let stream_signature = probe.streams.iter().filter_map(|stream| {
        let kind = stream.codec_type.as_deref()?;
        if !matches!(kind, "video" | "audio") { return None; }
        Some(StreamSignature {
            codec_type: kind.to_string(),
            codec_name: stream.codec_name.clone().unwrap_or_default(),
            width: stream.width,
            height: stream.height,
            pixel_format: stream.pix_fmt.clone(),
            profile: stream.profile.clone(),
            level: stream.level,
            frame_rate: if kind == "video" { stream.avg_frame_rate.clone().or_else(|| stream.r_frame_rate.clone()) } else { None },
            sample_rate: stream.sample_rate.clone(),
            channels: stream.channels,
            channel_layout: stream.channel_layout.clone(),
        })
    }).collect();
    let fps = video.avg_frame_rate.as_deref().and_then(parse_ratio)
        .or_else(|| video.r_frame_rate.as_deref().and_then(parse_ratio));
    Ok(MergeVideoInfo {
        path: path.canonicalize().map_err(|error| format!("Không xác minh được đường dẫn video: {error}"))?.to_string_lossy().into_owned(),
        file_name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        size_bytes,
        duration,
        width,
        height,
        fps,
        video_codec: video.codec_name.clone().unwrap_or_else(|| "unknown".into()),
        audio_codec: audio.and_then(|stream| stream.codec_name.clone()),
        audio_channels: audio.and_then(|stream| stream.channels),
        sample_rate: audio.and_then(|stream| stream.sample_rate.as_deref()?.parse().ok()),
        pixel_format: video.pix_fmt.clone(),
        other_stream_count: probe.streams.iter().filter(|stream| !matches!(stream.codec_type.as_deref(), Some("video" | "audio"))).count(),
        has_audio: audio.is_some(),
        stream_signature,
    })
}

fn parse_ratio(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator != 0.0).then_some(numerator / denominator).filter(|value| value.is_finite() && *value > 0.0)
}

fn canonical_directory(value: &str, label: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(value);
    if !raw.is_absolute() {
        return Err(format!("{label} phải được chọn bằng hộp thoại thư mục hệ thống."));
    }
    let canonical = fs::canonicalize(&raw).map_err(|error| format!("Không thể truy cập {label}: {error}"))?;
    if !canonical.is_dir() {
        return Err(format!("{label} không phải là thư mục."));
    }
    Ok(canonical)
}

fn ensure_child_directory(project_root: &Path, child: &Path) -> Result<PathBuf, String> {
    let canonical_root = fs::canonicalize(project_root).map_err(|error| format!("Không xác minh được thư mục dự án: {error}"))?;
    let canonical_child = fs::canonicalize(child).map_err(|error| format!("Không xác minh được thư mục lưu: {error}"))?;
    if canonical_child == canonical_root || !canonical_child.starts_with(&canonical_root) {
        return Err("Thư mục lưu mặc định phải nằm bên trong dự án.".into());
    }
    ensure_writable(&canonical_child)?;
    Ok(canonical_child)
}

fn resolve_output_directory(store: &ProjectStore, project_id: &str, requested: Option<&str>, default: fn(&ProjectStore, &str) -> Result<String, String>) -> Result<PathBuf, String> {
    store.get(project_id)?;
    let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return canonical_directory(&default(store, project_id)?, "Thư mục lưu");
    };
    let directory = canonical_directory(value, "Thư mục lưu")?;
    ensure_writable(&directory)?;
    Ok(directory)
}

fn ensure_writable(directory: &Path) -> Result<(), String> {
    let probe = tempfile::Builder::new().prefix(".avl-write-check-").tempfile_in(directory)
        .map_err(|error| format!("Không thể ghi vào thư mục đã chọn: {error}"))?;
    drop(probe);
    Ok(())
}

fn is_supported_video(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

pub fn start_tiktok_download(
    app: AppHandle,
    jobs: &VideoToolsJobs,
    store: ProjectStore,
    project_id: String,
    url: String,
    output_directory: Option<String>,
    cookie_file: Option<String>,
    use_browser_cookies: bool,
) -> Result<String, String> {
    let _start_guard = jobs.start_lock.lock().map_err(|_| "Không thể khóa trạng thái video jobs.".to_string())?;
    ensure_project_job_can_start(jobs, &project_id)?;
    let project = store.get(&project_id)?;
    let destination = resolve_output_directory(&store, &project_id, output_directory.as_deref(), default_drama_directory)?;
    let url = url.trim().to_string();
    validate_tiktok_url(&url)?;
    let cookie_file = cookie_file.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let use_browser_cookies = use_browser_cookies && cookie_file.is_none();
    if let Some(path) = cookie_file.as_deref() {
        let cookie_info = inspect_tiktok_cookie_file(path)?;
        if cookie_info.tiktok_cookies == 0 {
            return Err("File cookies hợp lệ nhưng không có cookie thuộc tiktok.com. Hãy xuất file sau khi đăng nhập TikTok.".into());
        }
    }
    let browser_profile = if use_browser_cookies {
        if !has_tiktok_browser_session(&app)? {
            return Err("Chưa kết nối TikTok bằng trình duyệt. Hãy đăng nhập TikTok trước.".into());
        }
        Some(tiktok_browser_profile_directory(&app)?)
    } else {
        None
    };
    let job_id = Uuid::new_v4().to_string();
    let worker = crate::runtime_paths::worker_dir()?;
    let request = json!({ "operation": "tiktok_drama_download", "url": url, "outputDirectory": destination, "cookieFile": cookie_file, "browserProfileDirectory": browser_profile, "useBrowserCookies": use_browser_cookies });
    let cancelled = Arc::new(AtomicBool::new(false));
    insert_job(jobs, &job_id, &project_id, cancelled.clone())?;
    crate::job_service::register_job(&app, job_id.clone(), project_id.clone(), project.name, "tiktok_download", &["network", "browser", "disk"], None);
    emit_video_tool_event(&app, VideoToolEvent { project_id: project_id.clone(), job_id: job_id.clone(), status: "queued".into(), progress: 0.0, message: "Đang xếp hàng tải TikTok Drama…".into(), stage: Some("queued".into()), details: None, output: None });

    let jobs_for_thread = jobs.clone();
    let app_for_thread = app.clone();
    let job_for_thread = job_id.clone();
    let project_for_thread = project_id.clone();
    thread::Builder::new().name(format!("tiktok-{}", &job_id[..8])).spawn(move || {
        let result = run_tiktok_worker(&app_for_thread, &worker, &request, &job_for_thread, &project_for_thread, &cancelled);
        if let Err(error) = result {
            let cancelled_now = cancelled.load(Ordering::SeqCst);
            emit_video_tool_event(&app_for_thread, VideoToolEvent {
                project_id: project_for_thread.clone(), job_id: job_for_thread.clone(),
                status: if cancelled_now { "cancelled" } else { "failed" }.into(), progress: 0.0,
                message: if cancelled_now { "Đã hủy tải TikTok Drama.".into() } else { error },
                stage: Some("terminal".into()), details: None, output: None,
            });
        }
        remove_job(&jobs_for_thread, &job_for_thread);
    }).map_err(|error| {
        remove_job(jobs, &job_id);
        format!("Không thể khởi chạy tác vụ TikTok: {error}")
    })?;
    Ok(job_id)
}

pub fn start_video_merge(
    app: AppHandle,
    jobs: &VideoToolsJobs,
    store: ProjectStore,
    project_id: String,
    source_directory: String,
    file_paths: Vec<String>,
    output_directory: Option<String>,
    frame_mode: Option<MergeFrameMode>,
    video_codec: Option<MergeVideoCodec>,
    encoder: Option<MergeEncoder>,
    logo_settings: Vec<MergeLogoSettings>,
    text_settings: Vec<MergeTextSettings>,
) -> Result<String, String> {
    let _start_guard = jobs.start_lock.lock().map_err(|_| "Không thể khóa trạng thái video jobs.".to_string())?;
    ensure_project_job_can_start(jobs, &project_id)?;
    let project = store.get(&project_id)?;
    let logos = prepare_merge_logos(&store, &project, logo_settings)?;
    validate_merge_texts(&text_settings)?;
    let source_root = canonical_directory(&source_directory, "Thư mục video nguồn")?;
    let destination = resolve_output_directory(&store, &project_id, output_directory.as_deref(), default_merge_directory)?;
    let videos = validate_merge_inputs(&source_root, &file_paths)?;
    if videos.len() < 2 { return Err("Chọn ít nhất hai video hợp lệ để gộp.".into()); }
    let total_duration = videos.iter().map(|video| video.duration).sum::<f64>();
    if !total_duration.is_finite() || total_duration <= 0.0 { return Err("Tổng thời lượng video không hợp lệ.".into()); }
    let frame_mode = frame_mode.unwrap_or_default();
    let video_codec = video_codec.unwrap_or_default();
    let encoder = encoder.unwrap_or_default();

    let job_id = Uuid::new_v4().to_string();
    let cancelled = Arc::new(AtomicBool::new(false));
    insert_job(jobs, &job_id, &project_id, cancelled.clone())?;
    let resources: &[&str] = if encoder == MergeEncoder::Nvidia { &["cpu", "gpu", "ffmpeg", "disk"] } else { &["cpu", "ffmpeg", "disk"] };
    crate::job_service::register_job(&app, job_id.clone(), project_id.clone(), project.name, "video_merge", resources, None);
    emit_video_tool_event(&app, VideoToolEvent { project_id: project_id.clone(), job_id: job_id.clone(), status: "queued".into(), progress: 0.0, message: "Đang chuẩn bị gộp video…".into(), stage: Some("queued".into()), details: None, output: None });

    let jobs_for_thread = jobs.clone();
    let app_for_thread = app.clone();
    let job_for_thread = job_id.clone();
    let project_for_thread = project_id.clone();
    thread::Builder::new().name(format!("video-merge-{}", &job_id[..8])).spawn(move || {
        let result = run_merge_job(&app_for_thread, &store, &project_for_thread, &job_for_thread, &source_root, &destination, &videos, &logos, &text_settings, total_duration, frame_mode, video_codec, encoder, &cancelled);
        match result {
            Ok(output) => emit_video_tool_event(&app_for_thread, VideoToolEvent {
                project_id: project_for_thread.clone(), job_id: job_for_thread.clone(), status: "completed".into(), progress: 1.0,
                message: "Đã gộp video và xác minh file đầu ra.".into(), stage: Some("completed".into()), details: None, output: Some(output.to_string_lossy().into_owned()),
            }),
            Err(error) => {
                let cancelled_now = cancelled.load(Ordering::SeqCst);
                emit_video_tool_event(&app_for_thread, VideoToolEvent {
                    project_id: project_for_thread.clone(), job_id: job_for_thread.clone(),
                    status: if cancelled_now { "cancelled" } else { "failed" }.into(), progress: 0.0,
                    message: if cancelled_now { "Đã hủy gộp video; file chưa hoàn tất đã được dọn.".into() } else { error },
                    stage: Some("terminal".into()), details: None, output: None,
                });
            }
        }
        remove_job(&jobs_for_thread, &job_for_thread);
    }).map_err(|error| {
        remove_job(jobs, &job_id);
        format!("Không thể khởi chạy tác vụ gộp video: {error}")
    })?;
    Ok(job_id)
}

fn ensure_project_job_can_start(jobs: &VideoToolsJobs, project_id: &str) -> Result<(), String> {
    if jobs.deleting.lock().map_err(|_| "Không thể truy cập trạng thái dự án.".to_string())?.contains(project_id) {
        return Err("Dự án đang được xóa.".into());
    }
    if jobs.has_active_project_job(project_id) {
        return Err("Một tác vụ Kéo video đang chạy trong dự án này. Hãy chờ hoặc hủy tác vụ hiện tại.".into());
    }
    Ok(())
}

fn prepare_merge_logos(
    store: &ProjectStore,
    project: &crate::project_service::Project,
    requested: Vec<MergeLogoSettings>,
) -> Result<Vec<PreparedMergeLogo>, String> {
    let mut seen = HashSet::new();
    requested.into_iter().map(|settings| {
        let id = Uuid::parse_str(&settings.logo_id)
            .map_err(|_| "Logo được chọn không hợp lệ.".to_string())?;
        if !seen.insert(id) {
            return Err("Không thể thêm cùng một logo nhiều lần.".into());
        }
        if ![settings.x, settings.y, settings.width, settings.height, settings.opacity].into_iter().all(f64::is_finite)
            || settings.x < 0.0 || settings.y < 0.0
            || settings.width <= 0.0 || settings.height <= 0.0
            || settings.x + settings.width > 1.0 + 1e-9
            || settings.y + settings.height > 1.0 + 1e-9
            || !(0.0..=1.0).contains(&settings.opacity)
        {
            return Err("Vị trí, kích thước hoặc độ mờ logo không hợp lệ.".into());
        }
        if !project.logos.iter().any(|logo| logo.id == settings.logo_id) {
            return Err("Không tìm thấy logo đã chọn trong dự án hiện tại.".into());
        }
        let path = crate::overlays_service::logo_asset_path(store, &project.id, &settings.logo_id)?;
        if !path.is_file() {
            return Err("Tệp logo đã chọn không còn tồn tại trong dự án.".into());
        }
        Ok(PreparedMergeLogo { path, settings })
    }).collect()
}

fn validate_merge_texts(texts: &[MergeTextSettings]) -> Result<(), String> {
    if texts.len() > 20 { return Err("Có thể thêm tối đa 20 lớp chữ khi gộp video.".into()); }
    for text in texts {
        if text.text.trim().is_empty() || text.text.chars().count() > 4_000
            || text.text.chars().any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
            || ![text.x, text.y, text.width, text.height, text.opacity].into_iter().all(f64::is_finite)
            || text.x < 0.0 || text.y < 0.0 || text.width <= 0.0 || text.height <= 0.0
            || text.x + text.width > 1.0 + 1e-9 || text.y + text.height > 1.0 + 1e-9
            || !(0.0..=1.0).contains(&text.opacity) || !(8..=240).contains(&text.font_size)
            || !(100..=900).contains(&text.font_weight) || !["left", "center", "right"].contains(&text.alignment.as_str())
            || text.color.len() != 7 || !text.color.starts_with('#')
            || !text.color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
            || text.font_family.trim().is_empty() || text.font_family.len() > 80
        { return Err("Thuộc tính lớp chữ khi gộp video không hợp lệ.".into()); }
    }
    Ok(())
}

fn insert_job(jobs: &VideoToolsJobs, job_id: &str, project_id: &str, cancelled: Arc<AtomicBool>) -> Result<(), String> {
    jobs.inner.lock().map_err(|_| "Không thể truy cập trạng thái video jobs.".to_string())?.insert(job_id.into(), VideoToolJobEntry { project_id: project_id.into(), cancelled });
    Ok(())
}

fn remove_job(jobs: &VideoToolsJobs, job_id: &str) {
    if let Ok(mut active) = jobs.inner.lock() { active.remove(job_id); }
}

fn emit_video_tool_event(app: &AppHandle, event: VideoToolEvent) {
    crate::job_service::sync_app_event(app, "video_tools", &event);
    let _ = app.emit("video-tool-progress", event);
}

fn validate_tiktok_url(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    let host = trimmed.strip_prefix("https://").or_else(|| trimmed.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next()).unwrap_or_default().split(':').next().unwrap_or_default().to_ascii_lowercase();
    if !(host == "tiktok.com" || host.ends_with(".tiktok.com")) || trimmed.len() > 4096 {
        return Err("Hãy nhập liên kết video/series thuộc tiktok.com.".into());
    }
    Ok(())
}

fn run_tiktok_worker(
    app: &AppHandle,
    worker: &Path,
    request: &Value,
    job_id: &str,
    project_id: &str,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let mut child = crate::runtime_paths::worker_command(worker)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|error| format!("Không thể khởi chạy downloader worker: {error}. Hãy kiểm tra Python worker và dependencies TikTok."))?;
    let stdout = child.stdout.take().ok_or_else(|| "Không mở được output của downloader.".to_string())?;
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || for line in BufReader::new(stdout).lines().flatten() { if tx.send(line).is_err() { break; } });
    let mut stdin = child.stdin.take().ok_or_else(|| "Không mở được input của downloader.".to_string())?;
    let mut bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).map_err(|error| format!("Không thể gửi yêu cầu tải TikTok: {error}"))?;
    drop(stdin);
    let mut terminal_received = false;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            kill_process_tree(&mut child);
            cleanup_tiktok_download_partials(request);
            return Err("cancelled".into());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let message: Value = match serde_json::from_str(&line) {
                    Ok(message) => message,
                    Err(error) => {
                        kill_process_tree(&mut child);
                        return Err(format!("Downloader worker trả JSON không hợp lệ: {error}"));
                    }
                };
                let kind = message["type"].as_str().unwrap_or_default();
                let status = match kind { "completed" => "completed", "failed" => "failed", "cancelled" => "cancelled", _ => "running" };
                let event = VideoToolEvent {
                    project_id: project_id.into(), job_id: job_id.into(), status: status.into(),
                    progress: message["progress"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0),
                    message: message["message"].as_str().unwrap_or(if kind == "info" { "Đã đọc thông tin TikTok." } else { "Đang tải video…" }).to_string(),
                    stage: message["stage"].as_str().map(str::to_owned).or_else(|| Some(kind.into())),
                    details: message.get("details").cloned(),
                    output: message["outputDirectory"].as_str().map(str::to_owned),
                };
                emit_video_tool_event(app, event);
                if kind == "failed" {
                    let _ = child.wait();
                    return Err(message["message"].as_str().unwrap_or("TikTok downloader gặp lỗi.").to_string());
                }
                if kind == "cancelled" { return Err("cancelled".into()); }
                if kind == "completed" { terminal_received = true; }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(status) = child.try_wait().map_err(|error| format!("Không kiểm tra được downloader worker: {error}"))? {
                    if !status.success() { return Err("Tải video đã dừng trước khi hoàn tất. Kiểm tra kết nối mạng, đăng nhập TikTok rồi thử lại.".into()); }
                    if terminal_received { return Ok(()); }
                    return Err("Tải video đã dừng trước khi hoàn tất.".into());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.wait();
    Err("Không nhận được kết quả tải video.".into())
}

fn cleanup_tiktok_download_partials(request: &Value) {
    let Some(directory) = request.get("outputDirectory").and_then(Value::as_str) else {
        return;
    };
    let Ok(root) = fs::canonicalize(directory) else { return };
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_file() || !is_tiktok_partial_name(&path) {
            continue;
        }
        let _ = fs::remove_file(path);
    }
}

fn is_tiktok_partial_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else { return false };
    let Some(rest) = name.strip_prefix(".EP") else { return false };
    let Some(rest) = rest.strip_suffix(".partial") else { return false };
    let Some((episode, token)) = rest.split_once('.') else { return false };
    episode.len() == 4
        && episode.bytes().all(|byte| byte.is_ascii_digit())
        && token.len() == 32
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(windows)]
fn kill_process_tree(child: &mut Child) {
    let pid = child.id().to_string();
    let mut command = Command::new("taskkill.exe");
    command.args(["/PID", &pid, "/T", "/F"]);
    use std::os::windows::process::CommandExt;
    let _ = command.creation_flags(0x0800_0000).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(windows))]
fn kill_process_tree(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn validate_merge_inputs(source_root: &Path, paths: &[String]) -> Result<Vec<MergeVideoInfo>, String> {
    if paths.len() > 500 { return Err("Một lần gộp hỗ trợ tối đa 500 video.".into()); }
    let mut seen = HashSet::new();
    let mut videos = Vec::with_capacity(paths.len());
    for value in paths {
        let raw = PathBuf::from(value);
        if !raw.is_absolute() { return Err("Danh sách video có đường dẫn không hợp lệ.".into()); }
        let metadata = fs::symlink_metadata(&raw).map_err(|error| format!("Không thể đọc video đã chọn: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || !is_supported_video(&raw) {
            return Err(format!("Tệp '{}' không phải video được hỗ trợ.", raw.file_name().unwrap_or_default().to_string_lossy()));
        }
        let canonical = raw.canonicalize().map_err(|error| format!("Không thể xác minh đường dẫn video: {error}"))?;
        if canonical.parent() != Some(source_root) { return Err("Có video nằm ngoài thư mục nguồn đã chọn.".into()); }
        if !seen.insert(canonical.clone()) { return Err("Danh sách video có mục bị trùng.".into()); }
        let info = probe_merge_video(&canonical)?;
        if info.file_name.contains(['\n', '\r']) { return Err("Tên video chứa ký tự không hỗ trợ.".into()); }
        videos.push(info);
    }
    Ok(videos)
}

fn run_merge_job(
    app: &AppHandle,
    store: &ProjectStore,
    project_id: &str,
    job_id: &str,
    source_root: &Path,
    output_directory: &Path,
    videos: &[MergeVideoInfo],
    logos: &[PreparedMergeLogo],
    texts: &[MergeTextSettings],
    total_duration: f64,
    frame_mode: MergeFrameMode,
    video_codec: MergeVideoCodec,
    encoder: MergeEncoder,
    cancelled: &AtomicBool,
) -> Result<PathBuf, String> {
    if cancelled.load(Ordering::SeqCst) { return Err("cancelled".into()); }
    emit_video_tool_event(app, VideoToolEvent { project_id: project_id.into(), job_id: job_id.into(), status: "waiting".into(), progress: 0.0, message: "Đang chờ tài nguyên FFmpeg…".into(), stage: Some("waiting".into()), details: None, output: None });
    let _ffmpeg_permit = loop {
        if cancelled.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        match crate::resource_scheduler::try_ffmpeg() {
            Ok(permit) => break permit,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(100)),
        }
    };
    let inputs = validate_merge_inputs(source_root, &videos.iter().map(|video| video.path.clone()).collect::<Vec<_>>())?;
    let output = next_merge_output(output_directory)?;
    let temporary = output_directory.join(format!(".avl-merge-{job_id}.partial.mp4"));
    let manifest = output_directory.join(format!(".avl-merge-{job_id}.concat.txt"));
    let filter_script = output_directory.join(format!(".avl-merge-{job_id}.filter.txt"));
    let text_ass_path = output_directory.join(format!(".avl-merge-{job_id}.texts.ass"));
    let mut wrote_text_ass = false;
    let result = (|| {
        if cancelled.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        let (target_width, target_height) = merge_canvas_size(&inputs, frame_mode)?;
        let copy_compatible = logos.is_empty() && texts.is_empty() && is_stream_copy_compatible(&inputs)
            && merge_codec_matches(&inputs[0].video_codec, video_codec)
            && (frame_mode == MergeFrameMode::Source || inputs.iter().all(|video| video.width == target_width && video.height == target_height));
        if copy_compatible {
            write_concat_manifest(&manifest, &inputs)?;
        } else {
            let ass_path = if texts.is_empty() { None } else {
                let specs = texts.iter().enumerate().map(|(index, text)| {
                    let rect = crate::render_spec::normalized_rect_to_pixels(text.x, text.y, text.width, text.height, target_width, target_height)?;
                    crate::render_spec::to_ass_color(&text.color, text.opacity)?;
                    let font_name = text.font_family.chars().filter(|c| !c.is_control() && !matches!(c, ',' | '\\' | '{' | '}')).take(80).collect::<String>().trim().to_string();
                    Ok(crate::render_spec::TextOverlayRenderSpec { id: format!("merge-text-{index}"), text: text.text.clone(), rect, enabled: true, font_name, font_size: text.font_size, font_weight: text.font_weight, color: text.color.clone(), opacity: text.opacity, alignment: text.alignment.clone() })
                }).collect::<Result<Vec<_>, String>>()?;
                let ass = crate::render_spec::generate_static_text_ass(target_width, target_height, total_duration, &specs)?;
                write_private_file(&text_ass_path, ass.as_bytes())?;
                wrote_text_ass = true;
                Some(text_ass_path.as_path())
            };
        let graph = build_transcode_filtergraph(&inputs, logos, target_width, target_height, ass_path)?;
        write_private_file(&filter_script, graph.as_bytes())?;
        }
        if !copy_compatible && encoder == MergeEncoder::Nvidia {
            crate::render_service::probe_nvidia_video_encoder(merge_codec_name(video_codec))?;
        }
        let codec_label = match video_codec { MergeVideoCodec::H264 => "H.264", MergeVideoCodec::Hevc => "HEVC" };
        let encoder_label = match encoder { MergeEncoder::Cpu => "CPU", MergeEncoder::Nvidia => "NVIDIA GPU · NVENC" };
        let message = if copy_compatible {
            "Các video tương thích; đang ghép bằng stream copy để giữ nguyên chất lượng.".to_string()
        } else {
            let action = if !logos.is_empty() && !texts.is_empty() { "Đang ghép video, chèn logo và chữ theo preview" }
                else if !logos.is_empty() { "Đang ghép video và chèn logo theo preview" }
                else if !texts.is_empty() { "Đang ghép video và chèn chữ theo preview" }
                else if frame_mode == MergeFrameMode::Source { "Định dạng khác nhau; đang chuẩn hóa khung hình và ghép" }
                else { "Đang căn giữa video trên nền đen theo tỷ lệ đã chọn rồi ghép" };
            format!("{action} · {codec_label} · {encoder_label}.")
        };
        emit_video_tool_event(app, VideoToolEvent {
            project_id: project_id.into(), job_id: job_id.into(), status: "running".into(), progress: 0.0,
            message,
            stage: Some(if copy_compatible { "copy" } else { "transcode" }.into()), details: None, output: None,
        });
        run_ffmpeg_merge(app, project_id, job_id, &inputs, logos, &manifest, &filter_script, &temporary, total_duration, copy_compatible, video_codec, encoder, cancelled)?;
        if cancelled.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        let (duration, has_video, has_audio, output_codec) = probe_merge_output(&temporary)?;
        if !has_video { return Err("File gộp không có video stream hợp lệ.".into()); }
        if !merge_codec_matches(&output_codec, video_codec) { return Err("Codec video đầu ra không khớp với lựa chọn H.264/HEVC.".into()); }
        if inputs.iter().any(|video| video.has_audio) && !has_audio { return Err("File gộp bị thiếu audio dù video nguồn có tiếng.".into()); }
        let tolerance = 2.0_f64.max(total_duration * 0.02);
        if (duration - total_duration).abs() > tolerance {
            return Err(format!("Thời lượng file gộp ({duration:.1}s) lệch quá nhiều so với tổng nguồn ({total_duration:.1}s)."));
        }
        // A project may have been removed while FFmpeg was running; do not report success
        // against a stale job or leave a file in a recreated project directory.
        store.get(project_id).map_err(|_| "Dự án đã bị xóa trong lúc gộp video.".to_string())?;
        fs::rename(&temporary, &output).map_err(|error| format!("Không thể hoàn tất file gộp: {error}"))?;
        Ok(output.clone())
    })();
    let _ = fs::remove_file(&manifest);
    let _ = fs::remove_file(&filter_script);
    if wrote_text_ass || result.is_err() { let _ = fs::remove_file(&text_ass_path); }
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

fn next_merge_output(directory: &Path) -> Result<PathBuf, String> {
    let stem = format!("merged_{}", Utc::now().format("%Y%m%d_%H%M%S"));
    for suffix in 1..=10_000u32 {
        let name = if suffix == 1 { format!("{stem}.mp4") } else { format!("{stem}_{suffix}.mp4") };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => { drop(file); let _ = fs::remove_file(&path); return Ok(path); }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Không thể chuẩn bị tên file gộp: {error}")),
        }
    }
    Err("Không tạo được tên file gộp duy nhất.".into())
}

fn is_stream_copy_compatible(videos: &[MergeVideoInfo]) -> bool {
    let Some(first) = videos.first() else { return false; };
    if first.other_stream_count > 0 || !matches!(first.stream_signature.len(), 1 | 2) { return false; }
    videos.iter().all(|video| video.other_stream_count == 0 && video.stream_signature == first.stream_signature)
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)
        .map_err(|error| format!("Không thể tạo file danh sách tạm: {error}"))?;
    file.write_all(contents).and_then(|_| file.sync_all())
        .map_err(|error| format!("Không thể ghi file danh sách tạm: {error}"))
}

fn write_concat_manifest(path: &Path, videos: &[MergeVideoInfo]) -> Result<(), String> {
    let mut content = String::from("ffconcat version 1.0\n");
    for video in videos {
        let normalized_path = video.path.replace('\\', "/");
        let escaped = normalized_path.replace('\'', "'\\''");
        content.push_str(&format!("file '{escaped}'\n"));
    }
    write_private_file(path, content.as_bytes())
}

fn merge_canvas_size(videos: &[MergeVideoInfo], frame_mode: MergeFrameMode) -> Result<(u32, u32), String> {
    let reference = videos.iter().max_by_key(|video| u64::from(video.width) * u64::from(video.height))
        .ok_or_else(|| "Không có video để gộp.".to_string())?;
    let even = |value: u32| value.saturating_add(value % 2).max(2);
    match frame_mode {
        MergeFrameMode::Source => Ok((even(reference.width), even(reference.height))),
        MergeFrameMode::Portrait | MergeFrameMode::Landscape => {
            let long_edge = videos.iter().map(|video| video.width.max(video.height)).max()
                .ok_or_else(|| "Không có video để gộp.".to_string())?;
            let long_edge = long_edge.saturating_add(long_edge % 2).max(2);
            let short_edge = (((long_edge as f64) * 9.0 / 16.0).round() as u32) & !1;
            if frame_mode == MergeFrameMode::Portrait { Ok((short_edge.max(2), long_edge)) }
            else { Ok((long_edge, short_edge.max(2))) }
        }
    }
}

fn build_transcode_filtergraph(videos: &[MergeVideoInfo], logos: &[PreparedMergeLogo], width: u32, height: u32, text_ass_path: Option<&Path>) -> Result<String, String> {
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return Err("Kích thước khung hình gộp không hợp lệ.".into());
    }
    let with_audio = videos.iter().any(|video| video.has_audio);
    let mut filters = Vec::new();
    for (index, video) in videos.iter().enumerate() {
        filters.push(format!(
            "[{index}:v:0]trim=duration={:.6},setpts=PTS-STARTPTS,scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=30,format=yuv420p[v{index}]",
            video.duration
        ));
        if with_audio {
            let duration = format!("{:.6}", video.duration);
            if video.has_audio {
                filters.push(format!("[{index}:a:0]asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad,atrim=duration={duration},asetpts=PTS-STARTPTS[a{index}]"));
            } else {
                filters.push(format!("anullsrc=channel_layout=stereo:sample_rate=48000,atrim=duration={duration},asetpts=PTS-STARTPTS[a{index}]"));
            }
        }
    }
    let mut concat_inputs = Vec::new();
    for index in 0..videos.len() {
        concat_inputs.push(format!("[v{index}]"));
        if with_audio { concat_inputs.push(format!("[a{index}]")); }
    }
    let has_post_processing = !logos.is_empty() || text_ass_path.is_some();
    let video_concat_label = if has_post_processing { "merge_base" } else { "outv" };
    filters.push(format!("{}concat=n={}:v=1:a={}[{}]{}",
        concat_inputs.concat(), videos.len(), if with_audio { 1 } else { 0 }, video_concat_label, if with_audio { "[outa]" } else { "" }));
    let mut current_video = video_concat_label.to_string();
    for (index, logo) in logos.iter().enumerate() {
        let rect = merge_logo_pixels(&logo.settings, width, height);
        let input_index = videos.len() + index;
        let layer = format!("merge_logo_layer{index}");
        let output = if index + 1 == logos.len() && text_ass_path.is_none() { "outv".to_string() } else { format!("merge_logo_composite{index}") };
        if logo.settings.maintain_aspect_ratio {
            filters.push(format!("[{input_index}:v:0]format=rgba,scale={}:{}:force_original_aspect_ratio=decrease:flags=lanczos,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black@0,colorchannelmixer=aa={:.4}[{layer}]", rect.2, rect.3, rect.2, rect.3, logo.settings.opacity));
        } else {
            filters.push(format!("[{input_index}:v:0]format=rgba,scale={}:{}:flags=lanczos,colorchannelmixer=aa={:.4}[{layer}]", rect.2, rect.3, logo.settings.opacity));
        }
        filters.push(format!("[{current_video}][{layer}]overlay={}:{}:eof_action=repeat:shortest=1[{output}]", rect.0, rect.1));
        current_video = output;
    }
    if let Some(path) = text_ass_path {
        let escaped_path = path.to_string_lossy().replace('\\', "/").replace(':', "\\:").replace('\'', "\\'").replace(',', "\\,").replace('[', "\\[").replace(']', "\\]");
        filters.push(format!("[{current_video}]subtitles=filename='{escaped_path}':original_size={width}x{height}[outv]"));
    }
    Ok(filters.join(";\n"))
}

fn merge_codec_matches(source_codec: &str, requested: MergeVideoCodec) -> bool {
    match requested {
        MergeVideoCodec::H264 => source_codec.eq_ignore_ascii_case("h264"),
        MergeVideoCodec::Hevc => source_codec.eq_ignore_ascii_case("hevc"),
    }
}

fn merge_codec_name(codec: MergeVideoCodec) -> &'static str {
    match codec { MergeVideoCodec::H264 => "h264", MergeVideoCodec::Hevc => "hevc" }
}

fn merge_logo_pixels(settings: &MergeLogoSettings, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let pixel_width = (settings.width * f64::from(width)).round().clamp(2.0, f64::from(width)) as u32;
    let pixel_height = (settings.height * f64::from(height)).round().clamp(2.0, f64::from(height)) as u32;
    let x = (settings.x * f64::from(width)).round().clamp(0.0, f64::from(width.saturating_sub(pixel_width))) as u32;
    let y = (settings.y * f64::from(height)).round().clamp(0.0, f64::from(height.saturating_sub(pixel_height))) as u32;
    (x, y, pixel_width, pixel_height)
}

fn run_ffmpeg_merge(
    app: &AppHandle,
    project_id: &str,
    job_id: &str,
    videos: &[MergeVideoInfo],
    logos: &[PreparedMergeLogo],
    manifest: &Path,
    filter_script: &Path,
    temporary_output: &Path,
    total_duration: f64,
    stream_copy: bool,
    video_codec: MergeVideoCodec,
    encoder: MergeEncoder,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let mut command = crate::runtime_paths::ffmpeg_command()?;
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-nostats", "-y", "-progress", "pipe:1"]);
    if stream_copy {
        command.args(["-f", "concat", "-safe", "0", "-i"]).arg(manifest)
            .args(["-map", "0:v:0"]);
        if videos[0].has_audio { command.args(["-map", "0:a:0"]); }
        command.args(["-c", "copy", "-movflags", "+faststart"]);
    } else {
        for video in videos { command.arg("-i").arg(&video.path); }
        for logo in logos { command.args(["-loop", "1", "-framerate", "30", "-i"]).arg(&logo.path); }
        command.args(["-filter_complex_script"]).arg(filter_script).args(["-map", "[outv]"]);
        if videos.iter().any(|video| video.has_audio) { command.args(["-map", "[outa]"]); }
        let (video_encoder, preset, quality_args): (&str, &str, &[&str]) = match (video_codec, encoder) {
            (MergeVideoCodec::H264, MergeEncoder::Cpu) => ("libx264", "veryfast", &["-crf", "19"]),
            (MergeVideoCodec::Hevc, MergeEncoder::Cpu) => ("libx265", "fast", &["-crf", "27"]),
            (MergeVideoCodec::H264, MergeEncoder::Nvidia) => ("h264_nvenc", "p5", &["-rc:v", "vbr", "-cq:v", "21", "-b:v", "0"]),
            (MergeVideoCodec::Hevc, MergeEncoder::Nvidia) => ("hevc_nvenc", "p5", &["-rc:v", "vbr", "-cq:v", "25", "-b:v", "0"]),
        };
        command.args(["-c:v", video_encoder, "-preset", preset]).args(quality_args).args(["-pix_fmt", "yuv420p"]);
        if video_codec == MergeVideoCodec::Hevc { command.args(["-tag:v", "hvc1"]); }
        if videos.iter().any(|video| video.has_audio) { command.args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2"]); }
        else { command.arg("-an"); }
        command.args(["-max_muxing_queue_size", "2048", "-movflags", "+faststart"]);
    }
    command.arg(temporary_output).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| format!("Không thể khởi chạy FFmpeg để gộp video: {error}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "Không mở được tiến trình FFmpeg.".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "Không đọc được lỗi FFmpeg.".to_string())?;
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || for line in BufReader::new(stdout).lines().flatten() { if tx.send(line).is_err() { break; } });
    let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let stderr_tail_thread = stderr_tail.clone();
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(mut tail) = stderr_tail_thread.lock() {
                        tail.push_back(line.trim().to_string());
                        while tail.len() > 12 { tail.pop_front(); }
                    }
                }
            }
        }
    });

    let mut out_time_seconds = 0.0_f64;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            kill_process_tree(&mut child);
            let _ = fs::remove_file(temporary_output);
            return Err("cancelled".into());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if let Some((key, value)) = line.split_once('=') {
                    match key {
                        "out_time_us" | "out_time_ms" => if let Ok(microseconds) = value.parse::<f64>() { out_time_seconds = microseconds / 1_000_000.0; },
                        "out_time" => if let Some(seconds) = parse_ffmpeg_timestamp(value) { out_time_seconds = seconds; },
                        "progress" => {
                            let progress = if value == "end" { 1.0 }
                                else { (out_time_seconds / total_duration).clamp(0.0, 0.99) };
                            emit_video_tool_event(app, VideoToolEvent {
                                project_id: project_id.into(), job_id: job_id.into(), status: "running".into(), progress,
                                message: format!("Đang ghép video · {}/{} · {:.0}%", format_time(out_time_seconds), format_time(total_duration), progress * 100.0),
                                stage: Some("render".into()), details: None, output: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected | mpsc::RecvTimeoutError::Timeout) => {
                if let Some(status) = child.try_wait().map_err(|error| format!("Không kiểm tra được FFmpeg: {error}"))? {
                    if !status.success() {
                        let details = stderr_tail.lock().map(|tail| tail.iter().rev().cloned().collect::<Vec<_>>().join(" ")).unwrap_or_default();
                        return Err(if details.is_empty() { "FFmpeg không thể gộp danh sách video đã chọn.".into() } else { format!("FFmpeg không thể gộp video: {}", details.chars().take(900).collect::<String>()) });
                    }
                    return Ok(());
                }
            }
        }
    }
}

fn parse_ffmpeg_timestamp(value: &str) -> Option<f64> {
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<f64>().ok()?;
    let minutes = parts.next()?.parse::<f64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn format_time(seconds: f64) -> String {
    let whole = seconds.max(0.0).floor() as u64;
    format!("{:02}:{:02}:{:02}", whole / 3600, (whole / 60) % 60, whole % 60)
}

fn probe_merge_output(path: &Path) -> Result<(f64, bool, bool, String), String> {
    if !path.is_file() || fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0) < 128 {
        return Err("FFmpeg không tạo được file đầu ra hợp lệ.".into());
    }
    let output = crate::runtime_paths::ffprobe_command()?
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,codec_name", "-of", "json"])
        .arg(path).output().map_err(|error| format!("Không chạy được FFprobe để kiểm tra video gộp: {error}"))?;
    if !output.status.success() { return Err("FFprobe không xác minh được file video gộp.".into()); }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| format!("Metadata video gộp không hợp lệ: {error}"))?;
    let duration = value["format"]["duration"].as_str().and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0).ok_or_else(|| "Video gộp không có thời lượng hợp lệ.".to_string())?;
    let streams = value["streams"].as_array().ok_or_else(|| "Video gộp không có stream hợp lệ.".to_string())?;
    let video = streams.iter().find(|stream| stream["codec_type"] == "video");
    let video_codec = video.and_then(|stream| stream["codec_name"].as_str()).unwrap_or_default().to_string();
    Ok((duration, video.is_some(), streams.iter().any(|stream| stream["codec_type"] == "audio"), video_codec))
}
