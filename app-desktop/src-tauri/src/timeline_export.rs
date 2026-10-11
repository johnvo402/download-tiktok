//! Immutable timeline render plan. Times and audio rules mirror timeline/{time,audio-layout}.ts.
mod audio;
mod graph;
mod runner;
#[cfg(test)]
mod tests;
use crate::{
    hardware_acceleration::{self, VideoEncoder},
    project_service::ProjectStore,
    render_service::ExportValidation,
    render_spec::ExportOptions,
    timeline::{
        seconds_to_time_us, Clip, ClipAudio, ClipContent, Rational, TimelineDocument, TrackKind,
    },
    timeline_audio::{self, SourceMedia},
};
pub(crate) use runner::render;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineExportOptions {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    /// Both absent means the whole project. A range uses absolute project timestamps.
    pub start_us: Option<u64>,
    pub end_us: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct AudioRegion {
    pub clip: Clip,
    pub path: PathBuf,
    pub voice: bool,
    pub start: u64,
    pub end: u64,
    pub source_start: u64,
    pub speed: f64,
    pub speech: (u64, u64),
    pub audio: ClipAudio,
}
pub struct Plan {
    pub document: TimelineDocument,
    pub options: TimelineExportOptions,
    pub encoder: VideoEncoder,
    pub start: u64,
    pub end: u64,
    pub assets: HashMap<String, PathBuf>,
    pub sources: HashMap<String, SourceMedia>,
    pub audio: Vec<AudioRegion>,
    pub warnings: Vec<String>,
}
/// Owns the captured media links/WAV copies until success, failure or queued cancellation.
pub struct Snapshot {
    pub plan: Plan,
    pub directory: tempfile::TempDir,
    pub output_directory: PathBuf,
}
impl Plan {
    pub fn duration(&self) -> f64 {
        seconds(self.end - self.start)
    }
    pub fn window(&self, clip: &Clip) -> Result<Option<(u64, u64)>, String> {
        let from = clip.start_us.max(self.start);
        let to = clip.end_us()?.min(self.end);
        Ok((from < to).then_some((from, to)))
    }
    pub fn visual_clips(&self) -> Vec<&Clip> {
        let mut clips: Vec<_> = self
            .document
            .clips
            .iter()
            .filter(|c| {
                matches!(
                    c.content,
                    ClipContent::Video { .. }
                        | ClipContent::Image { .. }
                        | ClipContent::Text { .. }
                        | ClipContent::Subtitles { .. }
                ) && self
                    .document
                    .tracks
                    .iter()
                    .any(|t| t.id == c.track_id && t.enabled)
                    && self.window(c).is_ok_and(|w| w.is_some())
            })
            .collect();
        clips.sort_by_key(|c| {
            self.document
                .tracks
                .iter()
                .find(|t| t.id == c.track_id)
                .unwrap()
                .order
        });
        clips
    }
    pub fn validation(&self) -> ExportValidation {
        let captions = self
            .visual_clips()
            .iter()
            .filter(|c| matches!(c.content, ClipContent::Subtitles { .. }))
            .map(|c| {
                caption_states(self, c)
                    .map(|s| s.iter().filter(|s| !s.2.is_empty()).count())
                    .unwrap_or(0)
            })
            .sum();
        ExportValidation {
            resolved_encoder: self.encoder,
            resolved_encoder_label: self.encoder.label().into(),
            valid: true,
            width: self.options.width,
            height: self.options.height,
            duration: self.duration(),
            subtitle_region_id: None,
            translated_segments: captions,
            fits_normally: captions,
            fit_with_smaller_font: 0,
            smallest_font_size: 0,
            warnings: vec![],
            dubbing_segments: self
                .audio
                .iter()
                .filter(|r| r.voice && r.start < self.end && r.end > self.start)
                .count(),
            expected_audio: self
                .audio
                .iter()
                .any(|r| r.start < self.end && r.end > self.start),
            audio_warnings: self.warnings.clone(),
        }
    }
}
pub fn seconds(us: u64) -> f64 {
    us as f64 / 1_000_000.0
}
fn id(c: &Clip) -> Option<&str> {
    match &c.content {
        ClipContent::Video {
            video_content_id, ..
        }
        | ClipContent::SourceAudio {
            video_content_id, ..
        }
        | ClipContent::Voice { video_content_id }
        | ClipContent::Subtitles { video_content_id } => Some(video_content_id),
        _ => None,
    }
}
pub fn asset_id(c: &Clip) -> Option<&str> {
    match &c.content {
        ClipContent::Video { asset_id, .. }
        | ClipContent::Image { asset_id }
        | ClipContent::Audio { asset_id }
        | ClipContent::SourceAudio { asset_id, .. } => Some(asset_id),
        _ => None,
    }
}
fn audible(doc: &TimelineDocument, c: &Clip, a: &ClipAudio) -> bool {
    let solo = doc
        .tracks
        .iter()
        .any(|t| t.kind != TrackKind::Subtitle && t.enabled && t.solo);
    !a.muted
        && a.volume > 0.0
        && doc.tracks.iter().any(|t| {
            t.id == c.track_id
                && t.kind != TrackKind::Subtitle
                && t.enabled
                && !t.muted
                && (!solo || t.solo)
        })
}
fn audio_settings(doc: &TimelineDocument, c: &Clip) -> ClipAudio {
    if let Some(a) = &c.audio {
        return a.clone();
    }
    let d = doc
        .video_contents
        .iter()
        .find(|v| Some(v.id.as_str()) == id(c))
        .map(|v| &v.settings.dubbing);
    let original = matches!(
        c.content,
        ClipContent::Video { .. } | ClipContent::SourceAudio { .. }
    );
    ClipAudio {
        volume: if original {
            d.map_or(1.0, |d| d.original_volume)
        } else if matches!(c.content, ClipContent::Voice { .. }) {
            d.map_or(1.0, |d| d.dubbed_volume)
        } else {
            1.0
        },
        muted: original && d.is_some_and(|d| d.mute_original),
        fade_in_us: 0,
        fade_out_us: 0,
        voice_enabled: true,
        voice_speed: 1.0,
        ducking_enabled: original && d.is_some_and(|d| d.ducking_enabled),
        ducking_amount: d.map_or(0.75, |d| d.ducking_amount),
        duck_attack_us: d.map_or(60_000, |d| d.audio_processing.duck_attack_ms as u64 * 1000),
        duck_release_us: d.map_or(180_000, |d| {
            d.audio_processing.duck_release_ms as u64 * 1000
        }),
    }
}
pub fn build(
    store: &ProjectStore,
    project_id: &str,
    options: &ExportOptions,
) -> Result<Plan, String> {
    let project = store.get(project_id)?;
    let doc = project.timeline.clone().ok_or("Dự án chưa có timeline.")?;
    crate::timeline::validate(&doc)?;
    let config = options
        .timeline
        .clone()
        .ok_or("Thiếu thiết lập export timeline.")?;
    let fps = config.frame_rate.numerator as f64 / config.frame_rate.denominator as f64;
    if !(1.0..=120.0).contains(&fps)
        || config.frame_rate.denominator == 0
        || ![config.width, config.height]
            .into_iter()
            .all(|v| (2..=8192).contains(&v) && v % 2 == 0)
    {
        return Err("Độ phân giải phải chẵn, từ 2–8192 pixel; FPS từ 1–120.".into());
    }
    let duration = doc
        .clips
        .iter()
        .map(Clip::end_us)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let (start, end) = match (config.start_us, config.end_us) {
        (None, None) => (0, duration),
        (Some(a), Some(b)) => (a, b),
        _ => return Err("Vùng xuất cần cả điểm bắt đầu và kết thúc.".into()),
    };
    if start >= end || end > duration {
        return Err("Vùng xuất phải nằm trong thời lượng timeline và có độ dài dương.".into());
    }
    if seconds(end - start) * fps < 1.0 {
        return Err("Vùng xuất phải dài ít nhất một frame ở FPS đã chọn.".into());
    }
    let mut plan = Plan {
        document: doc,
        options: config,
        encoder: hardware_acceleration::resolve(options.video_codec, options.encoder)?,
        start,
        end,
        assets: HashMap::new(),
        sources: HashMap::new(),
        audio: vec![],
        warnings: vec![],
    };
    let used: Vec<_> = plan
        .document
        .clips
        .iter()
        .filter(|c| {
            plan.window(c).is_ok_and(|w| w.is_some())
                && plan
                    .document
                    .tracks
                    .iter()
                    .any(|t| t.id == c.track_id && t.enabled)
        })
        .cloned()
        .collect();
    for c in &used {
        if let Some(content_id) = id(c) {
            if !plan.sources.contains_key(content_id) {
                let source = timeline_audio::resolve(store, project_id, content_id)?;
                plan.sources.insert(content_id.into(), source);
            }
        }
    }
    // Build all audible regions first; ducking also hears speech just outside the export range.
    for c in &plan.document.clips {
        if !matches!(
            c.content,
            ClipContent::Video { .. }
                | ClipContent::SourceAudio { .. }
                | ClipContent::Audio { .. }
                | ClipContent::Voice { .. }
        ) {
            continue;
        }
        let a = audio_settings(&plan.document, c);
        let d = plan
            .document
            .video_contents
            .iter()
            .find(|v| Some(v.id.as_str()) == id(c))
            .map(|v| &v.settings.dubbing);
        if let Some(asset) =
            asset_id(c).and_then(|id| plan.document.assets.iter().find(|v| v.id == id))
        {
            if audible(&plan.document, c, &a)
                && (!matches!(c.content, ClipContent::Video { .. })
                    || !asset.metadata.audio_streams.is_empty())
            {
                plan.audio.push(AudioRegion {
                    clip: c.clone(),
                    path: PathBuf::from(&asset.relative_path),
                    voice: false,
                    start: c.start_us,
                    end: c.end_us()?,
                    source_start: 0,
                    speed: c.rate.numerator as f64 / c.rate.denominator as f64,
                    speech: (c.start_us, c.start_us),
                    audio: a.clone(),
                });
            }
        }
        let implicit = matches!(c.content, ClipContent::Video { .. })
            && d.is_some_and(|d| d.enabled)
            && a.voice_enabled
            && !plan.document.clips.iter().any(|v| {
                matches!(v.content, ClipContent::Voice { .. })
                    && v.linked_clip_id.as_deref() == Some(&c.id)
            });
        let va = if implicit {
            ClipAudio {
                muted: false,
                volume: d.map_or(1.0, |d| d.dubbed_volume),
                ..a.clone()
            }
        } else {
            a.clone()
        };
        if !(implicit || matches!(c.content, ClipContent::Voice { .. }))
            || !audible(&plan.document, c, &va)
        {
            continue;
        }
        // Sources needed by duck attack/release may be adjacent to the chosen range.
        let content_id = id(c).unwrap();
        if !plan.sources.contains_key(content_id) {
            plan.sources.insert(
                content_id.into(),
                timeline_audio::resolve(store, project_id, content_id)?,
            );
        }
        let source = &plan.sources[content_id];
        for w in &source.warnings {
            if plan.window(c)?.is_some() {
                plan.warnings
                    .push(format!("{} · {}: {}", content_id, w.segment_id, w.message));
            }
        }
        for v in &source.voices {
            let adjusted =
                |value| v.start_us + ((value - v.start_us) as f64 / a.voice_speed).round() as u64;
            let Some((from, to)) = c.map_source_interval(v.start_us, adjusted(v.end_us))? else {
                continue;
            };
            let speech = c
                .map_source_interval(adjusted(v.speech_start_us), adjusted(v.speech_end_us))?
                .unwrap_or((from, from));
            plan.audio.push(AudioRegion {
                clip: c.clone(),
                path: v.path.clone().into(),
                voice: true,
                start: from,
                end: to,
                source_start: v.start_us,
                speed: a.voice_speed * c.rate.numerator as f64 / c.rate.denominator as f64,
                speech,
                audio: va.clone(),
            });
        }
    }
    let mut required: HashSet<String> = plan
        .visual_clips()
        .iter()
        .filter_map(|c| asset_id(c).map(str::to_owned))
        .collect();
    for r in &plan.audio {
        if !r.voice && r.start < end && r.end > start {
            required.insert(asset_id(&r.clip).unwrap().into());
        }
    }
    for asset in plan
        .document
        .assets
        .iter()
        .filter(|a| required.contains(&a.id))
    {
        let path = crate::media_library::owned_file(store, &project, asset, true).map_err(|e| {
            format!(
                "Nguồn {}: {e}",
                asset.metadata.original_name.as_deref().unwrap_or(&asset.id)
            )
        })?;
        plan.assets.insert(asset.id.clone(), path);
    }
    for r in &mut plan.audio {
        if !r.voice && r.start < end && r.end > start {
            r.path = plan.assets[asset_id(&r.clip).unwrap()].clone();
        }
    }
    if store.get(project_id)?.timeline.as_ref() != Some(&plan.document) {
        return Err(
            "Timeline vừa thay đổi khi chuẩn bị export. Hãy xuất lại để lấy snapshot mới.".into(),
        );
    }
    Ok(plan)
}
pub fn capture(
    store: &ProjectStore,
    project_id: &str,
    options: &ExportOptions,
) -> Result<Snapshot, String> {
    let mut plan = build(store, project_id, options)?;
    let root = store.data_directory(project_id, "cache/render")?;
    let directory = tempfile::Builder::new()
        .prefix("timeline-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let output_directory = crate::render_service::resolve_export_directory(
        store,
        project_id,
        options.output_directory.as_deref(),
    )?;
    crate::disk_space::require(
        directory.path(),
        plan.audio
            .iter()
            .filter(|r| r.voice && r.start < plan.end && r.end > plan.start)
            .filter_map(|r| fs::metadata(&r.path).ok())
            .map(|m| m.len())
            .sum(),
    )?;
    let mut frozen = HashMap::<PathBuf, PathBuf>::new();
    let mut freeze = |path: &PathBuf, copy: bool| -> Result<PathBuf, String> {
        if let Some(p) = frozen.get(path) {
            return Ok(p.clone());
        }
        let target = directory.path().join(format!(
            "media-{}.{}",
            frozen.len(),
            path.extension().unwrap_or_default().to_string_lossy()
        ));
        if copy || fs::hard_link(path, &target).is_err() {
            fs::copy(path, &target).map_err(|e| {
                format!("Không chụp được tài nguyên export {}: {e}", path.display())
            })?;
        }
        frozen.insert(path.clone(), target.clone());
        Ok(target)
    };
    for path in plan.assets.values_mut() {
        *path = freeze(path, false)?;
    }
    for r in &mut plan.audio {
        if r.start < plan.end && r.end > plan.start {
            r.path = freeze(&r.path, r.voice)?;
        }
    }
    // The document and sidecars are never read again by the rendering worker.
    fs::write(
        directory.path().join("timeline.json"),
        serde_json::to_vec_pretty(&plan.document).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(Snapshot {
        plan,
        directory,
        output_directory,
    })
}
/// Caption intervals include empty states and join overlapping segments like preview.
pub fn caption_states(plan: &Plan, c: &Clip) -> Result<Vec<(u64, u64, String)>, String> {
    let Some((from, to)) = plan.window(c)? else {
        return Ok(vec![]);
    };
    if let ClipContent::Text { text } = &c.content {
        return Ok(vec![(from, to, text.clone())]);
    }
    let transcript = id(c)
        .and_then(|id| plan.sources.get(id))
        .and_then(|s| s.transcript.as_ref());
    let mut segments = vec![];
    let mut cuts = vec![from, to];
    for s in transcript.into_iter().flat_map(|t| &t.segments) {
        if let Some((a, b)) =
            c.map_source_interval(seconds_to_time_us(s.start)?, seconds_to_time_us(s.end)?)?
        {
            let a = a.max(from);
            let b = b.min(to);
            if a < b {
                cuts.extend([a, b]);
                segments.push((
                    a,
                    b,
                    if s.translated.is_empty() {
                        s.source.clone()
                    } else {
                        s.translated.clone()
                    },
                ));
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    Ok(cuts
        .windows(2)
        .map(|w| {
            (
                w[0],
                w[1],
                segments
                    .iter()
                    .filter(|s| s.0 <= w[0] && s.1 > w[0])
                    .map(|s| s.2.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })
        .collect())
}
