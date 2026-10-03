use super::{clock::Clock, scene, Bounds};
use crate::{
    overlays_service::VisualSettings,
    project_service::{Project, ProjectStore},
    render::renderer::Renderer,
    render_spec::PixelRect,
    transcript_service::Transcript,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::Emitter;
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub session_id: String,
    pub state: String,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    pub frame_pts: f64,
    pub renderer: String,
    pub decode: String,
    pub render_ms: f64,
    pub decode_ms: f64,
    pub present_ms: f64,
    pub dropped_frames: u64,
    pub frames: u64,
    pub seek_ms: f64,
    pub render_scale: f32,
    pub error: Option<String>,
}
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationPatch {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub translated: String,
    #[serde(default)]
    pub deleted: bool,
}
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    pub kind: String,
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Default)]
pub struct Updates {
    pub bounds: Option<Bounds>,
    pub seek: Option<(u64, f64)>,
    pub playing: Option<bool>,
    pub audio: Option<f64>,
    pub scene: Option<(u64, Option<VisualSettings>, Option<Vec<TranslationPatch>>)>,
    pub guide: Option<Option<PixelRect>>,
    pub transforms: std::collections::HashMap<String, Transform>,
}
impl Updates {
    pub fn patch_scene(
        &mut self,
        version: u64,
        visual: Option<VisualSettings>,
        translations: Option<Vec<TranslationPatch>>,
    ) {
        if visual.is_none() && translations.is_none() {
            return;
        }
        if let Some((prev, v, t)) = self.scene.take() {
            if version > prev {
                self.scene = Some((
                    version,
                    visual.or(v),
                    match (t, translations) {
                        (Some(mut old), Some(new)) => {
                            for p in new {
                                if let Some(existing) = old.iter_mut().find(|s| s.id == p.id) {
                                    *existing = p;
                                } else {
                                    old.push(p);
                                }
                            }
                            Some(old)
                        }
                        (old, new) => new.or(old),
                    },
                ));
            } else {
                self.scene = Some((prev, v, t));
            }
        } else {
            self.scene = Some((version, visual, translations));
        }
    }
}
pub struct Session {
    pub id: String,
    pub owner: String,
    pub project_id: String,
    pub size: (u32, u32),
    pub range: (f64, f64),
    pub updates: Mutex<Updates>,
    pub wake: mpsc::SyncSender<()>,
    pub stopped: AtomicBool,
    pub generation: AtomicU64,
    pub scene_version: AtomicU64,
    pub status: Mutex<Status>,
}
struct Active {
    session: Arc<Session>,
    thread: Option<thread::JoinHandle<()>>,
}
#[derive(Default)]
pub struct PreviewManager {
    active: Mutex<Option<Active>>,
    operation: Mutex<()>,
}
impl PreviewManager {
    pub fn close_project(&self, project_id: &str) {
        let Ok(_operation) = self.operation.lock() else {
            return;
        };
        let id = self.active.lock().ok().and_then(|slot| {
            slot.as_ref()
                .filter(|a| a.session.project_id == project_id)
                .map(|a| a.session.id.clone())
        });
        if let Some(id) = id {
            self.close_inner(Some(&id), None);
        }
    }
    pub fn get(&self, id: &str, owner: &str) -> Result<Arc<Session>, String> {
        self.active
            .lock()
            .map_err(|_| "Preview bận.")?
            .as_ref()
            .filter(|a| a.session.id == id && a.session.owner == owner)
            .map(|a| a.session.clone())
            .ok_or("Preview đã đóng hoặc không thuộc cửa sổ hiện tại.".into())
    }
    pub fn close(&self, id: Option<&str>, owner: Option<&str>) {
        let Ok(_operation) = self.operation.lock() else {
            return;
        };
        self.close_inner(id, owner);
    }
    fn close_inner(&self, id: Option<&str>, owner: Option<&str>) {
        let old = {
            let Ok(mut slot) = self.active.lock() else {
                return;
            };
            if slot.as_ref().is_some_and(|a| {
                id.is_none_or(|id| id == a.session.id) && owner.is_none_or(|o| o == a.session.owner)
            }) {
                slot.take()
            } else {
                None
            }
        };
        if let Some(mut a) = old {
            a.session.stopped.store(true, Ordering::SeqCst);
            a.session.generation.fetch_add(1, Ordering::SeqCst);
            let _ = a.session.wake.try_send(());
            if let Some(t) = a.thread.take() {
                let _ = t.join();
            }
        }
    }
    pub fn open(
        &self,
        parent: isize,
        owner: String,
        app: tauri::AppHandle,
        store: ProjectStore,
        project: Project,
        transcript: Transcript,
        source: PathBuf,
        offset: f64,
        end: f64,
        bounds: Bounds,
    ) -> Result<String, String> {
        let _operation = self.operation.lock().map_err(|_| "Preview bận.")?;
        self.close_inner(None, None);
        let id = uuid::Uuid::new_v4().to_string();
        let duration = project.source.duration.ok_or("Video thiếu thời lượng.")?;
        if !offset.is_finite()
            || !end.is_finite()
            || offset < 0.0
            || end <= offset
            || end > duration + 0.1
        {
            return Err("Khoảng preview không hợp lệ.".into());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let session = Arc::new(Session {
            id: id.clone(),
            owner,
            project_id: project.id.clone(),
            range: (offset, end.min(duration)),
            size: (
                project.source.width.unwrap_or(1),
                project.source.height.unwrap_or(1),
            ),
            updates: Mutex::new(Updates::default()),
            wake: tx,
            stopped: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            scene_version: AtomicU64::new(0),
            status: Mutex::new(Status {
                session_id: id.clone(),
                state: "loading".into(),
                position: offset,
                duration,
                playing: false,
                frame_pts: offset,
                renderer: String::new(),
                decode: String::new(),
                render_ms: 0.0,
                decode_ms: 0.0,
                present_ms: 0.0,
                dropped_frames: 0,
                frames: 0,
                seek_ms: 0.0,
                render_scale: 1.0,
                error: None,
            }),
        });
        let s = session.clone();
        let t = thread::Builder::new()
            .name("native-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(
                        parent, &app, &store, project, transcript, source, offset, bounds, &s, rx,
                    )
                }));
                let error = match result {
                    Ok(Err(e)) => Some(e),
                    Err(_) => Some(
                        "Bộ xem video GPU gặp lỗi. Đang chuyển sang chế độ tương thích.".into(),
                    ),
                    _ => None,
                };
                if let Some(error) = error {
                    log::warn!("Native preview: {error}");
                    if let Ok(mut st) = s.status.lock() {
                        st.state = "error".into();
                        st.error = Some(error);
                        let _ = app.emit_to(&s.owner, "preview-status", st.clone());
                    }
                } else if s.stopped.load(Ordering::SeqCst) {
                    if let Ok(mut st) = s.status.lock() {
                        st.state = "closed".into();
                        st.playing = false;
                        let _ = app.emit_to(&s.owner, "preview-status", st.clone());
                    }
                }
            })
            .map_err(|_| "Không mở được luồng preview.")?;
        *self.active.lock().map_err(|_| "Preview bận.")? = Some(Active {
            session,
            thread: Some(t),
        });
        Ok(id)
    }
}
impl Drop for PreviewManager {
    fn drop(&mut self) {
        self.close(None, None);
    }
}
#[cfg(windows)]
fn run(
    parent: isize,
    app: &tauri::AppHandle,
    store: &ProjectStore,
    mut project: Project,
    mut transcript: Transcript,
    source: PathBuf,
    offset: f64,
    bounds: Bounds,
    s: &Session,
    rx: mpsc::Receiver<()>,
) -> Result<(), String> {
    // Reuse the live display device across project switches; export selection stays independent.
    static GPU: std::sync::OnceLock<Mutex<Option<crate::render::device::GpuDevice>>> =
        std::sync::OnceLock::new();
    let gpu = {
        let mut cache = GPU
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "GPU bận.")?;
        if cache
            .as_ref()
            .is_some_and(|g| g.error.lock().is_ok_and(|e| e.is_some()))
        {
            *cache = None;
        }
        if cache.is_none() {
            *cache = Some(crate::render::device::create_with_power(
                None,
                wgpu::PowerPreference::LowPower,
            )?);
        }
        cache.as_ref().unwrap().clone()
    };
    let quality = super::quality::Quality::environment();
    let mut scale = quality.initial(
        s.size,
        (bounds.width * bounds.dpr, bounds.height * bounds.dpr),
    );
    let scene = scene::build(&project, &transcript)?.scaled(scale);
    let paths = scene::logos(store, &project.id, &scene)?;
    let mut renderer = Renderer::with_device(scene, &paths, gpu.clone())?;
    let pointer_app = app.clone();
    let pointer_id = s.id.clone();
    let pointer_owner = s.owner.clone();
    let pointer = Box::new(move |kind: &str, x: f64, y: f64, buttons: u32| {
        let _ = pointer_app.emit_to(
            &pointer_owner,
            "preview-pointer",
            serde_json::json!({"sessionId":pointer_id,"kind":kind,"x":x,"y":y,"buttons":buttons}),
        );
    });
    let drop_app = app.clone();
    let drop_id = s.id.clone();
    let drop_owner = s.owner.clone();
    let drop_files = Box::new(move |paths: Vec<String>| {
        let _ = drop_app.emit_to(
            &drop_owner,
            "preview-drop",
            serde_json::json!({"sessionId":drop_id,"paths":paths}),
        );
    });
    let mut surface =
        super::surface::NativeSurface::new(parent, pointer, drop_files, &gpu, bounds)?;
    enum Source {
        Hardware(crate::media_gpu::windows::preview::PreviewSource),
        Software(super::software::SoftwareSource),
    }
    let source_path = source.clone();
    let mut source =
        match crate::media_gpu::windows::preview::PreviewSource::new(&renderer, &source) {
            Ok(v) => Source::Hardware(v),
            Err(e) => {
                log::warn!("Preview hardware decoder fallback: {e}");
                Source::Software(super::software::SoftwareSource::new(
                    &source,
                    (renderer.scene.width, renderer.scene.height),
                )?)
            }
        };
    {
        let mut st = s.status.lock().unwrap();
        st.renderer = renderer.adapter_name().into();
        st.decode = match source {
            Source::Hardware(_) => "hardware",
            _ => "software",
        }
        .into();
        st.state = "paused".into();
    }
    let duration = project.source.duration.unwrap();
    let mut clock = Clock::new(s.range.1);
    clock.seek(offset)?;
    let (mut visible, mut dirty, mut version, mut guide) = (bounds.visible, true, 0, None);
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut last_frame_pts = -1.0;
    let mut seek_started = None;
    let mut samples = 0;
    let mut render_average = 0.0;
    while !s.stopped.load(Ordering::SeqCst) {
        surface.pump();
        let updates = { std::mem::take(&mut *s.updates.lock().map_err(|_| "Preview bận.")?) };
        if let Some(b) = updates.bounds {
            surface.resize(&gpu, b)?;
            visible = b.visible;
            dirty = true;
        }
        if let Some(g) = updates.guide {
            guide = g;
            dirty = true;
        }
        let mut scene_dirty = false;
        if let Some((v, visual, translations)) = updates.scene {
            if v > version {
                if let Some(visual) = visual {
                    scene::apply(store, &mut project, visual)?;
                }
                if let Some(patches) = translations {
                    scene::apply_translations(&mut transcript, patches, duration)?;
                }
                version = v;
                scene_dirty = true;
            }
        }
        let transform_dirty = !updates.transforms.is_empty();
        for t in updates.transforms.into_values() {
            match t.kind.as_str() {
                "blur" => {
                    if let Some(r) = project.blur_regions.iter_mut().find(|r| r.id == t.id) {
                        r.x = t.x;
                        r.y = t.y;
                        r.width = t.width;
                        r.height = t.height;
                    }
                }
                "logo" => {
                    if let Some(r) = project.logos.iter_mut().find(|r| r.id == t.id) {
                        r.x = t.x;
                        r.y = t.y;
                        r.width = t.width;
                        r.height = t.height;
                    }
                }
                "text" => {
                    if let Some(r) = project.text_overlays.iter_mut().find(|r| r.id == t.id) {
                        r.x = t.x;
                        r.y = t.y;
                        r.width = t.width;
                        r.height = t.height;
                    }
                }
                "subtitle" => {
                    project.subtitle.region = Some(crate::project_service::SubtitleRegion {
                        x: t.x,
                        y: t.y,
                        width: t.width,
                        height: t.height,
                    });
                }
                _ => {}
            }
        }
        if scene_dirty || transform_dirty {
            let scene = scene::build(&project, &transcript)?.scaled(scale);
            let paths = scene::logos(store, &project.id, &scene)?;
            renderer.update_scene(scene, &paths)?;
            dirty = true;
        }
        if let Some(p) = updates.playing {
            clock.set_playing(p);
        }
        if let Some(t) = updates.audio {
            clock.audio_anchor(t);
        }
        if let Some((generation, t)) = updates.seek {
            let t = t.clamp(s.range.0, s.range.1);
            clock.seek(t)?;
            seek_started = Some(Instant::now());
            last_frame_pts = -1.0;
            let seek_result = match &mut source {
                Source::Hardware(v) => v.seek((t - offset).max(0.0)),
                Source::Software(v) => v.seek((t - offset).max(0.0)),
            };
            if let Err(error) = seek_result {
                if matches!(source, Source::Hardware(_)) {
                    log::warn!("Preview hardware seek fallback: {error}");
                    let mut fallback = super::software::SoftwareSource::new(
                        &source_path,
                        (renderer.scene.width, renderer.scene.height),
                    )?;
                    fallback.seek((t - offset).max(0.0))?;
                    source = Source::Software(fallback);
                    s.status.lock().unwrap().decode = "software".into();
                } else {
                    return Err(error);
                }
            }
            if generation != s.generation.load(Ordering::SeqCst) {
                continue;
            }
            dirty = true;
        }
        let mut position = clock.now();
        if position >= s.range.1 && clock.playing {
            clock.set_playing(false);
            position = s.range.1;
        }
        let busy = crate::resource_scheduler::ai_gpu_slot().try_lock().is_err()
            || crate::resource_scheduler::ffmpeg_slot().try_lock().is_err();
        if busy && clock.playing {
            clock.set_playing(false);
        }
        if visible && !busy && (clock.playing || dirty) {
            let generation = s.generation.load(Ordering::SeqCst);
            let stale = || {
                s.stopped.load(Ordering::SeqCst)
                    || generation != s.generation.load(Ordering::SeqCst)
            };
            let decode_started = Instant::now();
            let dropped_before = match &source {
                Source::Hardware(v) => v.dropped,
                Source::Software(v) => v.dropped,
            };
            let result = match &mut source {
                Source::Hardware(v) => v.select((position - offset).max(0.0), stale),
                Source::Software(v) => v.select((position - offset).max(0.0), stale),
            };
            let pts = match result {
                Ok(pts) => pts,
                Err(error) if matches!(source, Source::Hardware(_)) => {
                    log::warn!("Preview hardware decode failed; staging fallback: {error}");
                    let mut fallback = super::software::SoftwareSource::new(
                        &source_path,
                        (renderer.scene.width, renderer.scene.height),
                    )?;
                    fallback.seek((position - offset).max(0.0))?;
                    source = Source::Software(fallback);
                    s.status.lock().unwrap().decode = "software".into();
                    dirty = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            {
                let mut st = s.status.lock().unwrap();
                st.decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
                if clock.playing {
                    let dropped_after = match &source {
                        Source::Hardware(v) => v.dropped,
                        Source::Software(v) => v.dropped,
                    };
                    st.dropped_frames += dropped_after.saturating_sub(dropped_before);
                }
            }
            if stale() {
                continue;
            }
            if let Some(local) = pts {
                let pts = local + offset;
                if dirty || pts != last_frame_pts {
                    let start = Instant::now();
                    let frame = match surface.surface.get_current_texture() {
                        Ok(f) => f,
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            surface.surface.configure(&gpu.device, &surface.config);
                            continue;
                        }
                        Err(wgpu::SurfaceError::Timeout) => continue,
                        Err(e) => return Err(format!("Không hiển thị được video: {e}")),
                    };
                    let view = frame.texture.create_view(&Default::default());
                    let size = (surface.config.width, surface.config.height);
                    let render_guide = guide.map(|r: PixelRect| PixelRect {
                        x: (r.x as f32 * scale).round() as u32,
                        y: (r.y as f32 * scale).round() as u32,
                        width: (r.width as f32 * scale).round().max(1.0) as u32,
                        height: (r.height as f32 * scale).round().max(1.0) as u32,
                    });
                    match &mut source {
                        Source::Hardware(v) => {
                            v.present(&mut renderer, &view, size, pts, render_guide)?
                        }
                        Source::Software(v) => {
                            v.upload(&renderer)?;
                            renderer.present_preview(None, &view, size, pts, render_guide)?;
                        }
                    }
                    let present_started = Instant::now();
                    frame.present();
                    let present_ms = present_started.elapsed().as_secs_f64() * 1000.0;
                    last_frame_pts = pts;
                    dirty = false;
                    let mut st = s.status.lock().unwrap();
                    st.frame_pts = pts;
                    st.render_ms = start.elapsed().as_secs_f64() * 1000.0;
                    st.present_ms = present_ms;
                    st.render_scale = scale;
                    st.frames += 1;
                    if let Some(start) = seek_started.take() {
                        st.seek_ms = start.elapsed().as_secs_f64() * 1000.0;
                        log::debug!("Preview seek {:.2}ms, pts={pts}", st.seek_ms);
                    }
                    if clock.playing {
                        samples += 1;
                        render_average += (st.render_ms - render_average) / samples as f64;
                    }
                }
            }
        }
        if samples >= 60 {
            if let Some(next) = quality.slower(
                scale,
                render_average,
                1000.0 / project.source.fps.unwrap_or(30.0).clamp(1.0, 120.0),
            ) {
                let new_scene = scene::build(&project, &transcript)?.scaled(next);
                let paths = scene::logos(store, &project.id, &new_scene)?;
                let new_renderer = Renderer::with_device(new_scene, &paths, gpu.clone())?;
                let mut new_source = match crate::media_gpu::windows::preview::PreviewSource::new(
                    &new_renderer,
                    &source_path,
                ) {
                    Ok(mut v) => {
                        v.seek((position - offset).max(0.0))?;
                        Source::Hardware(v)
                    }
                    Err(_) => {
                        let mut v = super::software::SoftwareSource::new(
                            &source_path,
                            (new_renderer.scene.width, new_renderer.scene.height),
                        )?;
                        v.seek((position - offset).max(0.0))?;
                        Source::Software(v)
                    }
                };
                std::mem::swap(&mut source, &mut new_source);
                drop(new_source);
                renderer = new_renderer;
                scale = next;
                last_frame_pts = -1.0;
                dirty = true;
                s.status.lock().unwrap().decode = match source {
                    Source::Hardware(_) => "hardware",
                    _ => "software",
                }
                .into();
                log::info!("Preview Auto render scale={scale}, previous render/present average={render_average:.2}ms");
            }
            samples = 0;
            render_average = 0.0;
        }
        if last_emit.elapsed() >= Duration::from_millis(100) {
            let mut st = s.status.lock().unwrap();
            st.position = position;
            st.playing = clock.playing;
            st.state = if busy {
                "suspended"
            } else if clock.playing {
                "playing"
            } else {
                "paused"
            }
            .into();
            let _ = app.emit_to(&s.owner, "preview-status", st.clone());
            last_emit = Instant::now();
        }
        // Idle redraws only on changes. Playing pacing follows monotonic/audio time and source PTS.
        let _ = rx.recv_timeout(if clock.playing && visible && !busy {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(50)
        });
    }
    drop(source);
    drop(surface);
    drop(renderer);
    Ok(())
}
#[cfg(not(windows))]
fn run(
    _: isize,
    _: &tauri::AppHandle,
    _: &ProjectStore,
    _: Project,
    _: Transcript,
    _: PathBuf,
    _: f64,
    _: Bounds,
    _: &Session,
    _: mpsc::Receiver<()>,
) -> Result<(), String> {
    Err("Preview GPU native hiện hỗ trợ Windows. Dùng chế độ tương thích.".into())
}
