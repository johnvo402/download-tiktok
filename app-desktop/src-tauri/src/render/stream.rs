//! Bounded RGBA pipe bridge. FFmpeg supplies real PTS via showinfo; no intermediate frames.
use super::{
    frame::{byte_len, PixelFormat, VideoFrame},
    renderer::Renderer,
};
use crate::text_removal_service::AiTextRemovalPatch;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct StreamFailure {
    pub message: String,
    pub details: String,
    pub encoder: bool,
}
impl StreamFailure {
    fn render(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            details: String::new(),
            encoder: false,
        }
    }
}
pub struct Stats {
    pub frames: u64,
    pub seconds: f64,
    pub upload_bytes: u64,
    pub readback_bytes: u64,
    pub child_cpu_seconds: Option<f64>,
}
struct Process(Arc<Mutex<Child>>);
impl Process {
    fn cpu_seconds(&self) -> Option<f64> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let child = self.0.lock().ok()?;
            crate::media_gpu::windows::metrics::process_cpu_seconds(
                windows::Win32::Foundation::HANDLE(child.as_raw_handle()),
            )
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Ok(mut child) = self.0.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
struct Watchdog {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn rational(value: &str) -> Option<f64> {
    let (n, d) = value.split_once('/')?;
    let n = n.parse::<f64>().ok()?;
    let d = d.parse::<f64>().ok()?;
    let r = n / d;
    (r.is_finite() && r > 0.0).then_some(r)
}

/// Reject HDR/10-bit/rotation and obvious VFR before an SDR rawvideo bridge is used.
/// Actual decoded frame deltas are also checked during streaming; VFR never silently becomes CFR.
pub fn compatible_input(source: &Path) -> Result<f64, String> {
    let output = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=pix_fmt,color_transfer,avg_frame_rate,r_frame_rate:stream_side_data=rotation",
            "-of",
            "json",
        ])
        .arg(source)
        .output()
        .map_err(|_| "Không kiểm tra được định dạng video để render GPU.")?;
    if !output.status.success() {
        return Err("Không đọc được metadata video.".into());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "Metadata video không hợp lệ.")?;
    compatible_metadata(&json)
}
fn compatible_metadata(json: &serde_json::Value) -> Result<f64, String> {
    let stream = json["streams"]
        .as_array()
        .and_then(|s| s.first())
        .ok_or("Không có video stream.")?;
    let format = stream["pix_fmt"].as_str().unwrap_or("");
    if ![
        "yuv420p", "yuvj420p", "yuv422p", "yuvj422p", "yuv444p", "yuvj444p", "nv12", "rgb24",
        "bgr24", "rgba", "bgra", "gray",
    ]
    .contains(&format)
        || matches!(
            stream["color_transfer"].as_str(),
            Some("smpte2084" | "arib-std-b67")
        )
    {
        return Err("Video HDR/10-bit chưa hỗ trợ render GPU; cần chế độ tương thích.".into());
    }
    if stream["side_data_list"].as_array().is_some_and(|list| {
        list.iter()
            .any(|e| e["rotation"].as_f64().is_some_and(|r| r != 0.0))
    }) {
        return Err("Video có xoay khung hình cần chế độ tương thích.".into());
    }
    let fps = rational(stream["avg_frame_rate"].as_str().unwrap_or(""))
        .ok_or("Video không có frame rate xác định.")?;
    let nominal = rational(stream["r_frame_rate"].as_str().unwrap_or(""))
        .ok_or("Video không có frame rate xác định.")?;
    if (fps - nominal).abs() / fps > 0.001 {
        return Err("Video có frame rate biến đổi cần chế độ tương thích.".into());
    }
    Ok(fps)
}
pub fn decoder_command(source: &Path) -> Result<Command, String> {
    let mut command = crate::runtime_paths::ffmpeg_command()?;
    command
        .args(["-hide_banner", "-loglevel", "info", "-noautorotate", "-i"])
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            "setpts=PTS-STARTPTS,showinfo",
            "-fps_mode",
            "passthrough",
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    Ok(command)
}
pub(crate) fn pts_from_line(line: &str, time_base: Option<f64>) -> Option<f64> {
    if !line.contains("showinfo") {
        return None;
    }
    // showinfo's human-readable pts_time rounds long timestamps. Integer PTS
    // and the decoder time base preserve the actual frame interval for long videos.
    if let Some(base) = time_base {
        let pts = line
            .split("pts:")
            .nth(1)?
            .split_whitespace()
            .next()?
            .parse::<i64>()
            .ok()?;
        let value = pts as f64 * base;
        return value.is_finite().then_some(value);
    }
    let value = line
        .split("pts_time:")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value)
}
pub fn run(
    renderer: &mut Renderer,
    mut decoder: Command,
    mut encoder: Command,
    fps: f64,
    patches: &[AiTextRemovalPatch],
    state: &Arc<AtomicU8>,
    on_progress: impl Fn(f64, u64),
) -> Result<Stats, StreamFailure> {
    let started = Instant::now();
    encoder.stdin(Stdio::piped());
    let mut enc = encoder
        .spawn()
        .map_err(|e| StreamFailure::render(format!("Không khởi chạy được bộ mã hóa: {e}")))?;
    let mut input = enc
        .stdin
        .take()
        .ok_or_else(|| StreamFailure::render("Không gửi được frame đến bộ mã hóa."))?;
    let encerr = enc
        .stderr
        .take()
        .ok_or_else(|| StreamFailure::render("Không đọc được lỗi bộ mã hóa."))?;
    let encout = enc.stdout.take();
    let encoder = Process(Arc::new(Mutex::new(enc)));
    let mut dec = decoder
        .spawn()
        .map_err(|e| StreamFailure::render(format!("Không giải mã được video: {e}")))?;
    let mut decoded = dec
        .stdout
        .take()
        .ok_or_else(|| StreamFailure::render("Không đọc được frame nguồn."))?;
    let decerr = dec
        .stderr
        .take()
        .ok_or_else(|| StreamFailure::render("Không đọc được timestamp nguồn."))?;
    let decoder = Process(Arc::new(Mutex::new(dec)));
    let stop = Arc::new(AtomicBool::new(false));
    let stop_watch = stop.clone();
    let cancel = state.clone();
    let dec_watch = decoder.0.clone();
    let enc_watch = encoder.0.clone();
    let _watch = Watchdog {
        stop,
        thread: Some(thread::spawn(move || {
            while !stop_watch.load(Ordering::SeqCst) {
                if cancel.load(Ordering::SeqCst) == 2 {
                    for process in [&dec_watch, &enc_watch] {
                        if let Ok(mut child) = process.lock() {
                            let _ = child.kill();
                        }
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(30));
            }
        })),
    };
    let tail = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let err_tail = tail.clone();
    let errors = thread::spawn(move || {
        for line in BufReader::new(encerr).lines().map_while(Result::ok) {
            if let Ok(mut tail) = err_tail.lock() {
                tail.push_back(line);
                if tail.len() > 24 {
                    tail.pop_front();
                }
            }
        }
    });
    if let Some(output) = encout {
        thread::spawn(
            move || {
                for _ in BufReader::new(output).lines().map_while(Result::ok) {}
            },
        );
    }
    let (time_tx, time_rx) = mpsc::sync_channel(8);
    let decoder_tail = Arc::new(Mutex::new(String::new()));
    let decoder_log = decoder_tail.clone();
    thread::spawn(move || {
        let mut time_base = None;
        for line in BufReader::new(decerr).lines().map_while(Result::ok) {
            if let Some(config) = line.split("config in time_base:").nth(1) {
                time_base = config
                    .split_whitespace()
                    .next()
                    .and_then(|v| rational(v.trim_end_matches(',')));
            }
            if let Some(pts) = pts_from_line(&line, time_base) {
                if time_tx.send(pts).is_err() {
                    break;
                }
            } else if !line.contains("showinfo") && line.len() < 4096 {
                if let Ok(mut tail) = decoder_log.lock() {
                    *tail = line;
                }
            }
        }
    });
    let len =
        byte_len(renderer.scene.width, renderer.scene.height).map_err(StreamFailure::render)?;
    let (frames_tx, frames_rx) = mpsc::sync_channel(1);
    let (pool_tx, pool_rx) = mpsc::sync_channel(2);
    pool_tx.send(vec![0_u8; len]).unwrap();
    pool_tx.send(vec![0_u8; len]).unwrap();
    thread::spawn(move || {
        while let Ok(mut buffer) = pool_rx.recv() {
            let mut read = 0;
            let mut error = None;
            while read < len {
                match decoded.read(&mut buffer[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        error = Some(e.to_string());
                        break;
                    }
                }
            }
            if read == 0 {
                break;
            }
            if read != len || error.is_some() {
                let _ = frames_tx.send(Err(
                    error.unwrap_or_else(|| "Frame nguồn bị thiếu dữ liệu.".into())
                ));
                break;
            }
            if frames_tx.send(Ok(buffer)).is_err() {
                break;
            }
        }
    });
    let mut output = vec![0_u8; len];
    let mut frames = 0_u64;
    let mut previous = None;
    let mut last_progress = Instant::now();
    let mut patch_cache: Vec<_> = patches
        .iter()
        .map(crate::text_removal_cache::PatchReader::open)
        .collect::<Result<_, _>>()
        .map_err(StreamFailure::render)?;
    renderer.clear_patches();
    let mut patch_rects=Vec::new();
    let mut patch_layers=Vec::new();
    for patch in patches {
        let layer=if let Some(i)=patch_rects.iter().position(|r|*r==patch.rect){patch_layers[i]}else{
            renderer.register_patch(patch.rect).map_err(StreamFailure::render)?
        };
        patch_rects.push(patch.rect);patch_layers.push(layer);
    }
    let mut encode_failed = false;
    loop {
        if state.load(Ordering::SeqCst) == 2 {
            return Err(StreamFailure::render("Đã hủy export."));
        }
        let buffer = match frames_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(data)) => data,
            Ok(Err(error)) => return Err(StreamFailure::render(error)),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if encoder
                    .0
                    .lock()
                    .map_err(|_| StreamFailure::render("Không đọc được trạng thái mã hóa."))?
                    .try_wait()
                    .map_err(|_| StreamFailure::render("Không kiểm tra được bộ mã hóa."))?
                    .is_some()
                {
                    encode_failed = true;
                    break;
                }
                continue;
            }
        };
        let pts = loop {
            match time_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(pts) => break pts,
                Err(mpsc::RecvTimeoutError::Timeout) if state.load(Ordering::SeqCst) != 2 => {
                    continue
                }
                _ => {
                    return Err(StreamFailure::render(
                        "Không đọc được timestamp video hoặc tác vụ đã hủy.",
                    ))
                }
            }
        };
        if let Some(last) = previous {
            let delta: f64 = pts - last;
            if delta <= 0.0 || (delta - 1.0 / fps).abs() > 0.0025 {
                return Err(StreamFailure::render(
                    "Video có timestamp biến đổi cần chế độ tương thích để giữ đồng bộ.",
                ));
            }
        }
        previous = Some(pts);
        let mut active = Vec::new();
        for (index, cache) in patch_cache.iter_mut().enumerate() {
            if let Some(pixels) = cache.frame(pts).map_err(StreamFailure::render)? {
                active.push((patch_layers[index], pixels));
            }
        }
        renderer
            .render_frame_with_patches(
                VideoFrame {
                    width: renderer.scene.width,
                    height: renderer.scene.height,
                    pts_seconds: pts,
                    format: PixelFormat::Rgba8,
                    data: &buffer,
                },
                &active,
                &mut output,
            )
            .map_err(StreamFailure::render)?;
        if input.write_all(&output).is_err() {
            encode_failed = true;
            break;
        }
        frames += 1;
        let _ = pool_tx.send(buffer);
        if last_progress.elapsed() >= Duration::from_millis(250) {
            on_progress(pts, frames);
            last_progress = Instant::now();
        }
    }
    drop(input);
    let last_activity = Instant::now();
    let status = loop {
        if state.load(Ordering::SeqCst) == 2 {
            return Err(StreamFailure::render("Đã hủy export."));
        }
        if let Some(status) = encoder
            .0
            .lock()
            .map_err(|_| StreamFailure::render("Không đọc được trạng thái mã hóa."))?
            .try_wait()
            .map_err(|_| StreamFailure::render("Không kiểm tra được bộ mã hóa."))?
        {
            break status;
        }
        if last_activity.elapsed() > Duration::from_secs(120) {
            return Err(StreamFailure::render(
                "Bộ mã hóa không hoàn thành video trong thời gian chờ.",
            ));
        }
        thread::sleep(Duration::from_millis(50));
        // The timeout intentionally bounds final mux; no unbounded wait().
    };
    let _ = errors.join();
    if !status.success() || encode_failed {
        return Err(StreamFailure {
            message: "Bộ mã hóa GPU không xuất được video.".into(),
            details: tail
                .lock()
                .map(|v| v.iter().cloned().collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
            encoder: true,
        });
    }
    // EOF on stdout can precede the process exit by a few milliseconds.
    let decoder_deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if state.load(Ordering::SeqCst) == 2 {
            return Err(StreamFailure::render("Đã hủy export."));
        }
        let status = decoder
            .0
            .lock()
            .map_err(|_| StreamFailure::render("Không đọc được trạng thái giải mã."))?
            .try_wait()
            .map_err(|_| StreamFailure::render("Không kiểm tra được giải mã."))?;
        if status.is_some() || Instant::now() >= decoder_deadline {
            break status;
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.is_some_and(|s| s.success()) || frames == 0 {
        return Err(StreamFailure {
            message: "Không giải mã đầy đủ video nguồn.".into(),
            details: decoder_tail.lock().map(|v| v.clone()).unwrap_or_default(),
            encoder: false,
        });
    }
    on_progress(previous.unwrap_or(0.0), frames);
    let seconds = started.elapsed().as_secs_f64();
    log::info!("GPU render stream: adapter={}; resolution={}x{}; frames={frames}; elapsed={seconds:.3}s; fps={:.2}; ms/frame={:.3}",renderer.adapter_name(),renderer.scene.width,renderer.scene.height,frames as f64/seconds,seconds*1000.0/frames.max(1) as f64);
    let bytes = frames * renderer.scene.width as u64 * renderer.scene.height as u64 * 4;
    log::info!("Media pipeline: Tier C; frames={frames}; full-frame upload_bytes={bytes}; readback_bytes={bytes}; CPU RGBA staging");
    Ok(Stats {
        frames,
        seconds,
        upload_bytes: bytes,
        readback_bytes: bytes,
        child_cpu_seconds: decoder
            .cpu_seconds()
            .zip(encoder.cpu_seconds())
            .map(|(a, b)| a + b),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_come_from_decoder() {
        assert_eq!(
            pts_from_line(
                "[Parsed_showinfo_1] n: 2 pts: 700 pts_time:0.073 size:320x180",
                None
            ),
            Some(0.073)
        );
        assert_eq!(pts_from_line("unrelated pts_time:4", None), None);
        let precise = pts_from_line(
            "[Parsed_showinfo] n: 90000 pts: 90000001 pts_time:3000 size:1920x1080",
            Some(1.0 / 30000.0),
        )
        .unwrap();
        assert!((precise - 3000.000033333333).abs() < 1e-9);
    }
    #[test]
    fn sdr_cfr_only_without_silent_hdr_conversion() {
        let mut input = serde_json::json!({"streams":[{"pix_fmt":"yuv420p","avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001"}]});
        assert!((compatible_metadata(&input).unwrap() - 29.97).abs() < 0.01);
        input["streams"][0]["color_transfer"] = serde_json::json!("smpte2084");
        assert!(compatible_metadata(&input).is_err());
        input["streams"][0]["color_transfer"] = serde_json::json!("bt709");
        input["streams"][0]["avg_frame_rate"] = serde_json::json!("25/1");
        assert!(compatible_metadata(&input).is_err());
    }
}
