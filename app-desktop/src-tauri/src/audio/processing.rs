use crate::worker_manager::WorkerCommandExt;
use super::{signal, ProcessingSettings, RATE};
use std::{fs, path::Path};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub version: u32,
    pub raw: signal::Signal,
    pub processed: signal::Signal,
    pub trimmed_duration: f64,
    pub slot_duration: f64,
    pub playback_duration: f64,
    pub tempo: f64,
    pub lead_seconds: f64,
    pub overflow: bool,
    pub settings: ProcessingSettings,
}
pub fn tempo(duration: f64, slot: f64, speed: f64, fit: bool, max: f64) -> (f64, bool) {
    let rate = if fit {
        (duration / slot.max(0.001)).clamp(1.0, max)
    } else {
        speed
    };
    (rate, duration / rate > slot + 0.02)
}
pub fn report(path: &Path) -> Option<Report> {
    let sidecar = path.with_extension("audio.json");
    signal::regular_file(&sidecar).ok()?;
    let r: Report = serde_json::from_slice(&fs::read(sidecar).ok()?).ok()?;
    if r.version != 2
        || r.settings.validate().is_err()
        || [
            r.raw.duration,
            r.processed.duration,
            r.slot_duration,
            r.playback_duration,
            r.tempo,
            r.lead_seconds,
        ]
        .iter()
        .any(|n| !n.is_finite() || *n < 0.0)
        || r.tempo < 0.5
        || r.tempo > 2.0
        || r.lead_seconds > 0.3
        || r.processed.sample_rate != RATE
        || r.processed.channels != 1
    {
        return None;
    }
    Some(r)
}
/// Old valid assets are resampled once, before opening the real-time mixer.
pub fn ingest_legacy(path: &Path, info: &signal::Signal) -> Result<std::path::PathBuf, String> {
    if info.sample_rate == RATE && info.channels == 1 {
        return Ok(path.to_owned());
    }
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut hash = Sha256::new();
    let mut input = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = [0; 16384];
    loop {
        let count = input.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    let output = path
        .parent()
        .ok_or("Thiếu thư mục audio.")?
        .join(format!("ingest-v2-{:x}.wav", hash.finalize()));
    if signal::inspect(&output).is_ok_and(|s| {
        s.sample_rate == RATE && s.channels == 1 && (s.duration - info.duration).abs() < 0.002
    }) {
        return Ok(output);
    }
    for p in [&output, &output.with_extension("audio.json")] {
        if fs::symlink_metadata(p).is_ok() {
            signal::regular_file(p)?;
            fs::remove_file(p).map_err(|e| e.to_string())?;
        }
    }
    process(
        path,
        &output,
        info.duration,
        0.0,
        1.0,
        false,
        &ProcessingSettings {
            head_padding_ms: 0,
            tail_padding_ms: 0,
            normalize: false,
            ..Default::default()
        },
        || false,
    )?;
    Ok(output)
}
/// Preserve every raw sample. Fades are confined to newly added silence.
pub fn process(
    raw: &Path,
    output: &Path,
    slot: f64,
    start: f64,
    speed: f64,
    fit: bool,
    settings: &ProcessingSettings,
    cancel: impl Fn() -> bool,
) -> Result<Report, String> {
    settings.validate()?;
    let info = signal::inspect(raw)?;
    let head = settings.head_padding_ms as f64 / 1000.0;
    let tail = settings.tail_padding_ms as f64 / 1000.0;
    let (mut rate, _) = tempo(info.duration, slot, speed, fit, settings.max_fit_speed);
    if fit && head / rate > start {
        rate =
            ((info.duration + head) / (slot + start).max(0.001)).clamp(1.0, settings.max_fit_speed);
    }
    let lead = (head / rate).min(start.max(0.0));
    let decoded = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile_in(output.parent().ok_or("Đường dẫn WAV không hợp lệ.")?)
        .map_err(|e| e.to_string())?;
    // Padding precedes WSOLA, protecting raw consonant attacks at sample zero.
    let mut filter = format!(
        "aresample=48000,aformat=channel_layouts=mono,adelay={:.3}:all=1,apad=pad_dur={tail:.8}",
        head * 1000.0
    );
    if head > 0.0 {
        filter.push_str(&format!(",afade=t=in:st=0:d={:.8}", head.min(0.01)));
    }
    if tail > 0.0 {
        filter.push_str(&format!(
            ",afade=t=out:st={:.8}:d={:.8}",
            head + info.duration + tail - tail.min(0.01),
            tail.min(0.01)
        ));
    }
    if (rate - 1.0).abs() > 0.001 {
        filter.push_str(&format!(",atempo={rate:.8}"));
    }
    let mut child = crate::runtime_paths::ffmpeg_command()?
        .args(["-v", "error", "-y", "-i"])
        .arg(raw)
        .args([
            "-af",
            &filter,
            "-ar",
            "48000",
            "-ac",
            "1",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(decoded.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn_contained()
        .map_err(|e| format!("Không xử lý được giọng đọc: {e}"))?;
    loop {
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Đã hủy xử lý giọng đọc.".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("Không xử lý được giọng đọc bằng FFmpeg.".into());
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let normalized = signal::inspect(decoded.path())?;
    let gain = if settings.normalize && normalized.rms > 0.0001 {
        (0.10 / normalized.rms)
            .clamp(0.5, 2.0)
            .min(0.92 / normalized.peak.max(0.001))
    } else {
        1.0
    };
    let temp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile_in(output.parent().unwrap())
        .map_err(|e| e.to_string())?;
    let mut writer = hound::WavWriter::create(
        temp.path(),
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut input = signal::open(decoded.path())?;
    while let Some(v) = signal::sample(&mut input)? {
        writer
            .write_sample(v * gain as f32)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    let processed = signal::inspect(temp.path())?;
    let result = Report {
        version: 2,
        raw: info.clone(),
        trimmed_duration: info.duration,
        slot_duration: slot,
        playback_duration: processed.duration,
        tempo: rate,
        lead_seconds: lead,
        overflow: processed.speech_end.unwrap_or(processed.duration) - lead > slot + 0.02,
        processed,
        settings: settings.clone(),
    };
    if cancel() {
        return Err("Đã hủy xử lý giọng đọc.".into());
    }
    if cfg!(debug_assertions) && std::env::var("JC_AUDIO_DEBUG").as_deref() == Ok("1") {
        for (suffix, path) in [("raw", raw), ("trimmed", raw), ("fitted", decoded.path())] {
            fs::copy(path, output.with_extension(format!("{suffix}.wav")))
                .map_err(|e| e.to_string())?;
        }
    }
    temp.persist_noclobber(output).map_err(|e| e.to_string())?;
    let sidecar =
        tempfile::NamedTempFile::new_in(output.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(
        sidecar.path(),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if let Err(e) = sidecar.persist_noclobber(output.with_extension("audio.json")) {
        let _ = fs::remove_file(output);
        return Err(e.to_string());
    }
    Ok(result)
}
