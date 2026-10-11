//! Bound FFmpeg inputs on long edits. Encode video once in frame-aligned batches;
//! concatenate packets, then encode the continuous master audio once (no AAC seams).
use super::{
    graph::{self, Graph},
    seconds, Plan,
};
use crate::{
    hardware_acceleration::{self, EncodingProfile},
    render_spec::ExportOptions,
    worker_manager::WorkerCommandExt,
};
use std::{
    collections::VecDeque,
    fs,
    io::BufReader,
    path::Path,
    process::Stdio,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub const MAX_LAYERS: usize = 32;
pub fn windows(plan: &Plan) -> Result<Vec<(u64, u64, u64)>, String> {
    let fps = plan.options.frame_rate;
    let period = 1_000_000u128 * fps.denominator as u128;
    let frames = ((plan.end - plan.start) as u128 * fps.numerator as u128).div_ceil(period) as u64;
    let timestamp =
        |frame: u64| plan.start + ((frame as u128 * period) / fps.numerator as u128) as u64;
    let clips = plan.visual_clips();
    let mut output = Vec::new();
    let mut at = 0;
    while at < frames {
        let mut count = ((30 * fps.numerator as u64) / fps.denominator as u64)
            .max(1)
            .min(frames - at);
        loop {
            let start = timestamp(at);
            let end = timestamp(at + count).min(plan.end);
            let layers = clips
                .iter()
                .filter(|c| c.start_us < end && c.end_us().is_ok_and(|v| v > start))
                .count();
            if layers <= MAX_LAYERS {
                output.push((start, end, count));
                break;
            }
            if count == 1 {
                return Err(format!("Quá {MAX_LAYERS} lớp hình trong một frame. Hãy ẩn hoặc gộp bớt lớp trước khi xuất."));
            }
            count = count.div_ceil(2);
        }
        at += count;
    }
    Ok(output)
}

fn run(
    mut command: std::process::Command,
    cancel: &impl Fn() -> bool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    command
        .args(["-progress", "pipe:1", "-nostats"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn_contained().map_err(|e| e.to_string())?;
    let stderr = child.stderr.take().ok_or("Thiếu stderr FFmpeg.")?;
    let stdout = child.stdout.take().ok_or("Thiếu tiến độ FFmpeg.")?;
    let (tx, rx) = mpsc::sync_channel(256);
    let updates = thread::spawn(move || {
        for line in
            crate::worker_manager::bounded_lines(BufReader::new(stdout), 8192).map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let shared = tail.clone();
    let reader = thread::spawn(move || {
        for line in crate::worker_manager::bounded_lines(BufReader::new(stderr), 65536)
            .map_while(Result::ok)
        {
            if let Ok(mut rows) = shared.lock() {
                rows.push_back(line);
                if rows.len() > 20 {
                    rows.pop_front();
                }
            }
        }
    });
    let mut last = Instant::now();
    let mut frames = 0;
    let result = loop {
        while let Ok(line) = rx.try_recv() {
            if let Some(frame) = line
                .strip_prefix("frame=")
                .and_then(|v| v.trim().parse::<u64>().ok())
            {
                if frame > frames {
                    frames = frame;
                    last = Instant::now();
                    progress(frames);
                }
            }
        }
        if cancel() || last.elapsed() > Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            break Err(if cancel() {
                "Đã hủy export.".into()
            } else {
                "FFmpeg không tiến thêm trong 10 phút.".into()
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err("FFmpeg không xuất được vùng timeline.".into())
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    drop(rx);
    let _ = updates.join();
    let _ = reader.join();
    result.map_err(|e: String| {
        format!(
            "{e}\n{}",
            tail.lock()
                .map(|rows| rows.iter().cloned().collect::<Vec<_>>().join("\n"))
                .unwrap_or_default()
        )
    })
}

pub fn build(
    plan: &Plan,
    options: &ExportOptions,
    root: &Path,
    master: Option<&Path>,
    cancel: &impl Fn() -> bool,
    mut progress: impl FnMut(f64, String),
) -> Result<Graph, String> {
    if plan.visual_clips().len() <= MAX_LAYERS {
        return graph::build(plan, root, master, cancel);
    }
    let windows = windows(plan)?;
    crate::disk_space::require(
        root,
        (plan.duration() * options.video_bitrate_kbps.unwrap_or(8000) as f64 * 160.0) as u64,
    )?;
    let fps = plan.options.frame_rate;
    let mut list = String::from("ffconcat version 1.0\n");
    let candidates = hardware_acceleration::fallback_candidates(
        options.video_codec,
        options.encoder,
        plan.encoder,
    )?;
    let mut backend = 0;
    for (i, &(from, to, frames)) in windows.iter().enumerate() {
        if cancel() {
            return Err("Đã hủy export.".into());
        }
        let directory = root.join(format!("batch-{i}"));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let mut window = plan.clone();
        window.start = from;
        window.end = to;
        let graph = graph::build(&window, &directory, None, cancel)?;
        debug_assert!(graph.inputs.len() <= MAX_LAYERS);
        let filter = directory.join("filtergraph.txt");
        fs::write(&filter, graph.filters).map_err(|e| e.to_string())?;
        let path = directory.join("video.mp4");
        loop {
            let encoder = candidates[backend];
            let mut cmd = crate::runtime_paths::ffmpeg_command()?;
            cmd.args([
                "-v",
                "warning",
                "-nostdin",
                "-y",
                "-filter_complex_threads",
                "1",
            ]);
            for input in &graph.inputs {
                cmd.args(input);
            }
            cmd.arg("-filter_complex_script")
                .arg(&filter)
                .args(["-map", "[vout]", "-an"]);
            cmd.args(hardware_acceleration::encoder_arguments(
                options.video_codec,
                encoder,
                &options.quality,
                options.video_bitrate_kbps,
                EncodingProfile::Export,
            )?);
            cmd.args(["-pix_fmt", "yuv420p", "-r"])
                .arg(format!("{}/{}", fps.numerator, fps.denominator))
                .arg("-frames:v")
                .arg(frames.to_string())
                .arg("-video_track_timescale")
                .arg(fps.numerator.to_string())
                .arg(&path);
            let message = format!(
                "Đang xuất vùng {}/{} · {}",
                i + 1,
                windows.len(),
                encoder.label()
            );
            match run(cmd, cancel, |frame| {
                let local = (frame as f64 * fps.denominator as f64 / fps.numerator as f64)
                    .min(seconds(to - from));
                progress(
                    ((seconds(from - plan.start) + local) / plan.duration()).min(1.0),
                    message.clone(),
                );
            }) {
                Ok(()) => break,
                Err(e)
                    if i == 0
                        && backend + 1 < candidates.len()
                        && !cancel()
                        && hardware_acceleration::is_encoder_initialization_error(&e) =>
                {
                    backend += 1;
                }
                Err(e) => return Err(e),
            }
        }
        // Only generated relative names enter concat syntax; arbitrary Unicode asset paths are argv.
        list.push_str(&format!(
            "file 'batch-{i}/video.mp4'\nduration {:.12}\n",
            frames as f64 * fps.denominator as f64 / fps.numerator as f64
        ));
        progress(
            (seconds(to - plan.start) / plan.duration()).min(1.0),
            format!(
                "Đang xuất vùng {}/{} · {}",
                i + 1,
                windows.len(),
                candidates[backend].label()
            ),
        );
    }
    let list_path = root.join("video-batches.ffconcat");
    fs::write(&list_path, list).map_err(|e| e.to_string())?;
    let mut inputs = vec![vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        graph::ffmpeg_path(&list_path).into_os_string(),
    ]];
    if let Some(master) = master {
        inputs.push(vec![
            "-i".into(),
            graph::ffmpeg_path(master).into_os_string(),
        ]);
    }
    Ok(Graph {
        inputs,
        filters: String::new(),
        audio: master.is_some(),
        copy_video: true,
    })
}
