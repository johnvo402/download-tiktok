//! Windows audio output. The device callback only consumes a bounded PCM queue.
use super::{
    mixer::{Mix, Mixer},
    timeline::Voice,
    BLOCK, RATE,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub session_id: String,
    pub project_id: String,
    pub time: f64,
    pub playing: bool,
    pub buffering: bool,
    pub error: Option<String>,
    pub queued_frames: usize,
}
struct Desired {
    playing: bool,
    seek: f64,
    revision: u64,
    mix: Mix,
}
pub struct Session {
    id: String,
    project: String,
    duration: f64,
    desired: Mutex<Desired>,
    ring: Mutex<VecDeque<f32>>,
    stopped: AtomicBool,
    audible: AtomicBool,
    frames: AtomicU64,
    base: AtomicU64,
    latency: AtomicU64,
    volume: AtomicU32,
    error: Mutex<Option<String>>,
    buffering: AtomicBool,
}
impl Session {
    fn time(&self) -> f64 {
        // base/frames/latency are one clock snapshot; seek resets them together.
        let _snapshot = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        f64::from_bits(self.base.load(Ordering::Acquire))
            + self
                .frames
                .load(Ordering::Acquire)
                .saturating_sub(self.latency.load(Ordering::Acquire)) as f64
                / RATE as f64
    }
    #[cfg(any(windows, test))]
    fn finish(&self, revision: u64) -> bool {
        let Ok(mut desired) = self.desired.lock() else {
            return false;
        };
        if desired.revision != revision {
            return false;
        }
        let Ok(ring) = self.ring.lock() else {
            return false;
        };
        if !ring.is_empty() {
            return false;
        }
        desired.playing = false;
        self.audible.store(false, Ordering::Release);
        self.base.store(self.duration.to_bits(), Ordering::Release);
        self.frames.store(0, Ordering::Release);
        self.latency.store(0, Ordering::Release);
        true
    }
    pub fn status(&self) -> Status {
        Status {
            session_id: self.id.clone(),
            project_id: self.project.clone(),
            time: self.time().min(self.duration),
            playing: self.desired.lock().map(|d| d.playing).unwrap_or(false)
                && self.error.lock().map(|e| e.is_none()).unwrap_or(false),
            buffering: self.buffering.load(Ordering::Acquire),
            error: self.error.lock().ok().and_then(|e| e.clone()),
            queued_frames: self.ring.lock().map(|r| r.len() / 2).unwrap_or(0),
        }
    }
    fn control(
        &self,
        action: &str,
        time: Option<f64>,
        volume: Option<f32>,
        mix: Option<Mix>,
    ) -> Result<(), String> {
        let mut desired = self
            .desired
            .lock()
            .map_err(|_| "Không điều khiển được âm thanh.")?;
        match action {
            "play" => {
                desired.playing = true;
                self.buffering.store(
                    self.ring.lock().map(|r| r.is_empty()).unwrap_or(true),
                    Ordering::Release,
                );
            }
            "pause" => {
                desired.playing = false;
                self.audible.store(false, Ordering::Release);
            }
            "seek" => {
                let time = time
                    .filter(|v| v.is_finite() && *v >= 0.0 && *v <= self.duration)
                    .ok_or("Thời gian seek không hợp lệ.")?;
                desired.seek = time;
                desired.revision += 1;
                self.audible.store(false, Ordering::Release);
                let mut ring = self.ring.lock().map_err(|_| "Không xóa được audio cũ.")?;
                ring.clear();
                self.buffering.store(true, Ordering::Release);
                self.base.store(time.to_bits(), Ordering::Release);
                self.frames.store(0, Ordering::Release);
                self.latency.store(0, Ordering::Release);
            }
            "volume" => {
                let gain = volume
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    .ok_or("Âm lượng không hợp lệ.")?;
                self.volume.store(gain.to_bits(), Ordering::Release);
            }
            "settings" => {
                desired.mix = mix.ok_or("Thiếu cấu hình âm thanh.")?;
            }
            _ => return Err("Thao tác âm thanh không hợp lệ.".into()),
        }
        Ok(())
    }
}
struct Owned {
    session: Arc<Session>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.session.stopped.store(true, Ordering::Release);
        self.session.audible.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Default)]
