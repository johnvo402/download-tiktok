//! Deterministic CPU mixer. Preview and offline export call the same next_block.
use super::{signal, timeline::Voice, ProcessingSettings, BLOCK, RATE};
use crate::worker_manager::WorkerCommandExt;
use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Mix {
    pub original: bool,
    pub dubbing: bool,
    pub original_gain: f64,
    pub dub_gain: f64,
    pub reduction: f64,
    pub duck: f64,
    pub settings: ProcessingSettings,
}
impl Mix {
    pub fn project(p: &crate::project_service::DubbingSettings) -> Self {
        Self {
            original: !p.mute_original,
            dubbing: p.enabled,
            original_gain: p.original_volume,
            dub_gain: p.dubbed_volume,
            reduction: if p.dialogue_reduction_enabled {
                p.dialogue_reduction_amount
            } else {
                0.0
            },
            duck: if p.ducking_enabled && p.enabled {
                p.ducking_amount
            } else {
                0.0
            },
            settings: p.audio_processing.clone(),
        }
    }
    pub fn export(a: &crate::render_spec::RenderAudioConfig, settings: ProcessingSettings) -> Self {
        Self {
            original: a.include_original,
            dubbing: a.include_dubbing,
            original_gain: a.original_volume,
            dub_gain: a.dubbed_volume,
            reduction: if a.dialogue_reduction {
                a.dialogue_reduction_amount
            } else {
                0.0
            },
            duck: if a.ducking && a.include_dubbing {
                a.ducking_amount
            } else {
                0.0
            },
            settings,
        }
    }
}
/// Streaming FFmpeg decoder with a bounded two-block queue. Drop kills and joins it.
pub(crate) struct Decoder {
    rx: Option<mpsc::Receiver<Result<Vec<f32>, String>>>,
    child: Arc<Mutex<crate::worker_manager::ManagedWorker>>,
    reader: Option<thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}
