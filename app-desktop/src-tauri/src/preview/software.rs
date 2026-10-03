use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
pub struct Frame {
    pub pts: f64,
    pub rgba: Vec<u8>,
}
pub struct SoftwareSource {
    path: PathBuf,
    size: (u32, u32),
    child: Option<Child>,
    rx: Option<mpsc::Receiver<Frame>>,
    threads: Vec<thread::JoinHandle<()>>,
    next: Option<Frame>,
    current: Option<Frame>,
    pub dropped: u64,
}
impl SoftwareSource {
    pub fn new(path: &Path, size: (u32, u32)) -> Result<Self, String> {
        let mut s = Self {
            path: path.into(),
            size,
            child: None,
            rx: None,
            threads: vec![],
            next: None,
            current: None,
            dropped: 0,
        };
        s.seek(0.0)?;
        Ok(s)
    }
    fn stop(&mut self) {
        self.rx.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        self.next = None;
    }
    pub fn seek(&mut self, time: f64) -> Result<(), String> {
        self.stop();
        self.current = None;
        let mut command = crate::runtime_paths::ffmpeg_command()?;
        let filter = format!(
            "scale={}:{}:flags=bilinear,showinfo",
            self.size.0, self.size.1
        );
        let mut child = command
            .args([
                "-hide_banner",
                "-loglevel",
                "info",
                "-ss",
                &format!("{time:.6}"),
                "-i",
            ])
            .arg(&self.path)
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-vf",
                &filter,
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
            .stdin(Stdio::null())
            .spawn()
            .map_err(|_| "Không mở được video bằng bộ giải mã tương thích.")?;
        let mut out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        let (pts_tx, pts_rx) = mpsc::sync_channel(2);
        let (tx, rx) = mpsc::sync_channel(2);
        let length = crate::render::frame::byte_len(self.size.0, self.size.1)?;
        self.threads.push(thread::spawn(move || {
            let mut base = None;
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if let Some(config) = line.split("config in time_base:").nth(1) {
                    base = config.split_whitespace().next().and_then(|v| {
                        let (n, d) = v.trim_end_matches(',').split_once('/')?;
                        let denominator = d.parse::<f64>().ok()?;
                        (denominator > 0.0).then_some(n.parse::<f64>().ok()? / denominator)
                    });
                }
                if let Some(pts) = crate::render::stream::pts_from_line(&line, base) {
                    if pts_tx.send(time + pts).is_err() {
                        break;
                    }
                }
            }
        }));
        self.threads.push(thread::spawn(move || loop {
            let mut rgba = vec![0; length];
            if out.read_exact(&mut rgba).is_err() {
                break;
            }
            let Ok(pts) = pts_rx.recv_timeout(Duration::from_secs(15)) else {
                break;
            };
            if tx.send(Frame { pts, rgba }).is_err() {
                break;
            }
        }));
        self.child = Some(child);
        self.rx = Some(rx);
        Ok(())
    }
    pub fn select(&mut self, time: f64, stale: impl Fn() -> bool) -> Result<Option<f64>, String> {
        let mut consumed = false;
        loop {
            if stale() {
                return Ok(None);
            }
            if self.next.is_none() {
                self.next = match self
                    .rx
                    .as_ref()
                    .unwrap()
                    .recv_timeout(Duration::from_millis(100))
                {
                    Ok(frame) => Some(frame),
                    Err(mpsc::RecvTimeoutError::Disconnected) if self.current.is_none() => {
                        return Err("Không giải mã được video để preview.".into())
                    }
                    _ => None,
                };
            }
            if self.next.as_ref().is_some_and(|f| f.pts <= time + 0.0001)
                || (self.current.is_none() && self.next.is_some())
            {
                if consumed {
                    self.dropped += 1;
                }
                consumed = true;
                self.current = self.next.take();
            } else {
                break;
            }
        }
        Ok(self.current.as_ref().map(|f| f.pts))
    }
    pub fn upload(&self, renderer: &crate::render::renderer::Renderer) -> Result<(), String> {
        if let Some(frame) = &self.current {
            renderer.upload_preview_frame(&frame.rgba)?;
        }
        Ok(())
    }
}
impl Drop for SoftwareSource {
    fn drop(&mut self) {
        self.stop();
    }
}