pub struct AudioManager {
    active: Mutex<Option<Owned>>,
    transition: Mutex<()>,
}
impl AudioManager {
    pub fn open(
        &self,
        project: String,
        source: Option<PathBuf>,
        stem: Option<PathBuf>,
        voices: Vec<Voice>,
        mix: Mix,
        duration: f64,
    ) -> Result<Status, String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "Không chuyển được phiên audio.")?;
        self.close_inner(None, None);
        let session = Arc::new(Session {
            id: uuid::Uuid::new_v4().to_string(),
            project,
            duration,
            desired: Mutex::new(Desired {
                playing: false,
                seek: 0.0,
                revision: 0,
                mix,
            }),
            ring: Mutex::new(VecDeque::with_capacity(BLOCK * 8)),
            stopped: AtomicBool::new(false),
            audible: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            base: AtomicU64::new(0_f64.to_bits()),
            latency: AtomicU64::new(0),
            volume: AtomicU32::new(1_f32.to_bits()),
            error: Mutex::new(None),
            buffering: AtomicBool::new(false),
        });
        let state = session.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let result = run(state.clone(), source, stem, voices, tx);
            if let Err(e) = result {
                log::warn!("Audio preview stopped: {e}");
                if let Ok(mut err) = state.error.lock() {
                    *err = Some(e);
                }
            }
            state.audible.store(false, Ordering::Release);
        });
        let owned = Owned {
            session,
            worker: Some(worker),
        };
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {
                let status = owned.session.status();
                *self
                    .active
                    .lock()
                    .map_err(|_| "Không mở được audio preview.")? = Some(owned);
                Ok(status)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Không khởi động được thiết bị âm thanh.".into()),
        }
    }
    pub fn close(&self, project: Option<&str>) {
        self.close_session(project, None);
    }
    pub fn close_session(&self, project: Option<&str>, id: Option<&str>) {
        let Ok(_transition) = self.transition.lock() else {
            return;
        };
        self.close_inner(project, id);
    }
    fn close_inner(&self, project: Option<&str>, id: Option<&str>) {
        let previous = self.active.lock().ok().and_then(|mut s| {
            if s.as_ref().is_some_and(|a| {
                project.is_none_or(|p| a.session.project == p)
                    && id.is_none_or(|id| a.session.id == id)
            }) {
                s.take()
            } else {
                None
            }
        });
        drop(previous);
    }
    pub fn status(&self, project: &str) -> Result<Status, String> {
        self.active
            .lock()
            .map_err(|_| "Không đọc được audio preview.")?
            .as_ref()
            .filter(|a| a.session.project == project)
            .map(|a| a.session.status())
            .ok_or("Audio preview chưa sẵn sàng.".into())
    }
    pub fn control(
        &self,
        project: &str,
        action: &str,
        time: Option<f64>,
        volume: Option<f32>,
        mix: Option<Mix>,
    ) -> Result<(), String> {
        self.active
            .lock()
            .map_err(|_| "Không truy cập được audio preview.")?
            .as_ref()
            .filter(|a| a.session.project == project)
            .ok_or("Audio preview đã đóng.")?
            .session
            .control(action, time, volume, mix)
    }
    pub fn control_session(
        &self,
        project: &str,
        id: &str,
        action: &str,
        time: Option<f64>,
        volume: Option<f32>,
        mix: Option<Mix>,
    ) -> Result<(), String> {
        self.active
            .lock()
            .map_err(|_| "Không truy cập được audio preview.")?
            .as_ref()
            .filter(|a| a.session.project == project && a.session.id == id)
            .ok_or("Phiên audio đã đóng.")?
            .session
            .control(action, time, volume, mix)
    }
}
#[cfg(not(windows))]
fn run(
    _: Arc<Session>,
    _: Option<PathBuf>,
    _: Option<PathBuf>,
    _: Vec<Voice>,
    tx: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let e = "Audio preview hiện hỗ trợ Windows.".to_string();
    let _ = tx.send(Err(e.clone()));
    Err(e)
}
#[cfg(windows)]
fn stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    state: Arc<Session>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    fn consume<T: cpal::Sample + cpal::FromSample<f32>>(
        data: &mut [T],
        info: &cpal::OutputCallbackInfo,
        state: &Session,
    ) {
        let playing =
            state.audible.load(Ordering::Acquire) && !state.stopped.load(Ordering::Acquire);
        let mut frames = 0;
        let gain = f32::from_bits(state.volume.load(Ordering::Acquire));
        if playing {
            if let Ok(mut ring) = state.ring.try_lock() {
                for frame in data.chunks_exact_mut(2) {
                    if ring.len() >= 2 {
                        frame[0] = T::from_sample(ring.pop_front().unwrap() * gain);
                        frame[1] = T::from_sample(ring.pop_front().unwrap() * gain);
                        frames += 1;
                    } else {
                        frame.fill(T::from_sample(0.0));
                    }
                }
                state.frames.fetch_add(frames, Ordering::Release);
                let stamp = info.timestamp();
                let latency = stamp
                    .playback
                    .duration_since(&stamp.callback)
                    .unwrap_or_default();
                state.latency.store(
                    (latency.as_secs_f64() * RATE as f64).round() as u64 + data.len() as u64 / 2,
                    Ordering::Release,
                );
            } else {
                data.fill(T::from_sample(0.0));
            }
        } else {
            data.fill(T::from_sample(0.0));
        }
        if playing {
            state
                .buffering
                .store(frames < data.len() as u64 / 2, Ordering::Release);
        }
    }
    let errors = state.clone();
    let on_error = move |e| {
        if let Ok(mut error) = errors.error.lock() {
            *error = Some(format!("Thiết bị âm thanh gặp lỗi: {e}"));
        }
    };
    let result = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config,
            move |v: &mut [f32], i| consume(v, i, &state),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            config,
            move |v: &mut [i16], i| consume(v, i, &state),
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            config,
            move |v: &mut [u16], i| consume(v, i, &state),
            on_error,
            None,
        ),
        _ => return Err("Thiết bị không hỗ trợ định dạng âm thanh này.".into()),
    };
    result.map_err(|e| format!("Không mở được thiết bị âm thanh: {e}"))
}
#[cfg(windows)]
fn run(
    state: Arc<Session>,
    source: Option<PathBuf>,
    stem: Option<PathBuf>,
    voices: Vec<Voice>,
    tx: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let setup = (|| {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("Không tìm thấy thiết bị âm thanh.")?;
        let configs = device
            .supported_output_configs()
            .map_err(|e| e.to_string())?;
        let supported = configs
            .filter(|c| {
                c.channels() == 2
                    && c.min_sample_rate().0 <= RATE
                    && c.max_sample_rate().0 >= RATE
                    && matches!(
                        c.sample_format(),
                        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
                    )
            })
            .min_by_key(|c| {
                if c.sample_format() == cpal::SampleFormat::F32 {
                    0
                } else {
                    1
                }
            })
            .ok_or(
                "Thiết bị âm thanh cần hỗ trợ stereo 48 kHz. Hãy chọn thiết bị khác trong Windows.",
            )?
            .with_sample_rate(cpal::SampleRate(RATE));
        let format = supported.sample_format();
        let config = supported.config();
        let output = stream(&device, &config, format, state.clone())?;
        Ok::<_, String>((device, config, format, output))
    })();
    let (device, config, format, mut output) = match setup {
        Ok(v) => {
            let _ = tx.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = tx.send(Err(e.clone()));
            return Err(e);
        }
    };
    let mut revision = u64::MAX;
    let mut mixer: Option<Mixer> = None;
    let mut stream_playing = false;
    let mut exhausted = false;
    let mut last_data = Instant::now();
    let mut drained_at: Option<Instant> = None;
    while !state.stopped.load(Ordering::Acquire) {
        let (play, seek, rev, mix) = {
            let d = state
                .desired
                .lock()
                .map_err(|_| "Không đọc được điều khiển audio.")?;
            (d.playing, d.seek, d.revision, d.mix.clone())
        };
        if rev != revision {
            state.audible.store(false, Ordering::Release);
            let _ = output.pause();
            drop(output);
            state
                .ring
                .lock()
                .map_err(|_| "Không xóa được audio cũ.")?
                .clear();
            output = stream(&device, &config, format, state.clone())?;
            stream_playing = false;
            mixer = None;
            revision = rev;
            exhausted = false;
            last_data = Instant::now();
            drained_at = None;
        }
        if !play {
            drained_at = None;
            state.audible.store(false, Ordering::Release);
            if stream_playing {
                output.pause().map_err(|e| e.to_string())?;
                stream_playing = false;
            }
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        if mixer.is_none() {
            mixer = Some(Mixer::new(
                source.as_deref(),
                stem.as_deref(),
                voices.clone(),
                mix.clone(),
                seek,
                state.duration,
            )?);
        }
        if let Some(m) = mixer.as_mut() {
            m.mix = mix;
        }
        let space = state
            .ring
            .lock()
            .map_err(|_| "Không đọc được audio queue.")?
            .len()
            < BLOCK * 6;
        if space && !exhausted {
            let cancel = || {
                state.stopped.load(Ordering::Acquire)
                    || state
                        .desired
                        .lock()
                        .map(|d| d.revision != revision)
                        .unwrap_or(true)
            };
            match mixer.as_mut().unwrap().next_block(&cancel) {
                Ok(block) => {
                    exhausted = block.is_empty();
                    if !exhausted {
                        let desired = state
                            .desired
                            .lock()
                            .map_err(|_| "Không đọc được phiên audio.")?;
                        if desired.revision != revision {
                            continue;
                        }
                        last_data = Instant::now();
                        state
                            .ring
                            .lock()
                            .map_err(|_| "Không ghi được audio queue.")?
                            .extend(block);
                    }
                }
                Err(_) if cancel() => continue,
                Err(e) => return Err(e),
            }
        }
        let current = state
            .desired
            .lock()
            .map_err(|_| "Không đọc được phiên audio.")?;
        if current.revision != revision || !current.playing {
            continue;
        }
        if !stream_playing
            && state
                .ring
                .lock()
                .map_err(|_| "Không đọc được audio queue.")?
                .len()
                > 0
        {
            state.audible.store(true, Ordering::Release);
            output.play().map_err(|e| e.to_string())?;
            stream_playing = true;
        }
        if stream_playing {
            state.audible.store(true, Ordering::Release);
        }
        drop(current);
        // Submitted samples are still in WASAPI after the software queue drains.
        // Let that final device buffer play before pausing, including the last phoneme.
        let drained = exhausted && state.ring.lock().map(|r| r.is_empty()).unwrap_or(false);
        if !drained {
            drained_at = None;
        }
        let final_buffer_played = drained
            && drained_at
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_secs_f64()
                >= (state.latency.load(Ordering::Acquire) as f64 / RATE as f64 + 0.005);
        if final_buffer_played && state.finish(revision) {
            let _ = output.pause();
            stream_playing = false;
        }
        if !exhausted && last_data.elapsed() > Duration::from_secs(15) {
            return Err("Giải mã âm thanh mất quá lâu. Hãy thử phát lại.".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
    let _ = output.pause();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Arc<Session> {
        Arc::new(Session {
            id: "current".into(),
            project: "project".into(),
            duration: 60.0,
            desired: Mutex::new(Desired {
                playing: false,
                seek: 0.0,
                revision: 0,
                mix: Mix::project(&Default::default()),
            }),
            ring: Mutex::new(VecDeque::from([0.7, 0.7, 0.4, 0.4])),
            stopped: AtomicBool::new(false),
            audible: AtomicBool::new(true),
            frames: AtomicU64::new(12000),
            base: AtomicU64::new(0_f64.to_bits()),
            latency: AtomicU64::new(200),
            volume: AtomicU32::new(1_f32.to_bits()),
            error: Mutex::new(None),
            buffering: AtomicBool::new(false),
        })
    }
    #[test]
    fn seek_flushes_pcm_and_pause_preserves_clock() {
        let s = session();
        s.control("seek", Some(30.0), None, None).unwrap();
        assert!(s.ring.lock().unwrap().is_empty());
        assert_eq!(s.time(), 30.0);
        assert!(!s.audible.load(Ordering::Acquire));
        assert_eq!(s.desired.lock().unwrap().revision, 1);
        s.control("pause", None, None, None).unwrap();
        assert_eq!(s.time(), 30.0);
        assert!(s.control("seek", Some(f64::NAN), None, None).is_err());
        assert!(s.control("seek", Some(-1.0), None, None).is_err());
    }
    #[test]
    fn obsolete_end_of_file_cannot_override_a_new_seek() {
        let s = session();
        s.control("seek", Some(0.4), None, None).unwrap();
        s.control("play", None, None, None).unwrap();
        assert!(!s.finish(0));
        assert_eq!(s.time(), 0.4);
        assert!(s.status().playing);
        assert!(s.finish(1));
        assert_eq!(s.time(), 60.0);
        assert!(!s.status().playing);
    }
    #[test]
    fn stale_component_cannot_control_or_close_new_session() {
        let s = session();
        let manager = AudioManager::default();
        *manager.active.lock().unwrap() = Some(Owned {
            session: s.clone(),
            worker: None,
        });
        manager.close_session(Some("project"), Some("old"));
        assert_eq!(manager.status("project").unwrap().session_id, "current");
        assert!(manager
            .control_session("project", "old", "play", None, None, None)
            .is_err());
        manager
            .control_session("project", "current", "volume", None, Some(0.5), None)
            .unwrap();
        assert_eq!(f32::from_bits(s.volume.load(Ordering::Acquire)), 0.5);
        manager.close_session(Some("project"), Some("current"));
        assert!(manager.status("project").is_err());
    }
}