impl Decoder {
    fn new(path: &Path, start: f64, channels: u16) -> Result<Self, String> {
        Self::timed(path, start, channels, None)
    }
    pub(crate) fn timed(
        path: &Path,
        start: f64,
        channels: u16,
        window: Option<(f64, f64)>,
    ) -> Result<Self, String> {
        Self::timed_stream(path, start, channels, window, None)
    }
    pub(crate) fn timed_stream(
        path: &Path,
        start: f64,
        channels: u16,
        window: Option<(f64, f64)>,
        stream: Option<u32>,
    ) -> Result<Self, String> {
        let mut command = crate::runtime_paths::ffmpeg_command()?;
        command
            .args(["-v", "error", "-nostdin", "-threads", "1", "-ss"])
            .arg(format!("{start:.9}"));
        if let Some((duration, _)) = window {
            command.arg("-t").arg(format!("{duration:.9}"));
        }
        command
            .arg("-i")
            .arg(path)
            .arg("-map")
            .arg(
                stream
                    .map(|i| format!("0:{i}"))
                    .unwrap_or_else(|| "0:a:0".into()),
            )
            .args(["-vn", "-ar", "48000", "-ac"])
            .arg(channels.to_string());
        if let Some((_, mut rate)) = window {
            if !rate.is_finite() || rate <= 0.0 {
                return Err("Tốc độ audio không hợp lệ.".into());
            }
            let mut filters = Vec::new();
            while rate > 2.0 {
                filters.push("atempo=2".to_string());
                rate /= 2.0;
            }
            while rate < 0.5 {
                filters.push("atempo=0.5".to_string());
                rate *= 2.0;
            }
            filters.push(format!("atempo={rate:.12}"));
            command.arg("-af").arg(filters.join(","));
        }
        let mut child = command
            .args(["-f", "f32le", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_contained()
            .map_err(|e| format!("Không giải mã được audio: {e}"))?;
        let mut stdout = child.stdout.take().ok_or("Không đọc được audio.")?;
        let mut stderr = child.stderr.take().ok_or("Không đọc được lỗi audio.")?;
        let child = Arc::new(Mutex::new(child));
        let process = child.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let cancelled = stopped.clone();
        let (tx, rx) = mpsc::sync_channel(2);
        let reader = thread::spawn(move || {
            // Drain diagnostics concurrently; retain only a bounded tail.
            let diagnostics = thread::spawn(move || {
                let mut bytes = [0_u8; 1024];
                let mut tail = Vec::new();
                while let Ok(n) = stderr.read(&mut bytes) {
                    if n == 0 {
                        break;
                    }
                    tail.extend_from_slice(&bytes[..n]);
                    if tail.len() > 4096 {
                        tail.drain(..tail.len() - 4096);
                    }
                }
                String::from_utf8_lossy(&tail).into_owned()
            });
            loop {
                let mut bytes = vec![0; BLOCK * channels as usize * 4];
                let mut count = 0;
                while count < bytes.len() {
                    match stdout.read(&mut bytes[count..]) {
                        Ok(0) => break,
                        Ok(n) => count += n,
                        Err(e) => {
                            let _ = tx.send(Err(format!("Lỗi đọc audio: {e}")));
                            return;
                        }
                    }
                }
                if count == 0 {
                    break;
                }
                if count % (4 * channels as usize) != 0 {
                    let _ = tx.send(Err("Audio giải mã bị thiếu mẫu.".into()));
                    break;
                }
                let values: Vec<f32> = bytes[..count]
                    .chunks_exact(4)
                    .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                    .collect();
                if values.iter().any(|v| !v.is_finite()) {
                    let _ = tx.send(Err("Audio chứa mẫu không hợp lệ.".into()));
                    break;
                }
                if tx.send(Ok(values)).is_err() {
                    break;
                }
            }
            let status = loop {
                let Ok(mut p) = process.lock() else {
                    break None;
                };
                if cancelled.load(Ordering::Acquire) {
                    let _ = p.kill();
                    break p.wait().ok();
                }
                match p.try_wait() {
                    Ok(Some(s)) => break Some(s),
                    Err(_) => break None,
                    _ => {}
                }
                drop(p);
                thread::sleep(Duration::from_millis(5));
            };
            let detail = diagnostics.join().unwrap_or_default();
            if !status.is_some_and(|s| s.success()) {
                let _ = tx.send(Err(format!("Không giải mã được audio: {}", detail.trim())));
            }
        });
        Ok(Self {
            rx: Some(rx),
            child,
            reader: Some(reader),
            stopped,
        })
    }
    pub(crate) fn block(&mut self, cancelled: &impl Fn() -> bool) -> Result<Vec<f32>, String> {
        let began = std::time::Instant::now();
        loop {
            if began.elapsed() > Duration::from_secs(15) {
                return Err("Giải mã âm thanh mất quá lâu.".into());
            }
            if cancelled() {
                return Err("Đã hủy xử lý audio.".into());
            }
            match self
                .rx
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(25))
            {
                Ok(v) => return v,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Vec::new()),
            }
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.rx.take(); // Unblock the producer before waiting for it.
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
enum VoiceReader {
    Wav(signal::Reader),
    Decode {
        decoder: Decoder,
        samples: std::collections::VecDeque<f32>,
    },
}
impl VoiceReader {
    fn new(v: &Voice, time: f64) -> Result<Self, String> {
        let mut reader = signal::open(&v.path)?;
        let spec = reader.spec();
        let offset = (time - v.start).max(0.0);
        if spec.sample_rate == RATE && spec.channels == 1 {
            reader
                .seek((offset * RATE as f64).round() as u32)
                .map_err(|e| e.to_string())?;
            Ok(Self::Wav(reader))
        } else {
            Ok(Self::Decode {
                decoder: Decoder::new(&v.path, offset, 1)?,
                samples: Default::default(),
            })
        }
    }
    fn next(&mut self, cancel: &impl Fn() -> bool) -> Result<f32, String> {
        match self {
            Self::Wav(r) => Ok(signal::sample(r)?.unwrap_or(0.0)),
            Self::Decode { decoder, samples } => {
                if samples.is_empty() {
                    samples.extend(decoder.block(cancel)?);
                }
                Ok(samples.pop_front().unwrap_or(0.0))
            }
        }
    }
}
pub fn duck_envelope(t: f64, intervals: &[(f64, f64)], attack: f64, release: f64) -> f64 {
    intervals
        .iter()
        .map(|&(start, end)| {
            if t < start {
                ((t - (start - attack)) / attack.max(0.00001)).clamp(0.0, 1.0)
            } else if t <= end {
                1.0
            } else {
                (1.0 - (t - end) / release.max(0.00001)).clamp(0.0, 1.0)
            }
        })
        .fold(0.0, f64::max)
}
pub fn limit(v: f64) -> f32 {
    let sign = v.signum();
    let value = v.abs();
    if value <= 0.9 {
        v as f32
    } else {
        (sign * (0.9 + 0.08 * ((value - 0.9) / 0.08).tanh())) as f32
    }
}
#[cfg(test)]
pub fn mix_sample(original: f32, dialogue: f32, dub: f32, mix: &Mix, duck: f64) -> f32 {
    limit(raw_sample(original, dialogue, dub, mix, duck))
}
fn raw_sample(original: f32, dialogue: f32, dub: f32, mix: &Mix, duck: f64) -> f64 {
    let bed = if mix.original {
        (original as f64 - dialogue as f64 * mix.reduction)
            * mix.original_gain
            * (1.0 - mix.duck * duck)
    } else {
        0.0
    };
    bed + if mix.dubbing {
        dub as f64 * mix.dub_gain
    } else {
        0.0
    }
}
pub struct Mixer {
    original: Option<Decoder>,
    dialogue: Option<Decoder>,
    voices: Vec<Voice>,
    readers: HashMap<usize, VoiceReader>,
    intervals: Vec<(f64, f64)>,
    pub mix: Mix,
    frame: u64,
    end: u64,
    warned_clipping: bool,
}
impl Mixer {
    pub fn new(
        source: Option<&Path>,
        stem: Option<&Path>,
        voices: Vec<Voice>,
        mix: Mix,
        start: f64,
        duration: f64,
    ) -> Result<Self, String> {
        if mix.original && mix.reduction > 0.0 && source.is_some() && stem.is_none() {
            return Err("Chưa có audio tách thoại. Hãy tạo audio tách thoại trước.".into());
        }
        let intervals = super::timeline::intervals(&voices);
        Ok(Self {
            original: source.map(|p| Decoder::new(p, start, 2)).transpose()?,
            dialogue: stem.map(|p| Decoder::new(p, start, 2)).transpose()?,
            voices,
            readers: HashMap::new(),
            intervals,
            mix,
            frame: (start * RATE as f64).round() as u64,
            end: (duration * RATE as f64).round() as u64,
            warned_clipping: false,
        })
    }
    pub fn next_block(&mut self, cancel: &impl Fn() -> bool) -> Result<Vec<f32>, String> {
        // Disabled voices must not resume a stale reader at a later timeline position.
        if !self.mix.dubbing {
            self.readers.clear();
        }
        if self.frame >= self.end {
            return Ok(vec![]);
        }
        let count = (self.end - self.frame).min(BLOCK as u64) as usize;
        let original = self
            .original
            .as_mut()
            .map(|d| d.block(cancel))
            .transpose()?
            .unwrap_or_default();
        let dialogue = self
            .dialogue
            .as_mut()
            .map(|d| d.block(cancel))
            .transpose()?
            .unwrap_or_default();
        let mut output = Vec::with_capacity(count * 2);
        let begin = self.frame as f64 / RATE as f64;
        let end = (self.frame + count as u64) as f64 / RATE as f64;
        let active: Vec<usize> = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.start < end && v.end > begin)
            .map(|(i, _)| i)
            .collect();
        if self.original.is_none()
            && self.dialogue.is_none()
            && (!self.mix.dubbing || active.is_empty())
        {
            self.frame += count as u64;
            self.readers.retain(|key, _| self.voices[*key].end > end);
            return Ok(vec![0.0; count * 2]);
        }
        let intervals: Vec<_> = self
            .intervals
            .iter()
            .copied()
            .filter(|&(s, e)| {
                s - self.mix.settings.duck_attack_ms as f64 / 1000.0 < end
                    && e + self.mix.settings.duck_release_ms as f64 / 1000.0 > begin
            })
            .collect();
        for index in 0..count {
            if index % 128 == 0 && cancel() {
                return Err("Đã hủy xử lý audio.".into());
            }
            let time = (self.frame + index as u64) as f64 / RATE as f64;
            let mut dub = 0.0;
            if self.mix.dubbing {
                for &key in &active {
                    let v = &self.voices[key];
                    if time < v.start || time >= v.end {
                        continue;
                    }
                    if !self.readers.contains_key(&key) {
                        self.readers.insert(key, VoiceReader::new(v, time)?);
                    }
                    dub += self.readers.get_mut(&key).unwrap().next(cancel)?;
                }
            }
            let duck = duck_envelope(
                time,
                &intervals,
                self.mix.settings.duck_attack_ms as f64 / 1000.0,
                self.mix.settings.duck_release_ms as f64 / 1000.0,
            );
            for channel in 0..2 {
                let value = raw_sample(
                    *original.get(index * 2 + channel).unwrap_or(&0.0),
                    *dialogue.get(index * 2 + channel).unwrap_or(&0.0),
                    dub,
                    &self.mix,
                    duck,
                );
                if value.abs() > 1.0 && !self.warned_clipping {
                    self.warned_clipping = true;
                    log::warn!("Audio mix exceeds full scale; applying master peak protection.");
                }
                output.push(limit(value));
            }
        }
        self.frame += count as u64;
        let time = self.frame as f64 / RATE as f64;
        self.readers.retain(|key, _| self.voices[*key].end > time);
        Ok(output)
    }
}
pub fn render(
    source: Option<&Path>,
    stem: Option<&Path>,
    voices: Vec<Voice>,
    mix: Mix,
    duration: f64,
    output: &Path,
    cancel: impl Fn() -> bool,
    mut progress: impl FnMut(f64),
) -> Result<(), String> {
    let temp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile_in(output.parent().ok_or("Thiếu thư mục audio.")?)
        .map_err(|e| e.to_string())?;
    let mut writer = hound::WavWriter::create(
        temp.path(),
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut mixer = Mixer::new(source, stem, voices, mix, 0.0, duration)?;
    let mut frames = 0_u64;
    loop {
        let block = mixer.next_block(&cancel)?;
        if block.is_empty() {
            break;
        }
        frames += block.len() as u64 / 2;
        for value in block {
            writer.write_sample(value).map_err(|e| e.to_string())?;
        }
        if frames % (BLOCK as u64 * 48) == 0 {
            progress(frames as f64 / RATE as f64 / duration);
        }
    }
    writer.finalize().map_err(|e| e.to_string())?;
    let validated = signal::open(temp.path())?;
    if validated.spec().sample_rate != RATE
        || validated.spec().channels != 2
        || validated.duration() == 0
        || (validated.duration() as f64 / RATE as f64 - duration).abs() > 1.0 / RATE as f64
    {
        return Err("Audio tổng không khớp thời lượng video.".into());
    }
    drop(validated);
    if cancel() {
        return Err("Đã hủy xử lý audio.".into());
    }
    temp.persist_noclobber(output).map_err(|e| e.to_string())?;
    Ok(())
}
