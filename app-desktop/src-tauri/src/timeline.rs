//! Persisted editor foundation. All intervals are half-open integer microseconds.
use crate::project_service::{
    self, BlurRegion, DubbingSettings, Project, ProjectLogo, ProjectSource, ProjectTextOverlay,
    SubtitleSettings, TranscriptMetadata, TranscriptSettings, TranslationSettings,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const MAX_TIME_US: u64 = 9_007_199_254_740_991;
pub const TIMEBASE: u64 = 1_000_000;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rational {
    pub numerator: u32,
    pub denominator: u32,
}
impl Default for Rational {
    fn default() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }
}
impl Rational {
    fn validate(self) -> Result<(), String> {
        if self.numerator == 0 || self.denominator == 0 {
            return Err("Tỷ lệ thời gian không hợp lệ.".into());
        }
        Ok(())
    }
    pub(crate) fn from_fps(fps: f64) -> Result<Self, String> {
        if !fps.is_finite() || fps <= 0.0 || fps * 1_000_000.0 > u32::MAX as f64 {
            return Err("FPS nguồn không hợp lệ.".into());
        }
        // Preserve NTSC rates instead of accumulating decimal rounding drift.
        for numerator in [24_000, 30_000, 60_000, 120_000] {
            if (fps - numerator as f64 / 1001.0).abs() < 0.0001 {
                return Ok(Self {
                    numerator,
                    denominator: 1001,
                });
            }
        }
        let numerator = (fps * 1_000_000.0 + 0.5).floor() as u32;
        let mut a = numerator;
        let mut b = 1_000_000;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let result = Self {
            numerator: numerator / a,
            denominator: 1_000_000 / a,
        };
        result.validate()?;
        Ok(result)
    }
}
fn time(value: u64) -> Result<u64, String> {
    if value > MAX_TIME_US {
        return Err("Thời gian vượt miền số nguyên an toàn.".into());
    }
    Ok(value)
}
fn safe(value: u128) -> Result<u64, String> {
    if value > MAX_TIME_US as u128 {
        return Err("Thời gian timeline bị tràn.".into());
    }
    Ok(value as u64)
}
pub fn seconds_to_time_us(seconds: f64) -> Result<u64, String> {
    if !seconds.is_finite() || seconds < 0.0 || seconds > MAX_TIME_US as f64 / TIMEBASE as f64 {
        return Err("Thời gian nguồn không hợp lệ.".into());
    }
    time((seconds * TIMEBASE as f64 + 0.5).floor() as u64)
}
pub fn rescale_time(value: u64, rate: Rational) -> Result<u64, String> {
    time(value)?;
    rate.validate()?;
    safe(
        (value as u128 * rate.numerator as u128 + (rate.denominator / 2) as u128)
            / rate.denominator as u128,
    )
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: String,
    pub kind: MediaKind,
    pub relative_path: String,
    pub duration_us: Option<u64>,
    pub frame_rate: Option<Rational>,
    pub variable_frame_rate: Option<bool>,
    /// Original probe metadata is retained during migration, including audio/subtitle streams.
    pub metadata: ProjectSource,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoSettings {
    pub transcript: Option<TranscriptMetadata>,
    pub translation: TranslationSettings,
    pub subtitle: SubtitleSettings,
    pub dubbing: DubbingSettings,
    pub transcript_settings: TranscriptSettings,
    pub blur_regions: Vec<BlurRegion>,
    pub logos: Vec<ProjectLogo>,
    pub text_overlays: Vec<ProjectTextOverlay>,
}
impl VideoSettings {
    pub(crate) fn from_project(project: &Project) -> Self {
        Self {
            transcript: project.transcript.clone(),
            translation: project.translation.clone(),
            subtitle: project.subtitle.clone(),
            dubbing: project.dubbing.clone(),
            transcript_settings: project.transcript_settings.clone(),
            blur_regions: project.blur_regions.clone(),
            logos: project.logos.clone(),
            text_overlays: project.text_overlays.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoContent {
    pub id: String,
    pub asset_id: String,
    pub revision: u64,
    pub settings: VideoSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    Visual,
    Audio,
    Subtitle,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    pub name: String,
    pub order: u32,
    pub locked: bool,
    pub enabled: bool,
    pub muted: bool,
    pub solo: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    Contain,
    Cover,
    Stretch,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClipTransform {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub fit: Fit,
    pub crop: Crop,
}
impl Default for ClipTransform {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.5,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            flip_x: false,
            flip_y: false,
            fit: Fit::Contain,
            crop: Crop {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClipContent {
    Video {
        asset_id: String,
        video_content_id: String,
    },
    Image {
        asset_id: String,
    },
    Audio {
        asset_id: String,
    },
    SourceAudio {
        asset_id: String,
        video_content_id: String,
    },
    Text {
        text: String,
    },
    Subtitles {
        video_content_id: String,
    },
    Voice {
        video_content_id: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub track_id: String,
    pub start_us: u64,
    pub source_in_us: u64,
    pub source_out_us: u64,
    pub rate: Rational,
    pub content: ClipContent,
    pub transform: ClipTransform,
    pub linked_clip_id: Option<String>,
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ClipStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ClipAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<crate::timeline_effects::VisualEffects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<crate::timeline_effects::Transition>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipAudio {
    pub volume: f64,
    pub muted: bool,
    pub fade_in_us: u64,
    pub fade_out_us: u64,
    pub voice_enabled: bool,
    pub voice_speed: f64,
    pub ducking_enabled: bool,
    pub ducking_amount: f64,
    pub duck_attack_us: u64,
    pub duck_release_us: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipStyle {
    pub font_family: String,
    pub font_size: f64,
    pub font_weight: u32,
    pub color: String,
    pub background_color: String,
    pub background_opacity: f64,
    pub alignment: String,
}
impl Clip {
    pub fn duration_us(&self) -> Result<u64, String> {
        time(self.source_in_us)?;
        time(self.source_out_us)?;
        self.rate.validate()?;
        if self.source_out_us <= self.source_in_us {
            return Err("Clip phải có khoảng nguồn dương.".into());
        }
        let value =
            (self.source_out_us - self.source_in_us) as u128 * self.rate.denominator as u128;
        safe(value.div_ceil(self.rate.numerator as u128))
    }
    pub fn end_us(&self) -> Result<u64, String> {
        time(self.start_us)?;
        safe(self.start_us as u128 + self.duration_us()? as u128)
    }
    pub fn timeline_to_source_us(&self, at: u64) -> Result<Option<u64>, String> {
        time(at)?;
        let end = self.end_us()?;
        if at < self.start_us || at >= end {
            return Ok(None);
        }
        let value = self.source_in_us as u128
            + ((at - self.start_us) as u128 * self.rate.numerator as u128
                + (self.rate.denominator / 2) as u128)
                / self.rate.denominator as u128;
        Ok(Some(safe(value)?.min(self.source_out_us - 1)))
    }
    pub fn source_to_timeline_us(&self, at: u64) -> Result<Option<u64>, String> {
        time(at)?;
        let end = self.end_us()?;
        if at < self.source_in_us || at >= self.source_out_us {
            return Ok(None);
        }
        let value = self.start_us as u128
            + ((at - self.source_in_us) as u128 * self.rate.denominator as u128
                + (self.rate.numerator / 2) as u128)
                / self.rate.numerator as u128;
        Ok(Some(safe(value)?.min(end - 1)))
    }
    pub fn map_source_interval(&self, start: u64, end: u64) -> Result<Option<(u64, u64)>, String> {
        time(start)?;
        time(end)?;
        let from = start.max(self.source_in_us);
        let to = end.min(self.source_out_us);
        if from >= to {
            return Ok(None);
        }
        let start = self.source_to_timeline_us(from)?.unwrap();
        let end = if to == self.source_out_us {
            self.end_us()?
        } else {
            self.source_to_timeline_us(to)?.unwrap()
        };
        Ok(Some((start, end.max(start + 1).min(self.end_us()?))))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    pub background_color: String,
}
impl Default for TimelineSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_rate: Rational {
                numerator: 30,
                denominator: 1,
            },
            background_color: "#000000".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyBinding {
    pub asset_id: String,
    pub video_content_id: String,
    pub clip_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDocument {
    pub schema_version: u32,
    pub settings: TimelineSettings,
    pub assets: Vec<MediaAsset>,
    pub video_contents: Vec<VideoContent>,
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub legacy_binding: Option<LegacyBinding>,
}
impl Default for TimelineDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            settings: TimelineSettings::default(),
            assets: vec![],
            video_contents: vec![],
            tracks: vec![Track {
                id: Uuid::new_v4().to_string(),
                kind: TrackKind::Visual,
                name: "Video".into(),
                order: 0,
                locked: false,
                enabled: true,
                muted: false,
                solo: false,
            }],
            clips: vec![],
            legacy_binding: None,
        }
    }
}
impl TimelineDocument {
    pub fn duration_us(&self) -> Result<u64, String> {
        self.clips
            .iter()
            .try_fold(0, |end, clip| Ok(end.max(clip.end_us()?)))
    }
    pub fn from_legacy(project: &Project) -> Result<Self, String> {
        let mut result = Self::default();
        if let Some(width) = project.source.width {
            result.settings.width = width;
        }
        if let Some(height) = project.source.height {
            result.settings.height = height;
        }
        if let Some(fps) = project.source.fps {
            result.settings.frame_rate = Rational::from_fps(fps)?;
        }
        if project.source.video_path.is_some() {
            result.bind_legacy(project)?;
        }
        Ok(result)
    }
    fn bind_legacy(&mut self, project: &Project) -> Result<(), String> {
        let id = Uuid::new_v4().to_string();
        let content_id = Uuid::new_v4().to_string();
        let duration = project
            .source
            .duration
            .map(seconds_to_time_us)
            .transpose()?;
        self.assets.push(MediaAsset {
            id: id.clone(),
            kind: MediaKind::Video,
            relative_path: project
                .source
                .video_path
                .clone()
                .ok_or("Thiếu video nguồn.")?,
            duration_us: duration,
            frame_rate: project.source.fps.map(Rational::from_fps).transpose()?,
            variable_frame_rate: None,
            metadata: project.source.clone(),
        });
        self.video_contents.push(VideoContent {
            id: content_id.clone(),
            asset_id: id.clone(),
            revision: 0,
            settings: VideoSettings::from_project(project),
        });
        let clip_id = if let Some(duration) = duration {
            let clip_id = Uuid::new_v4().to_string();
            let track = self
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Visual)
                .ok_or("Thiếu track hình ảnh.")?;
            self.clips.push(Clip {
                id: clip_id.clone(),
                track_id: track.id.clone(),
                start_us: 0,
                source_in_us: 0,
                source_out_us: duration,
                rate: Rational::default(),
                content: ClipContent::Video {
                    asset_id: id.clone(),
                    video_content_id: content_id.clone(),
                },
                transform: ClipTransform::default(),
                linked_clip_id: None,
                group_id: None,
                style: None,
                audio: None,
                effects: None,
                transition_in: None,
            });
            Some(clip_id)
        } else {
            None
        }; // Missing metadata remains recoverable; never invent a duration.
        self.legacy_binding = Some(LegacyBinding {
            asset_id: id,
            video_content_id: content_id,
            clip_id,
        });
        Ok(())
    }
}

fn validate_id(id: &str, ids: &mut HashSet<String>) -> Result<(), String> {
    if Uuid::parse_str(id).is_err() || !ids.insert(id.into()) {
        return Err("ID timeline không hợp lệ hoặc bị trùng.".into());
    }
    Ok(())
}
fn validate_settings(settings: &VideoSettings) -> Result<(), String> {
    project_service::validate_translation_custom_prompt(&settings.translation.custom_prompt)?;
    project_service::validate_translation_context(&settings.translation.context)?;
    if let Some(transcript) = &settings.transcript {
        project_service::validate_relative_asset(&transcript.path, "subtitles")?;
    }
    crate::overlays_service::validate_subtitle_settings(&settings.subtitle)?;
    crate::overlays_service::validate_blur_regions(&settings.blur_regions)?;
    crate::overlays_service::validate_text_overlays(&settings.text_overlays)?;
    project_service::validate_logo_documents(&settings.logos)?;
    let d = &settings.dubbing;
    if ![
        d.original_volume,
        d.dubbed_volume,
        d.ducking_amount,
        d.dialogue_reduction_amount,
    ]
    .iter()
    .all(|v| v.is_finite() && *v >= 0.0)
        || d.ducking_amount > 1.0
        || d.dialogue_reduction_amount > 1.0
        || d.speed.is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        return Err("Thông số lồng tiếng không hợp lệ.".into());
    }
    let mut segments = HashSet::new();
    for segment in &settings.translation.segments {
        if segment.id.is_empty()
            || !segments.insert(&segment.id)
            || segment.end_ms <= segment.start_ms
        {
            return Err("Đoạn dịch không hợp lệ.".into());
        }
        if let Some(tts) = &segment.tts {
            project_service::validate_relative_asset(&tts.audio_relative_path, "audio/tts")?;
        }
    }
    Ok(())
}
pub fn validate(document: &TimelineDocument) -> Result<(), String> {
    if document.schema_version != 1 {
        return Err("Phiên bản timeline chưa được hỗ trợ.".into());
    }
    document.settings.frame_rate.validate()?;
    if !(1..=16384).contains(&document.settings.width)
        || !(1..=16384).contains(&document.settings.height)
        || document.settings.background_color.len() != 7
        || !document.settings.background_color.starts_with('#')
        || !document.settings.background_color[1..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err("Khung hình timeline không hợp lệ.".into());
    }
    let mut ids = HashSet::new();
    let mut assets = HashMap::new();
    let mut contents = HashMap::new();
    let mut tracks = HashMap::new();
    for asset in &document.assets {
        validate_id(&asset.id, &mut ids)?;
        project_service::validate_relative_asset(&asset.relative_path, "media")?;
        if let Some(path) = &asset.metadata.video_path {
            project_service::validate_relative_asset(path, "media")?;
            if path != &asset.relative_path {
                return Err("Đường dẫn asset không khớp metadata.".into());
            }
        }
        if asset
            .duration_us
            .is_some_and(|duration| duration == 0 || duration > MAX_TIME_US)
            || asset.metadata.width == Some(0)
            || asset.metadata.height == Some(0)
            || asset
                .metadata
                .duration
                .is_some_and(|v| !v.is_finite() || v <= 0.0)
            || asset
                .metadata
                .fps
                .is_some_and(|v| !v.is_finite() || v <= 0.0)
        {
            return Err("Metadata asset không hợp lệ.".into());
        }
        if let Some(rate) = asset.frame_rate {
            rate.validate()?;
        }
        if let (Some(seconds), Some(duration)) = (asset.metadata.duration, asset.duration_us) {
            if seconds_to_time_us(seconds)? != duration {
                return Err("Thời lượng asset không khớp metadata.".into());
            }
        }
        assets.insert(asset.id.as_str(), asset);
    }
    for content in &document.video_contents {
        validate_id(&content.id, &mut ids)?;
        time(content.revision)?;
        if !assets
            .get(content.asset_id.as_str())
            .is_some_and(|a| a.kind == MediaKind::Video)
        {
            return Err("Nội dung dịch không có video nguồn hợp lệ.".into());
        }
        validate_settings(&content.settings)?;
        contents.insert(content.id.as_str(), content);
    }
    let mut orders = HashSet::new();
    for track in &document.tracks {
        validate_id(&track.id, &mut ids)?;
        if !orders.insert(track.order) || track.name.trim().is_empty() {
            return Err("Thứ tự hoặc tên track không hợp lệ.".into());
        }
        tracks.insert(track.id.as_str(), track);
    }
    let mut intervals: HashMap<&str, Vec<&Clip>> = HashMap::new();
    let mut clips = HashMap::new();
    for clip in &document.clips {
        validate_id(&clip.id, &mut ids)?;
        crate::timeline_effects::validate_effects(clip)?;
        let track = tracks
            .get(clip.track_id.as_str())
            .ok_or("Clip không có track hợp lệ.")?;
        clip.end_us()?;
        let (expected, asset, content) = match &clip.content {
            ClipContent::Video {
                asset_id,
                video_content_id,
            } => (
                TrackKind::Visual,
                Some((asset_id, MediaKind::Video)),
                Some(video_content_id),
            ),
            ClipContent::Image { asset_id } => {
                (TrackKind::Visual, Some((asset_id, MediaKind::Image)), None)
            }
            ClipContent::Audio { asset_id } => {
                (TrackKind::Audio, Some((asset_id, MediaKind::Audio)), None)
            }
            ClipContent::SourceAudio {
                asset_id,
                video_content_id,
            } => (
                TrackKind::Audio,
                Some((asset_id, MediaKind::Video)),
                Some(video_content_id),
            ),
            ClipContent::Text { .. } => (TrackKind::Visual, None, None),
            ClipContent::Subtitles { video_content_id } => {
                (TrackKind::Subtitle, None, Some(video_content_id))
            }
            ClipContent::Voice { video_content_id } => {
                (TrackKind::Audio, None, Some(video_content_id))
            }
        };
        if track.kind != expected {
            return Err("Loại clip không phù hợp track.".into());
        }
        let source = if let Some((id, kind)) = asset {
            let source = assets
                .get(id.as_str())
                .ok_or("Clip không có asset hợp lệ.")?;
            if source.kind != kind {
                return Err("Clip tham chiếu sai loại asset.".into());
            }
            Some(*source)
        } else {
            None
        };
        let content = content
            .map(|id| {
                contents
                    .get(id.as_str())
                    .copied()
                    .ok_or("Clip không có nội dung dịch hợp lệ.")
            })
            .transpose()?;
        if let (Some(source), Some(content)) = (source, content) {
            if content.asset_id != source.id {
                return Err("Clip ghép nội dung dịch từ video khác.".into());
            }
        }
        let timed_source = source
            .filter(|s| s.kind != MediaKind::Image)
            .or_else(|| content.and_then(|c| assets.get(c.asset_id.as_str()).copied()));
        if let Some(source) = timed_source {
            let duration = source
                .duration_us
                .ok_or("Chưa có thời lượng nguồn của clip.")?;
            if clip.source_out_us > duration {
                return Err("Điểm cắt vượt thời lượng nguồn.".into());
            }
        }
        let t = &clip.transform;
        if let Some(audio) = &clip.audio {
            if !matches!(
                clip.content,
                ClipContent::Video { .. }
                    | ClipContent::Audio { .. }
                    | ClipContent::SourceAudio { .. }
                    | ClipContent::Voice { .. }
            ) || !audio.volume.is_finite()
                || !(0.0..=2.0).contains(&audio.volume)
                || !audio.voice_speed.is_finite()
                || !(0.5..=2.0).contains(&audio.voice_speed)
                || !audio.ducking_amount.is_finite()
                || !(0.0..=1.0).contains(&audio.ducking_amount)
                || audio.fade_in_us > MAX_TIME_US
                || audio.fade_out_us > MAX_TIME_US
                || audio.duck_attack_us > 500_000
                || audio.duck_release_us > 2_000_000
            {
                return Err("Thuộc tính âm thanh clip không hợp lệ.".into());
            }
        }
        if let ClipContent::Text { text } = &clip.content {
            if text.trim().is_empty() || text.chars().count() > 10000 {
                return Err("Nội dung chữ phải từ 1 đến 10.000 ký tự.".into());
            }
        }
        if let Some(style) = &clip.style {
            let color = |s: &str| {
                s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
            };
            if !matches!(
                clip.content,
                ClipContent::Text { .. } | ClipContent::Subtitles { .. }
            ) || style.font_family.trim().is_empty()
                || style.font_family.len() > 96
                || !style.font_size.is_finite()
                || !(8.0..=240.0).contains(&style.font_size)
                || !(100..=900).contains(&style.font_weight)
                || !color(&style.color)
                || !color(&style.background_color)
                || !style.background_opacity.is_finite()
                || !(0.0..=1.0).contains(&style.background_opacity)
                || !["left", "center", "right"].contains(&style.alignment.as_str())
            {
                return Err("Kiểu chữ clip không hợp lệ.".into());
            }
        }
        let crop = &t.crop;
        if ![
            t.x,
            t.y,
            t.scale_x,
            t.scale_y,
            t.rotation,
            t.opacity,
            crop.x,
            crop.y,
            crop.width,
            crop.height,
        ]
        .iter()
        .all(|v| v.is_finite())
            || t.scale_x <= 0.0
            || t.scale_y <= 0.0
            || t.scale_x > 100.0
            || t.scale_y > 100.0
            || !(0.0..=1.0).contains(&t.opacity)
            || crop.x < 0.0
            || crop.y < 0.0
            || crop.width <= 0.0
            || crop.height <= 0.0
            || crop.x + crop.width > 1.0 + 1e-9
            || crop.y + crop.height > 1.0 + 1e-9
        {
            return Err("Biến đổi hình ảnh clip không hợp lệ.".into());
        }
        if clip
            .group_id
            .as_ref()
            .is_some_and(|id| Uuid::parse_str(id).is_err())
        {
            return Err("ID nhóm clip không hợp lệ.".into());
        }
        intervals.entry(&clip.track_id).or_default().push(clip);
        clips.insert(clip.id.as_str(), clip);
    }
    crate::timeline_effects::validate_transitions(document)?;
    for list in intervals.values_mut() {
        list.sort_by_key(|c| c.start_us);
        let mut active: Vec<&Clip> = Vec::new();
        for c in list.iter() {
            active.retain(|a| a.end_us().is_ok_and(|end| end > c.start_us));
            for a in &active {
                if !crate::timeline_effects::transition_overlap(a, c, document)? {
                    return Err(
                        "Clip chồng lấn trên cùng một track ngoài chuyển cảnh hợp lệ.".into(),
                    );
                }
            }
            active.push(c);
        }
    }
    for clip in &document.clips {
        if let Some(parent) = &clip.linked_clip_id {
            let parent = clips
                .get(parent.as_str())
                .ok_or("Clip liên kết không tồn tại.")?;
            let ClipContent::Video {
                video_content_id, ..
            } = &parent.content
            else {
                return Err("Lớp liên kết phải thuộc một clip video.".into());
            };
            if matches!(clip.content, ClipContent::Video { .. }) {
                return Err(
                    "Clip video dùng nhóm để liên kết, không dùng liên kết phụ thuộc.".into(),
                );
            }
            if let ClipContent::Subtitles {
                video_content_id: child,
            }
            | ClipContent::Voice {
                video_content_id: child,
            }
            | ClipContent::SourceAudio {
                video_content_id: child,
                ..
            } = &clip.content
            {
                if child != video_content_id {
                    return Err("Phụ đề/giọng đọc liên kết sai video.".into());
                }
            }
        }
    }
    if let Some(binding) = &document.legacy_binding {
        let asset = assets
            .get(binding.asset_id.as_str())
            .ok_or("Binding nguồn cũ không hợp lệ.")?;
        let content = contents
            .get(binding.video_content_id.as_str())
            .ok_or("Binding nội dung cũ không hợp lệ.")?;
        if asset.kind != MediaKind::Video || content.asset_id != asset.id {
            return Err("Binding ghép sai video nguồn.".into());
        }
        if let Some(id) = &binding.clip_id {
            let clip = clips
                .get(id.as_str())
                .ok_or("Binding clip cũ không hợp lệ.")?;
            if clip.content
                != (ClipContent::Video {
                    asset_id: binding.asset_id.clone(),
                    video_content_id: binding.video_content_id.clone(),
                })
            {
                return Err("Binding clip không khớp video nguồn.".into());
            }
        }
    }
    document.duration_us()?;
    Ok(())
}

/// Bridge existing single-video commands without overwriting timeline placement or other sources.
pub fn reconcile_legacy(project: &mut Project, previous: &Project) -> Result<(), String> {
    let mut document = project.timeline.clone().ok_or("Thiếu dữ liệu timeline.")?;
    validate(&document)?;
    let legacy = VideoSettings::from_project(project);
    let old_legacy = VideoSettings::from_project(previous);
    if let Some(binding) = document.legacy_binding.clone() {
        let asset_index = document
            .assets
            .iter()
            .position(|a| a.id == binding.asset_id)
            .ok_or("Thiếu asset cũ.")?;
        let content_index = document
            .video_contents
            .iter()
            .position(|c| c.id == binding.video_content_id)
            .ok_or("Thiếu nội dung cũ.")?;
        let asset = &document.assets[asset_index];
        let same_source = match (
            &project.source.checksum_sha256,
            &asset.metadata.checksum_sha256,
        ) {
            (Some(a), Some(b)) => a == b,
            _ => project.source.video_path == asset.metadata.video_path,
        };
        let source_changed = project.source != previous.source;
        let settings_changed = legacy != old_legacy;
        if source_changed {
            if let Some(old_asset) = previous
                .timeline
                .as_ref()
                .and_then(|d| d.assets.iter().find(|a| a.id == asset.id))
            {
                if old_asset.metadata != asset.metadata && asset.metadata != project.source {
                    return Err(
                        "Nguồn và timeline đã được sửa đồng thời; hãy tải lại dự án.".into(),
                    );
                }
            }
        }
        if source_changed && !same_source {
            if document.clips.iter().any(|clip| match &clip.content {
                ClipContent::Video { asset_id, .. } => {
                    asset_id == &binding.asset_id && Some(&clip.id) != binding.clip_id.as_ref()
                }
                ClipContent::Subtitles { video_content_id }
                | ClipContent::Voice { video_content_id }
                | ClipContent::SourceAudio {
                    video_content_id, ..
                } => video_content_id == &binding.video_content_id,
                _ => {
                    clip.linked_clip_id.as_ref() == binding.clip_id.as_ref()
                        && clip.linked_clip_id.is_some()
                }
            }) || document
                .video_contents
                .iter()
                .any(|c| c.asset_id == binding.asset_id && c.id != binding.video_content_id)
                || document
                    .assets
                    .iter()
                    .any(|a| a.id != binding.asset_id && a.relative_path == asset.relative_path)
                || old_legacy.transcript.as_ref().is_some_and(|transcript| {
                    document.video_contents.iter().any(|c| {
                        c.id != binding.video_content_id
                            && c.settings
                                .transcript
                                .as_ref()
                                .is_some_and(|t| t.path == transcript.path)
                    })
                })
            {
                return Err("Video đang được nhiều clip sử dụng; không thể thay thế bằng thao tác một video.".into());
            }
            document
                .clips
                .retain(|c| Some(&c.id) != binding.clip_id.as_ref());
            document.assets.remove(asset_index);
            document.video_contents.remove(content_index);
            document.legacy_binding = None;
            if project.source.video_path.is_some() {
                document.bind_legacy(project)?;
            }
        } else {
            if source_changed {
                document.assets[asset_index].metadata = project.source.clone();
                document.assets[asset_index].relative_path = project
                    .source
                    .video_path
                    .clone()
                    .ok_or("Thiếu đường dẫn video.")?;
                document.assets[asset_index].duration_us = project
                    .source
                    .duration
                    .map(seconds_to_time_us)
                    .transpose()?;
                document.assets[asset_index].frame_rate =
                    project.source.fps.map(Rational::from_fps).transpose()?;
            } else {
                // A timeline edit is authoritative when legacy fields were not edited.
                project.source = document.assets[asset_index].metadata.clone();
            }
            let content = &mut document.video_contents[content_index];
            if settings_changed {
                let old_content = previous
                    .timeline
                    .as_ref()
                    .and_then(|d| d.video_contents.iter().find(|c| c.id == content.id));
                if old_content.is_some_and(|old| {
                    old.settings != content.settings && content.settings != legacy
                }) {
                    return Err(
                        "Nội dung nguồn và timeline đã được sửa đồng thời; hãy tải lại dự án."
                            .into(),
                    );
                }
                content.settings = legacy;
            } else {
                project.transcript = content.settings.transcript.clone();
                project.translation = content.settings.translation.clone();
                project.subtitle = content.settings.subtitle.clone();
                project.dubbing = content.settings.dubbing.clone();
                project.transcript_settings = content.settings.transcript_settings.clone();
                project.blur_regions = content.settings.blur_regions.clone();
                project.logos = content.settings.logos.clone();
                project.text_overlays = content.settings.text_overlays.clone();
            }
            // A null clip binding after an editor delete is intentional, not a missing migration.
            if binding.clip_id.is_none()
                && previous
                    .timeline
                    .as_ref()
                    .and_then(|d| d.assets.iter().find(|a| a.id == binding.asset_id))
                    .is_some_and(|a| a.duration_us.is_none())
            {
                if let Some(duration) = document.assets[asset_index].duration_us {
                    let id = Uuid::new_v4().to_string();
                    let track = document
                        .tracks
                        .iter()
                        .find(|t| t.kind == TrackKind::Visual)
                        .ok_or("Thiếu track hình ảnh.")?;
                    document.clips.push(Clip {
                        id: id.clone(),
                        track_id: track.id.clone(),
                        start_us: 0,
                        source_in_us: 0,
                        source_out_us: duration,
                        rate: Rational::default(),
                        content: ClipContent::Video {
                            asset_id: binding.asset_id,
                            video_content_id: binding.video_content_id,
                        },
                        transform: ClipTransform::default(),
                        linked_clip_id: None,
                        group_id: None,
                        style: None,
                        audio: None,
                        effects: None,
                        transition_in: None,
                    });
                    document.legacy_binding.as_mut().unwrap().clip_id = Some(id);
                }
            }
        }
    } else if project.source.video_path.is_some() && project.source != previous.source {
        // First import into an empty project. Set canvas from its real metadata.
        let defaults = TimelineDocument::from_legacy(project)?;
        if document.assets.is_empty() && document.clips.is_empty() {
            document = defaults;
        } else {
            document.bind_legacy(project)?;
        }
    }
    // Each independently edited video's settings receives a monotonic revision.
    if let Some(previous) = &previous.timeline {
        for content in &mut document.video_contents {
            if let Some(old) = previous
                .video_contents
                .iter()
                .find(|old| old.id == content.id)
            {
                if content.settings != old.settings {
                    content.revision =
                        time(old.revision.checked_add(1).ok_or("Revision bị tràn.")?)?;
                } else {
                    content.revision = old.revision;
                }
            }
        }
    }
    validate(&document)?;
    project.timeline = Some(document);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::ProjectStore;
    use serde_json::{json, Value};
    use std::fs;

    fn legacy_project() -> (tempfile::TempDir, ProjectStore, Project) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let mut project = store.create("Phim Việt Nam").unwrap();
        project.source.video_path = Some("media/source/Việt Nam.mp4".into());
        project.source.duration = Some(120.0);
        project.source.width = Some(1080);
        project.source.height = Some(1920);
        project.source.fps = Some(30000.0 / 1001.0);
        project.source.checksum_sha256 = Some("original-checksum".into());
        project = store.update_project(&project).unwrap();
        (dir, store, project)
    }
    #[test]
    fn shared_time_contract_and_safe_arithmetic() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../test-fixtures/timeline-time.json")).unwrap();
        for row in fixture["cases"].as_array().unwrap() {
            let clip = Clip {
                id: "fixture".into(),
                track_id: "fixture".into(),
                start_us: row["startUs"].as_u64().unwrap(),
                source_in_us: row["sourceInUs"].as_u64().unwrap(),
                source_out_us: row["sourceOutUs"].as_u64().unwrap(),
                rate: serde_json::from_value(row["rate"].clone()).unwrap(),
                content: ClipContent::Text {
                    text: "fixture".into(),
                },
                transform: ClipTransform::default(),
                linked_clip_id: None,
                group_id: None,
                style: None,
                audio: None,
                effects: None,
                transition_in: None,
            };
            assert_eq!(
                clip.duration_us().unwrap(),
                row["durationUs"].as_u64().unwrap()
            );
            assert_eq!(clip.end_us().unwrap(), row["endUs"].as_u64().unwrap());
            for point in row["timelinePoints"].as_array().unwrap() {
                assert_eq!(
                    clip.timeline_to_source_us(point[0].as_u64().unwrap())
                        .unwrap(),
                    point[1].as_u64()
                );
            }
            for point in row["sourcePoints"].as_array().unwrap() {
                assert_eq!(
                    clip.source_to_timeline_us(point[0].as_u64().unwrap())
                        .unwrap(),
                    point[1].as_u64()
                );
            }
            for interval in row["intervals"].as_array().unwrap() {
                let expected = interval[2]
                    .as_array()
                    .map(|v| (v[0].as_u64().unwrap(), v[1].as_u64().unwrap()));
                assert_eq!(
                    clip.map_source_interval(
                        interval[0].as_u64().unwrap(),
                        interval[1].as_u64().unwrap()
                    )
                    .unwrap(),
                    expected
                );
            }
        }
        assert_eq!(seconds_to_time_us(0.0000005).unwrap(), 1);
        assert_eq!(
            rescale_time(
                10,
                Rational {
                    numerator: 1,
                    denominator: 4
                }
            )
            .unwrap(),
            3
        );
        assert!(rescale_time(
            MAX_TIME_US,
            Rational {
                numerator: 2,
                denominator: 1
            }
        )
        .is_err());
        assert!(rescale_time(
            1,
            Rational {
                numerator: 1,
                denominator: 0
            }
        )
        .is_err());
        for seconds in [-1.0, f64::NAN, f64::INFINITY, MAX_TIME_US as f64] {
            assert!(seconds_to_time_us(seconds).is_err());
        }
    }
    #[test]
    fn v11_migration_preserves_content_files_visuals_and_stable_ids() {
        let (_dir, store, mut project) = legacy_project();
        let root = store.project_directory(&project.id).unwrap();
        let transcript: crate::transcript_service::Transcript = serde_json::from_value(json!({
            "schemaVersion":1, "projectId":project.id, "sourceType":"ocr", "sourceTrack":null,
            "language":"en", "videoChecksum":project.source.checksum_sha256,
            "segments":[{"id":"segment-1", "start":1.0, "end":2.0, "source":"hello", "translated":"xin chào",
                "sourceRevision":2, "translatedRevision":2, "speakerId":null, "sourceType":"ocr", "translationStatus":"edited",
                "tts":{"audioPath":"audio/tts/voice.wav", "duration":1.0, "fingerprint":"original", "status":"ready", "voice":"Ngọc Huyền"}}]
        })).unwrap();
        let transcript_bytes = serde_json::to_vec_pretty(&transcript).unwrap();
        fs::write(root.join("subtitles/transcript.json"), &transcript_bytes).unwrap();
        fs::write(root.join("audio/tts/voice.wav"), b"original audio bytes").unwrap();
        fs::write(
            root.join("media/source/Việt Nam.mp4"),
            b"original video bytes",
        )
        .unwrap();
        project.transcript = Some(serde_json::from_value(json!({ "status":"completed", "path":"subtitles/transcript.json", "segmentCount":1 })).unwrap());
        project.translation.segments = serde_json::from_value(json!([{ "id":"segment-1", "startMs":1000,"endMs":2000,
            "source":"hello","translated":"xin chào","translationStatus":"edited",
            "tts":{"audioRelativePath":"audio/tts/voice.wav","fingerprint":"original","durationMs":1000,"status":"ready"} }])).unwrap();
        project.logos = serde_json::from_value(json!([{ "id":Uuid::new_v4().to_string(), "relativePath":"media/overlays/logo.png", "originalName":"logo.png",
            "x":0.1,"y":0.1,"width":0.2,"height":0.2,"enabled":true,"opacity":0.8,"maintainAspectRatio":true }])).unwrap();
        project.text_overlays = serde_json::from_value(json!([{ "id":Uuid::new_v4().to_string(), "text":"Chữ Việt", "x":0.1,"y":0.1,"width":0.5,"height":0.2,
            "enabled":true,"fontFamily":"Arial","fontSize":24,"fontWeight":700,"color":"#FFFFFF","opacity":1.0,"alignment":"center" }])).unwrap();
        project.blur_regions = serde_json::from_value(json!([{"id":Uuid::new_v4().to_string(),
            "x":0.1,"y":0.7,"width":0.8,"height":0.2,"startMs":1000,"endMs":2000,"enabled":true,"strength":20.0,
            "role":"subtitle","effect":"sttn","textRemovalMode":"ocrGuided"}])).unwrap();
        project = store.update_project(&project).unwrap();
        let mut old = serde_json::to_value(&project).unwrap();
        old["schemaVersion"] = json!(11);
        old.as_object_mut().unwrap().remove("timeline");
        let bytes = serde_json::to_vec_pretty(&old).unwrap();
        let file = root.join("project.json");
        fs::write(&file, &bytes).unwrap();
        let migrated = store.get(&project.id).unwrap();
        let timeline = migrated.timeline.as_ref().unwrap();
        assert_eq!(migrated.schema_version, 12);
        assert_eq!(timeline.assets.len(), 1);
        assert_eq!(timeline.clips.len(), 1);
        assert_eq!(timeline.clips[0].start_us, 0);
        assert_eq!(timeline.clips[0].duration_us().unwrap(), 120_000_000);
        assert_eq!(
            timeline.settings.frame_rate,
            Rational {
                numerator: 30000,
                denominator: 1001
            }
        );
        assert_eq!(timeline.settings.width, 1080);
        assert_eq!(
            timeline.video_contents[0].settings,
            VideoSettings::from_project(&project)
        );
        assert_eq!(timeline.assets[0].metadata, project.source);
        assert_eq!(
            fs::read(root.join("subtitles/transcript.json")).unwrap(),
            transcript_bytes
        );
        assert_eq!(
            crate::transcript_service::get_transcript(&store, &project.id).unwrap(),
            Some(transcript)
        );
        assert_eq!(
            fs::read(root.join("audio/tts/voice.wav")).unwrap(),
            b"original audio bytes"
        );
        assert_eq!(
            fs::read(root.join("media/source/Việt Nam.mp4")).unwrap(),
            b"original video bytes"
        );
        let backups: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("project.json.pre-v11-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].path()).unwrap(), bytes);
        assert_eq!(store.get(&project.id).unwrap(), migrated);
        assert_eq!(
            store.update_project(&migrated).unwrap().timeline,
            migrated.timeline
        );
    }
    #[test]
    fn timeline_edits_project_to_legacy_fields_and_conflicts_do_not_overwrite_saved_data() {
        let (_dir, store, mut project) = legacy_project();
        let doc = project.timeline.as_mut().unwrap();
        doc.video_contents[0].settings.translation.custom_prompt = "Natural English".into();
        doc.assets[0].relative_path = "media/source/restored.mp4".into();
        doc.assets[0].metadata.video_path = Some(doc.assets[0].relative_path.clone());
        project = store.update_project(&project).unwrap();
        assert_eq!(project.translation.custom_prompt, "Natural English");
        assert_eq!(
            project.source.video_path.as_deref(),
            Some("media/source/restored.mp4")
        );
        assert_eq!(
            project.timeline.as_ref().unwrap().video_contents[0].revision,
            1
        );
        let file = store
            .project_directory(&project.id)
            .unwrap()
            .join("project.json");
        let saved = fs::read(&file).unwrap();
        let mut conflict = project.clone();
        conflict.translation.custom_prompt = "Root change".into();
        conflict.timeline.as_mut().unwrap().video_contents[0]
            .settings
            .translation
            .custom_prompt = "Timeline change".into();
        assert!(store.update_project(&conflict).is_err());
        assert_eq!(fs::read(&file).unwrap(), saved);
        let mut conflict = project.clone();
        conflict.source.original_name = Some("root.mp4".into());
        conflict.timeline.as_mut().unwrap().assets[0]
            .metadata
            .original_name = Some("timeline.mp4".into());
        assert!(store.update_project(&conflict).is_err());
        assert_eq!(fs::read(&file).unwrap(), saved);
        // Placement changes alone never invalidate a video's content revision.
        project.timeline.as_mut().unwrap().clips[0].start_us = 15_000_000;
        let saved = store.update_project(&project).unwrap();
        assert_eq!(
            saved.timeline.as_ref().unwrap().video_contents[0].revision,
            1
        );
    }
    #[test]
    fn moved_split_and_multiple_sources_round_trip_without_overwriting_other_content() {
        let (_dir, store, mut project) = legacy_project();
        let document = project.timeline.as_mut().unwrap();
        document.clips[0].start_us = 60_000_000;
        document.clips[0].source_out_us = 10_000_000;
        let mut second = document.clips[0].clone();
        second.id = Uuid::new_v4().to_string();
        second.start_us = 70_000_000;
        second.source_in_us = 10_000_000;
        second.source_out_us = 20_000_000;
        document.clips.push(second);
        let mut asset_b = document.assets[0].clone();
        asset_b.id = Uuid::new_v4().to_string();
        asset_b.relative_path = "media/source/B.mp4".into();
        asset_b.metadata.video_path = Some(asset_b.relative_path.clone());
        asset_b.metadata.checksum_sha256 = Some("B".into());
        let mut content_b = document.video_contents[0].clone();
        content_b.id = Uuid::new_v4().to_string();
        content_b.asset_id = asset_b.id.clone();
        content_b.settings.translation.target_language = "en".into();
        let mut clip_b = document.clips[0].clone();
        clip_b.id = Uuid::new_v4().to_string();
        clip_b.start_us = 0;
        clip_b.content = ClipContent::Video {
            asset_id: asset_b.id.clone(),
            video_content_id: content_b.id.clone(),
        };
        document.assets.push(asset_b);
        document.video_contents.push(content_b.clone());
        document.clips.push(clip_b);
        let saved = store.update_project(&project).unwrap();
        assert_eq!(store.get(&project.id).unwrap(), saved);
        let mut edited = saved.clone();
        edited.translation.custom_prompt = "Use natural Vietnamese".into();
        let edited = store.update_project(&edited).unwrap();
        let doc = edited.timeline.as_ref().unwrap();
        assert_eq!(doc.clips, saved.timeline.as_ref().unwrap().clips);
        assert_eq!(doc.video_contents[0].revision, 1);
        assert_eq!(doc.video_contents[1], content_b);
        let bytes = fs::read(
            store
                .project_directory(&project.id)
                .unwrap()
                .join("project.json"),
        )
        .unwrap();
        let mut replaced = edited;
        replaced.source.video_path = Some("media/source/replacement.mp4".into());
        replaced.source.checksum_sha256 = Some("replacement".into());
        assert!(store.update_project(&replaced).is_err());
        assert_eq!(
            fs::read(
                store
                    .project_directory(&project.id)
                    .unwrap()
                    .join("project.json")
            )
            .unwrap(),
            bytes
        );
    }
    #[test]
    fn layers_and_linked_speech_round_trip_with_correct_video_ownership() {
        let (_dir, store, mut project) = legacy_project();
        let doc = project.timeline.as_mut().unwrap();
        let parent = doc.clips[0].clone();
        let content_id = doc.video_contents[0].id.clone();
        for (kind, order) in [
            (TrackKind::Audio, 1),
            (TrackKind::Subtitle, 2),
            (TrackKind::Visual, 3),
        ] {
            doc.tracks.push(Track {
                id: Uuid::new_v4().to_string(),
                kind,
                name: format!("Layer {order}"),
                order,
                locked: false,
                enabled: true,
                muted: false,
                solo: false,
            });
        }
        let mut voice = parent.clone();
        voice.id = Uuid::new_v4().to_string();
        voice.track_id = doc.tracks[1].id.clone();
        voice.content = ClipContent::Voice {
            video_content_id: content_id.clone(),
        };
        voice.linked_clip_id = Some(parent.id.clone());
        let mut caption = voice.clone();
        caption.id = Uuid::new_v4().to_string();
        caption.track_id = doc.tracks[2].id.clone();
        caption.content = ClipContent::Subtitles {
            video_content_id: content_id,
        };
        let image = MediaAsset {
            id: Uuid::new_v4().to_string(),
            kind: MediaKind::Image,
            relative_path: "media/overlays/ảnh.png".into(),
            duration_us: None,
            frame_rate: None,
            variable_frame_rate: None,
            metadata: serde_json::from_value(json!({})).unwrap(),
        };
        let mut image_clip = parent.clone();
        image_clip.id = Uuid::new_v4().to_string();
        image_clip.track_id = doc.tracks[3].id.clone();
        image_clip.start_us = 130_000_000;
        image_clip.source_out_us = 5_000_000;
        image_clip.content = ClipContent::Image {
            asset_id: image.id.clone(),
        };
        doc.assets.push(image);
        doc.clips.extend([voice, caption, image_clip]);
        let saved = store.update_project(&project).unwrap();
        assert_eq!(store.get(&project.id).unwrap().timeline, saved.timeline);
        assert_eq!(
            saved.timeline.as_ref().unwrap().duration_us().unwrap(),
            135_000_000
        );
        let mut invalid = saved.clone();
        let doc = invalid.timeline.as_mut().unwrap();
        // An independently edited content of the same media still cannot be linked to this parent.
        let mut content = doc.video_contents[0].clone();
        content.id = Uuid::new_v4().to_string();
        doc.clips[1].content = ClipContent::Voice {
            video_content_id: content.id.clone(),
        };
        doc.video_contents.push(content);
        assert!(store.update_project(&invalid).is_err());
        let mut invalid = saved.clone();
        invalid.timeline.as_mut().unwrap().clips[1].linked_clip_id =
            Some(Uuid::new_v4().to_string());
        assert!(store.update_project(&invalid).is_err());
        assert_eq!(store.get(&project.id).unwrap(), saved);
    }
    #[test]
    fn invalid_references_ranges_tracks_and_paths_are_rejected_before_write() {
        let (_dir, store, project) = legacy_project();
        let base = project.timeline.as_ref().unwrap();
        let mut invalid = Vec::new();
        let mut doc = base.clone();
        doc.clips[0].source_out_us += 1;
        invalid.push(doc);
        let mut doc = base.clone();
        doc.clips[0].rate.denominator = 0;
        invalid.push(doc);
        let mut doc = base.clone();
        doc.clips[0].start_us = MAX_TIME_US;
        invalid.push(doc);
        let mut doc = base.clone();
        doc.clips[0].track_id = Uuid::new_v4().to_string();
        invalid.push(doc);
        let mut doc = base.clone();
        doc.assets[0].relative_path = "media/../outside.mp4".into();
        invalid.push(doc);
        let mut doc = base.clone();
        doc.video_contents[0].asset_id = Uuid::new_v4().to_string();
        invalid.push(doc);
        let mut doc = base.clone();
        doc.tracks[0].kind = TrackKind::Audio;
        invalid.push(doc);
        let mut doc = base.clone();
        let mut clip = doc.clips[0].clone();
        clip.id = Uuid::new_v4().to_string();
        doc.clips.push(clip);
        invalid.push(doc);
        let mut doc = base.clone();
        doc.assets[0].id = doc.clips[0].id.clone();
        invalid.push(doc);
        let mut doc = base.clone();
        doc.clips[0].transform.crop.width = 2.0;
        invalid.push(doc);
        let file = store
            .project_directory(&project.id)
            .unwrap()
            .join("project.json");
        let original = fs::read(&file).unwrap();
        for doc in invalid {
            let mut changed = project.clone();
            changed.timeline = Some(doc);
            assert!(store.update_project(&changed).is_err());
            assert_eq!(fs::read(&file).unwrap(), original);
        }
    }
    #[test]
    fn incomplete_legacy_source_empty_projects_and_recovery_remain_editable() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let empty = store.create("AI studio").unwrap();
        assert_eq!(empty.timeline.as_ref().unwrap().duration_us().unwrap(), 0);
        let mut old = serde_json::to_value(&empty).unwrap();
        old["schemaVersion"] = json!(11);
        old.as_object_mut().unwrap().remove("timeline");
        old["source"]["videoPath"] = json!("media/source/missing.mp4");
        let file = store
            .project_directory(&empty.id)
            .unwrap()
            .join("project.json");
        fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
        let mut migrated = store.get(&empty.id).unwrap();
        assert_eq!(migrated.timeline.as_ref().unwrap().assets.len(), 1);
        assert!(migrated.timeline.as_ref().unwrap().clips.is_empty());
        let asset_id = migrated.timeline.as_ref().unwrap().assets[0].id.clone();
        migrated.source.duration = Some(10.0);
        migrated = store.update_project(&migrated).unwrap();
        assert_eq!(migrated.timeline.as_ref().unwrap().assets[0].id, asset_id);
        assert_eq!(migrated.timeline.as_ref().unwrap().clips.len(), 1);
        store.rename(&empty.id, "renamed").unwrap();
        fs::write(&file, b"interrupted write").unwrap();
        let recovered = store.get(&empty.id).unwrap();
        assert_eq!(recovered.timeline, migrated.timeline);
        let mut future_timeline = serde_json::to_value(&recovered).unwrap();
        future_timeline["timeline"] = json!({"schemaVersion": 99, "unknownPayload": "preserve"});
        let bytes = serde_json::to_vec(&future_timeline).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert!(store.get(&empty.id).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        let mut future = serde_json::to_value(&recovered).unwrap();
        future["schemaVersion"] = json!(99);
        future["timeline"] = json!("future incompatible shape");
        let bytes = serde_json::to_vec(&future).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert!(store.get(&empty.id).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
    }
    #[test]
    fn migration_failure_never_replaces_original_document() {
        let (_dir, store, project) = legacy_project();
        let file = store
            .project_directory(&project.id)
            .unwrap()
            .join("project.json");
        let mut old = serde_json::to_value(&project).unwrap();
        old["schemaVersion"] = json!(11);
        old.as_object_mut().unwrap().remove("timeline");
        old["source"]["duration"] = json!(MAX_TIME_US as f64);
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert!(store.get(&project.id).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
    }
}
