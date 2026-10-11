use super::{audio, batches, Snapshot};
use crate::{
    hardware_acceleration::{self, EncodingProfile, VideoEncoder},
    render_service::{JOB_CANCELLED, JOB_COMMITTED, JOB_RUNNING},
    render_spec::ExportOptions,
    worker_manager::WorkerCommandExt,
};
use std::{
    collections::VecDeque,
    fs,
    io::BufReader,
    process::Stdio,
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

/// Publishes only a verified, synced MP4, with the same cancellation/commit boundary as source export.
pub fn render(
    snapshot: &Snapshot,
    options: &ExportOptions,
    state: &AtomicU8,
    mut progress: impl FnMut(f64, Option<f64>, String),
) -> Result<(String, f64), String> {
    let plan = &snapshot.plan;
    let duration = plan.duration();
    let cancelled = || state.load(Ordering::SeqCst) == JOB_CANCELLED;
    if cancelled() {
        return Err("Đã hủy export.".into());
    }
    crate::disk_space::require(
        &snapshot.output_directory,
        (duration * options.video_bitrate_kbps.unwrap_or(8000) as f64 * 250.0) as u64,
    )?;
    progress(
        0.0,
        None,
        "Đang trộn âm thanh theo snapshot timeline…".into(),
    );
    let master = audio::render(plan, snapshot.directory.path(), &cancelled, |p| {
        progress(p * 0.08, None, "Đang trộn âm thanh theo từng clip…".into())
    })?;
    progress(
        0.08,
        None,
        "Đang dựng các lớp của snapshot timeline…".into(),
    );
    let graph = batches::build(
        plan,
        options,
        snapshot.directory.path(),
        master.as_deref(),
        &cancelled,
        |p, message| progress(0.08 + p * 0.84, None, message),
    )?;
    if cancelled() {
        return Err("Đã hủy export.".into());
    }
    let filter = snapshot.directory.path().join("filtergraph.txt");
    fs::write(&filter, &graph.filters).map_err(|e| e.to_string())?;
    let prefix = snapshot
        .journal
        .as_ref()
        .map(|(id, _)| format!(".avl-render-{id}-"))
        .unwrap_or_else(|| ".avl-timeline-".into());
    let temp = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".partial.mp4")
        .tempfile_in(&snapshot.output_directory)
        .map_err(|e| format!("Không tạo được file export tạm: {e}"))?
        .into_temp_path();
    if let Some((id, directory)) = &snapshot.journal {
        crate::recovery_store::write(
            &directory.join("temporary-output.json"),
            &serde_json::to_vec(
                &serde_json::json!({"jobId":id,"outputPath":temp.to_string_lossy()}),
            )
            .map_err(|e| e.to_string())?,
        )?;
    }
    let candidates = hardware_acceleration::fallback_candidates(
        options.video_codec,
        options.encoder,
        plan.encoder,
    )?;
    for (attempt, backend) in candidates.iter().copied().enumerate() {
        if cancelled() {
            return Err("Đã hủy export.".into());
        }
        let mut cmd = crate::runtime_paths::ffmpeg_command()?;
        cmd.args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-loglevel",
            "warning",
            "-filter_complex_threads",
            "1",
        ]);
        for args in &graph.inputs {
            cmd.args(args);
        }
        if graph.copy_video {
            cmd.args(["-map", "0:v:0"]);
        } else {
            cmd.arg("-filter_complex_script")
                .arg(&filter)
                .args(["-map", "[vout]"]);
        }
        if graph.audio {
            cmd.args([
                "-map",
                if graph.copy_video { "1:a:0" } else { "[aout]" },
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-ar",
                "48000",
            ]);
        } else {
            cmd.arg("-an");
        }
        if graph.copy_video {
            cmd.args(["-c:v", "copy"]);
        } else {
            cmd.args(hardware_acceleration::encoder_arguments(
                options.video_codec,
                backend,
                &options.quality,
                options.video_bitrate_kbps,
                EncodingProfile::Export,
            )?);
            cmd.args(["-pix_fmt", "yuv420p", "-r"]).arg(format!(
                "{}/{}",
                plan.options.frame_rate.numerator, plan.options.frame_rate.denominator
            ));
        }
        cmd.arg("-t")
            .arg(format!("{duration:.9}"))
            .args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
            .arg(&temp)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn_contained()
            .map_err(|e| format!("Không chạy được FFmpeg: {e}"))?;
        let (tx, rx) = mpsc::sync_channel(256);
        let stdout = child.stdout.take().ok_or("Thiếu tiến độ FFmpeg.")?;
        let reader = thread::spawn(move || {
            for line in crate::worker_manager::bounded_lines(BufReader::new(stdout), 8192)
                .map_while(Result::ok)
            {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let shared = tail.clone();
        let stderr = child.stderr.take().ok_or("Thiếu stderr FFmpeg.")?;
        let errors = thread::spawn(move || {
            for line in crate::worker_manager::bounded_lines(BufReader::new(stderr), 65536)
                .map_while(Result::ok)
            {
                if let Ok(mut t) = shared.lock() {
                    t.push_back(line);
                    if t.len() > 20 {
                        t.pop_front();
                    }
                }
            }
        });
        let mut frames = 0u64;
        let mut at = 0f64;
        let mut speed = None;
        let mut last = Instant::now();
        let mut emit = Instant::now();
        let status = loop {
            if cancelled() || last.elapsed() > Duration::from_secs(600) {
                let _ = child.kill();
                let _ = child.wait();
                drop(rx);
                let _ = reader.join();
                let _ = errors.join();
                return Err(if cancelled() {
                    "Đã hủy export."
                } else {
                    "FFmpeg không tiến thêm trong 10 phút. Hãy kiểm tra nguồn và thử lại."
                }
                .into());
            }
            while let Ok(line) = rx.try_recv() {
                if let Some((key, value)) = line.split_once('=') {
                    match key {
                        "frame" => {
                            let next = value.trim().parse().unwrap_or(frames);
                            if next > frames {
                                last = Instant::now();
                            }
                            frames = next;
                        }
                        "out_time_us" => {
                            let next = value.parse::<f64>().unwrap_or(0.0) / 1e6;
                            if next > at {
                                last = Instant::now();
                            }
                            at = next;
                        }
                        "speed" => speed = value.trim_end_matches('x').parse().ok(),
                        "progress" if emit.elapsed() >= Duration::from_millis(250) => {
                            progress(
                                if graph.copy_video {
                                    0.92 + (at / duration).clamp(0.0, 0.99) * 0.075
                                } else {
                                    0.08 + (at / duration).clamp(0.0, 0.99) * 0.915
                                },
                                speed,
                                format!("Đang xuất timeline · {}", backend.label()),
                            );
                            emit = Instant::now();
                        }
                        _ => {}
                    }
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(Duration::from_millis(40)),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    drop(rx);
                    let _ = reader.join();
                    let _ = errors.join();
                    return Err(e.to_string());
                }
            }
        };
        drop(rx);
        let _ = reader.join();
        let _ = errors.join();
        if status.success() {
            break;
        }
        let details = tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if options.encoder == VideoEncoder::Auto
            && frames == 0
            && attempt + 1 < candidates.len()
            && hardware_acceleration::is_encoder_initialization_error(&details)
        {
            progress(
                0.0,
                None,
                format!(
                    "{} không khởi tạo được; đang thử {}…",
                    backend.label(),
                    candidates[attempt + 1].label()
                ),
            );
            continue;
        }
        log::error!("Timeline export FFmpeg failed: {details}");
        return Err(format!(
            "{}\n{details}",
            hardware_acceleration::encoding_error(options.video_codec, backend, &details)
        ));
    }
    if cancelled() {
        return Err("Đã hủy export.".into());
    }
    progress(
        0.995,
        None,
        "Đang xác minh video, thời lượng và âm thanh…".into(),
    );
    crate::render_service::validate_output(
        &temp,
        plan.options.width,
        plan.options.height,
        duration,
        graph.audio,
        options.video_codec,
    )?;
    // Check CFR as well as dimensions/codec/duration/audio.
    let probe = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate,duration",
            "-of",
            "json",
        ])
        .arg(&temp)
        .output_contained(Duration::from_secs(30))
        .map_err(|e| e.to_string())?;
    let data: serde_json::Value =
        serde_json::from_slice(&probe.stdout).map_err(|e| e.to_string())?;
    let rate = data["streams"][0]["r_frame_rate"]
        .as_str()
        .and_then(|s| s.split_once('/'))
        .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
        .ok_or("Không xác minh được FPS export.")?;
    let wanted =
        plan.options.frame_rate.numerator as f64 / plan.options.frame_rate.denominator as f64;
    if (rate - wanted).abs() > 0.001 {
        return Err("FPS video xuất không khớp cấu hình.".into());
    }
    let video_duration = data["streams"][0]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or("Thiếu thời lượng stream video.")?;
    if !video_duration.is_finite() || (video_duration - duration).abs() > 1.0 / wanted + 0.002 {
        return Err("Thời lượng video lệch hơn một frame so với snapshot timeline.".into());
    }
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temp)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    if state
        .compare_exchange(
            JOB_RUNNING,
            JOB_COMMITTED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        return Err("Đã hủy export.".into());
    }
    let final_path = crate::render_service::unique_output_path(
        &snapshot.output_directory,
        &options.output_file_name,
    );
    temp.persist_noclobber(&final_path)
        .map_err(|e| format!("Không lưu được video mà không ghi đè: {}", e.error))?;
    Ok((final_path.to_string_lossy().into_owned(), duration))
}
