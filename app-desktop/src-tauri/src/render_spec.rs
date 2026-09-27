use crate::project_service::{BlurRegion, BlurRegionRole, Project, ProjectLogo, SubtitleSettings};
use crate::transcript_service::Transcript;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurRenderSpec {
    pub id: String,
    pub role: BlurRegionRole,
    pub rect: PixelRect,
    pub enabled: bool,
    pub strength_px: u32,
    pub edge_blend: u8,
    pub pixel_noise: u8,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoRenderSpec {
    pub id: String,
    pub asset_path: String,
    pub rect: PixelRect,
    pub enabled: bool,
    pub opacity: f64,
    pub maintain_aspect_ratio: bool,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default = "default_true")]
    pub apply_blur: bool,
    #[serde(default = "default_true")]
    pub apply_logos: bool,
    #[serde(default = "default_true")]
    pub burn_subtitles: bool,
    #[serde(default)]
    pub enable_subtitle_blur_override: bool,
    #[serde(default = "default_true")]
    pub include_original_audio: bool,
    #[serde(default)]
    pub include_dubbing: bool,
    #[serde(default)]
    pub ducking: bool,
    #[serde(default)]
    pub output_directory: Option<String>,
    #[serde(default = "default_output_file_name")]
    pub output_file_name: String,
    #[serde(default = "default_one")]
    pub original_volume: f64,
    #[serde(default = "default_one")]
    pub dubbed_volume: f64,
    #[serde(default = "default_ducking_amount")]
    pub ducking_amount: f64,
    #[serde(default)]
    pub dialogue_reduction: bool,
    #[serde(default = "default_dialogue_reduction_amount")]
    pub dialogue_reduction_amount: f64,
}

fn default_quality() -> String {
    "balanced".to_string()
}
fn default_true() -> bool {
    true
}
fn default_one() -> f64 {
    1.0
}
fn default_ducking_amount() -> f64 {
    0.75
}
fn default_dialogue_reduction_amount() -> f64 { 0.65 }
fn default_output_file_name() -> String {
    chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            quality: default_quality(),
            apply_blur: true,
            apply_logos: true,
            burn_subtitles: true,
            enable_subtitle_blur_override: false,
            include_original_audio: true,
            include_dubbing: false,
            ducking: false,
            output_directory: None,
            output_file_name: default_output_file_name(),
            original_volume: 1.0,
            dubbed_volume: 1.0,
            ducking_amount: 0.75,
            dialogue_reduction: false,
            dialogue_reduction_amount: default_dialogue_reduction_amount(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleLayout {
    pub text: String,
    pub lines: Vec<String>,
    pub font_size: u16,
    pub x: u32,
    pub y: u32,
    pub alignment: u8,
    pub fits: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleLayoutWarning {
    pub segment_id: String,
    pub start: f64,
    pub end: f64,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleEvent {
    pub segment_id: String,
    pub start: f64,
    pub end: f64,
    pub layout: SubtitleLayout,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleRenderSpec {
    pub enabled: bool,
    pub region_id: Option<String>,
    pub region: Option<PixelRect>,
    pub font_name: String,
    pub bold: bool,
    pub primary_colour: String,
    pub outline_colour: String,
    pub outline_width: u16,
    pub outline_enabled: bool,
    pub alignment: String,
    pub vertical_alignment: String,
    pub padding_x: u8,
    pub padding_y: u8,
    pub configured_font_size: u16,
    pub auto_fit: bool,
    pub min_font_size: u16,
    pub events: Vec<SubtitleEvent>,
    pub warnings: Vec<SubtitleLayoutWarning>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderSpecification {
    pub source_video: String,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub duration: f64,
    pub blur_regions: Vec<BlurRenderSpec>,
    pub logos: Vec<LogoRenderSpec>,
    pub subtitle: SubtitleRenderSpec,
    pub audio: RenderAudioConfig,
    pub quality: String,
    pub output: RenderOutputConfig,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderAudioConfig {
    pub include_original: bool,
    pub original_volume: f64,
    pub include_dubbing: bool,
    pub dubbed_volume: f64,
    pub ducking: bool,
    pub ducking_amount: f64,
    pub dialogue_reduction: bool,
    pub dialogue_reduction_amount: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutputConfig {
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub pixel_format: String,
}

pub fn normalized_rect_to_pixels(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    video_width: u32,
    video_height: u32,
) -> Result<PixelRect, String> {
    if video_width == 0 || video_height == 0 {
        return Err("Kích thước video không hợp lệ.".into());
    }
    if ![x, y, width, height].into_iter().all(f64::is_finite)
        || x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > 1.0 + 1e-9
        || y + height > 1.0 + 1e-9
    {
        return Err("Hình chữ nhật overlay vượt ra ngoài video.".into());
    }
    let left = (x * video_width as f64)
        .floor()
        .clamp(0.0, video_width as f64) as u32;
    let top = (y * video_height as f64)
        .floor()
        .clamp(0.0, video_height as f64) as u32;
    let right = ((x + width) * video_width as f64)
        .ceil()
        .clamp(0.0, video_width as f64) as u32;
    let bottom = ((y + height) * video_height as f64)
        .ceil()
        .clamp(0.0, video_height as f64) as u32;
    Ok(PixelRect {
        x: left,
        y: top,
        width: right.saturating_sub(left).max(1),
        height: bottom.saturating_sub(top).max(1),
    })
}

pub fn to_ass_color(color: &str, opacity: f64) -> Result<String, String> {
    if color.len() != 7
        || !color.starts_with('#')
        || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        || !opacity.is_finite()
        || !(0.0..=1.0).contains(&opacity)
    {
        return Err("Không thể chuyển màu sang định dạng ASS.".into());
    }
    let red = u8::from_str_radix(&color[1..3], 16).map_err(|_| "Mã màu ASS không hợp lệ.")?;
    let green = u8::from_str_radix(&color[3..5], 16).map_err(|_| "Mã màu ASS không hợp lệ.")?;
    let blue = u8::from_str_radix(&color[5..7], 16).map_err(|_| "Mã màu ASS không hợp lệ.")?;
    let alpha = ((1.0 - opacity) * 255.0).round() as u8;
    Ok(format!("&H{alpha:02X}{blue:02X}{green:02X}{red:02X}"))
}

pub fn build_render_specification(
    project: &Project,
    transcript: &Transcript,
    duration: f64,
    options: &ExportOptions,
) -> Result<RenderSpecification, String> {
    let width = project
        .source
        .width
        .ok_or("Video chưa có metadata chiều rộng.")?;
    let height = project
        .source
        .height
        .ok_or("Video chưa có metadata chiều cao.")?;
    if width == 0 || height == 0 || !duration.is_finite() || duration <= 0.0 {
        return Err("Metadata video không hợp lệ.".into());
    }
    if transcript.project_id != project.id {
        return Err("Transcript không thuộc dự án hiện tại.".into());
    }
    let source_video = project
        .source
        .video_path
        .clone()
        .ok_or("Dự án chưa có đường dẫn video nguồn.")?;
    if !["balanced", "high", "fast"].contains(&options.quality.as_str()) {
        return Err("Chất lượng export không hợp lệ.".into());
    }
    let blur_regions = project
        .blur_regions
        .iter()
        .map(|region| blur_spec(region, width, height, options))
        .collect::<Result<Vec<_>, _>>()?;
    let logos = project
        .logos
        .iter()
        .map(|logo| logo_spec(logo, width, height))
        .collect::<Result<Vec<_>, _>>()?;
    let subtitle = subtitle_spec(
        &project.subtitle,
        &project.blur_regions,
        transcript,
        width,
        height,
        duration,
        options,
    )?;
    Ok(RenderSpecification {
        source_video,
        width,
        height,
        fps: project
            .source
            .fps
            .filter(|fps| fps.is_finite() && *fps > 0.0),
        duration,
        blur_regions,
        logos,
        subtitle,
        audio: RenderAudioConfig {
            include_original: options.include_original_audio
                && !project.source.audio_streams.is_empty(),
            original_volume: options.original_volume,
            include_dubbing: options.include_dubbing,
            dubbed_volume: options.dubbed_volume,
            ducking: options.ducking,
            ducking_amount: options.ducking_amount,
            dialogue_reduction: options.dialogue_reduction,
            dialogue_reduction_amount: options.dialogue_reduction_amount,
        },
        quality: options.quality.clone(),
        output: RenderOutputConfig {
            container: "mp4".into(),
            video_codec: "libx264".into(),
            audio_codec: "aac".into(),
            pixel_format: "yuv420p".into(),
        },
    })
}

fn blur_spec(
    region: &BlurRegion,
    width: u32,
    height: u32,
    options: &ExportOptions,
) -> Result<BlurRenderSpec, String> {
    Ok(BlurRenderSpec {
        id: region.id.clone(),
        role: region.role,
        rect: normalized_rect_to_pixels(
            region.x,
            region.y,
            region.width,
            region.height,
            width,
            height,
        )?,
        enabled: (options.apply_blur && region.enabled)
            || (options.enable_subtitle_blur_override && region.role == BlurRegionRole::Subtitle),
        strength_px: ((region.strength / 4.0).round() as u32).clamp(1, 32),
        edge_blend: region.edge_blend,
        pixel_noise: region.pixel_noise,
        start_ms: region.start_ms,
        end_ms: region.end_ms,
    })
}

fn logo_spec(logo: &ProjectLogo, width: u32, height: u32) -> Result<LogoRenderSpec, String> {
    if !logo.opacity.is_finite() || !(0.0..=1.0).contains(&logo.opacity) {
        return Err("Độ mờ logo không hợp lệ.".into());
    }
    Ok(LogoRenderSpec {
        id: logo.id.clone(),
        asset_path: logo.relative_path.clone(),
        rect: normalized_rect_to_pixels(logo.x, logo.y, logo.width, logo.height, width, height)?,
        enabled: logo.enabled,
        opacity: logo.opacity,
        maintain_aspect_ratio: logo.maintain_aspect_ratio,
        start_ms: 0,
        end_ms: None,
    })
}

fn subtitle_spec(
    settings: &SubtitleSettings,
    regions: &[BlurRegion],
    transcript: &Transcript,
    width: u32,
    height: u32,
    duration: f64,
    options: &ExportOptions,
) -> Result<SubtitleRenderSpec, String> {
    let enabled = settings.enabled && options.burn_subtitles;
    let selected: Vec<_> = regions
        .iter()
        .filter(|region| region.role == BlurRegionRole::Subtitle)
        .collect();
    if enabled && selected.len() != 1 {
        return Err("Phụ đề dịch đang bật nhưng chưa có một vùng blur được chọn làm Subtitle Blur. Hãy chọn vùng trong Properties.".into());
    }
    let region = selected
        .first()
        .map(|item| {
            normalized_rect_to_pixels(item.x, item.y, item.width, item.height, width, height)
        })
        .transpose()?;
    if enabled
        && selected.first().is_some_and(|item| !item.enabled)
        && !options.enable_subtitle_blur_override
    {
        return Err(
            "Subtitle Blur đang tắt. Bật vùng blur hoặc chọn “Bật blur cho lần export này”.".into(),
        );
    }
    let mut events = Vec::new();
    let mut warnings = Vec::new();
    if enabled {
        let rect = region.ok_or("Không thể xác định vùng subtitle blur.")?;
        let mut seen = std::collections::HashSet::new();
        for segment in &transcript.segments {
            if !segment.translated.trim().is_empty() {
                if !seen.insert(&segment.id) {
                    return Err(format!("Segment ID bị trùng: {}", segment.id));
                }
                if !segment.start.is_finite()
                    || !segment.end.is_finite()
                    || segment.start < 0.0
                    || segment.start >= segment.end
                    || segment.end > duration + 0.05
                {
                    return Err(format!(
                        "Timestamp không hợp lệ cho segment {}.",
                        segment.id
                    ));
                }
                let layout = layout_subtitle(&segment.translated, rect, settings)?;
                if !layout.fits {
                    warnings.push(SubtitleLayoutWarning { segment_id: segment.id.clone(), start: segment.start, end: segment.end, message: "Nội dung không vừa vùng blur ở cỡ chữ tối thiểu; không thể xuất an toàn nếu không chỉnh vùng hoặc nội dung.".into() });
                }
                events.push(SubtitleEvent {
                    segment_id: segment.id.clone(),
                    start: segment.start,
                    end: segment.end,
                    layout,
                });
            }
        }
        if events.is_empty() {
            return Err("Chưa có bản dịch nào để chèn phụ đề vào vùng blur.".into());
        }
    }
    Ok(SubtitleRenderSpec {
        enabled,
        region_id: selected.first().map(|item| item.id.clone()),
        region,
        font_name: safe_font_name(&settings.font_family),
        bold: settings.font_weight >= 600,
        primary_colour: to_ass_color(&settings.text_color, 1.0)?,
        outline_colour: to_ass_color(&settings.outline_color, 1.0)?,
        outline_width: if settings.outline {
            settings.outline_width
        } else {
            0
        },
        outline_enabled: settings.outline,
        alignment: settings.alignment.clone(),
        vertical_alignment: settings.vertical_alignment.clone(),
        padding_x: settings.padding_x.min(30),
        padding_y: settings.padding_y.min(40),
        configured_font_size: settings.font_size.max(1),
        auto_fit: settings.auto_fit,
        min_font_size: settings.min_font_size.max(1).min(settings.font_size.max(1)),
        events,
        warnings,
    })
}

pub fn layout_subtitle(
    text: &str,
    region: PixelRect,
    settings: &SubtitleSettings,
) -> Result<SubtitleLayout, String> {
    if text.trim().is_empty() {
        return Err("Không thể bố trí phụ đề trống.".into());
    }
    let pad_x = (region.width as f64 * settings.padding_x.min(30) as f64 / 100.0).round() as u32;
    let pad_y = (region.height as f64 * settings.padding_y.min(40) as f64 / 100.0).round() as u32;
    let inner = PixelRect {
        x: region.x.saturating_add(pad_x),
        y: region.y.saturating_add(pad_y),
        width: region.width.saturating_sub(pad_x.saturating_mul(2)).max(1),
        height: region.height.saturating_sub(pad_y.saturating_mul(2)).max(1),
    };
    let configured = settings.font_size.max(1);
    let minimum = if settings.auto_fit {
        settings.min_font_size.max(1).min(configured)
    } else {
        configured
    };
    let mut best = None;
    for font_size in (minimum..=configured).rev() {
        let lines = wrap_text(text, inner.width as f64 / font_size as f64);
        let line_height = (font_size as f64 * settings.line_height.max(0.8)).ceil();
        let height_fits = line_height * lines.len() as f64 <= inner.height as f64;
        let width_fits = lines
            .iter()
            .all(|line| text_width_units(line) * font_size as f64 <= inner.width as f64 + 0.5);
        best = Some((font_size, lines, height_fits && width_fits));
        if height_fits && width_fits {
            break;
        }
        if !settings.auto_fit {
            break;
        }
    }
    let (font_size, lines, fits) = best.unwrap();
    let horizontal = match settings.alignment.as_str() {
        "left" => (inner.x, 4),
        "right" => (inner.x + inner.width, 6),
        _ => (inner.x + inner.width / 2, 5),
    };
    let vertical = match settings.vertical_alignment.as_str() {
        "top" => 6,
        "bottom" => -6,
        _ => 0,
    };
    let y = if vertical == 6 {
        inner.y
    } else if vertical == -6 {
        inner.y + inner.height
    } else {
        inner.y + inner.height / 2
    };
    let alignment = match (vertical, horizontal.1) {
        (6, 4) => 7,
        (6, 5) => 8,
        (6, 6) => 9,
        (-6, 4) => 1,
        (-6, 5) => 2,
        (-6, 6) => 3,
        (_, 4) => 4,
        (_, 5) => 5,
        _ => 6,
    };
    Ok(SubtitleLayout {
        text: lines.join("\n"),
        lines,
        font_size,
        x: horizontal.0,
        y,
        alignment,
        fits,
    })
}

fn wrap_text(text: &str, max_units: f64) -> Vec<String> {
    let mut result = Vec::new();
    for manual_line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if manual_line.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in manual_line.split_whitespace() {
            let separator = if current.is_empty() { "" } else { " " };
            let candidate = format!("{current}{separator}{word}");
            if text_width_units(&candidate) <= max_units || current.is_empty() {
                if text_width_units(word) > max_units && current.is_empty() {
                    let mut chunk = String::new();
                    for character in word.chars() {
                        let mut next = chunk.clone();
                        next.push(character);
                        if !chunk.is_empty() && text_width_units(&next) > max_units {
                            result.push(chunk);
                            chunk = String::new();
                        }
                        chunk.push(character);
                    }
                    current = chunk;
                } else {
                    current = candidate;
                }
            } else {
                result.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        result.push(current);
    }
    result
}

fn text_width_units(text: &str) -> f64 {
    text.chars()
        .map(|character| match character {
            '\u{0300}'..='\u{036f}' => 0.0,
            ' ' | '\t' => 0.32,
            'i' | 'l' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' | '`' => 0.30,
            'm' | 'w' | 'M' | 'W' | '@' | '%' => 0.88,
            c if c.is_ascii() => 0.56,
            c if c.is_whitespace() => 0.32,
            c if is_east_asian_wide(c) => 1.0,
            c if c.is_alphabetic() => 0.62,
            _ => 0.72,
        })
        .sum()
}

fn is_east_asian_wide(character: char) -> bool {
    matches!(
        character as u32,
        0x1100..=0x11ff
            | 0x2e80..=0xa4cf
            | 0xac00..=0xd7af
            | 0xf900..=0xfaff
            | 0xfe10..=0xfe6f
            | 0xff00..=0xffef
            | 0x1f300..=0x1faff
    )
}

fn safe_font_name(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ',' | '\\' | '{' | '}'))
        .take(80)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn generate_ass(spec: &RenderSpecification) -> Result<String, String> {
    if !spec.subtitle.enabled || spec.subtitle.events.is_empty() {
        return Err("Không có phụ đề đã dịch để render.".into());
    }
    let mut ass = format!("[Script Info]\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\nScaledBorderAndShadow: yes\nWrapStyle: 2\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Localized,{},{} ,{}, {},{},&H00000000,{},0,1,{},0,5,0,0,0,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n", spec.width, spec.height, spec.subtitle.font_name, spec.subtitle.configured_font_size, spec.subtitle.primary_colour, spec.subtitle.primary_colour, spec.subtitle.outline_colour, if spec.subtitle.bold { -1 } else { 0 }, spec.subtitle.outline_width);
    for event in &spec.subtitle.events {
        if !event.layout.fits {
            continue;
        }
        let escaped = escape_ass_text(&event.layout.text);
        let tags = format!(
            "{{\\an{}\\pos({}, {})\\fs{}\\bord{}\\1c{}\\3c{}}}",
            event.layout.alignment,
            event.layout.x,
            event.layout.y,
            event.layout.font_size,
            spec.subtitle.outline_width,
            spec.subtitle.primary_colour,
            spec.subtitle.outline_colour
        );
        ass.push_str(&format!(
            "Dialogue: 0,{},{},Localized,,0,0,0,,{}{}\n",
            ass_timestamp(event.start),
            ass_timestamp(event.end),
            tags,
            escaped
        ));
    }
    Ok(ass)
}

fn escape_ass_text(text: &str) -> String {
    let mut output = String::new();
    for character in text.chars() {
        match character {
            '\n' | '\r' => output.push_str("\\N"),
            '\\' => output.push_str("\\\\"),
            '{' => output.push_str("\\{"),
            '}' => output.push_str("\\}"),
            _ => output.push(character),
        }
    }
    output
}

fn ass_timestamp(value: f64) -> String {
    let centiseconds = (value.max(0.0) * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        centiseconds / 360000,
        (centiseconds / 6000) % 60,
        (centiseconds / 100) % 60,
        centiseconds % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::{
        DubbingSettings, ProjectSource, TranscriptSourceType, TranslationSettings,
    };
    use crate::transcript_service::{TranscriptSegment, TranscriptTranslationStatus};

    fn segment(text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: "one".into(),
            start: 0.5,
            end: 2.0,
            source: "hello".into(),
            translated: text.into(),
            source_revision: 0,
            translated_revision: Some(0),
            translation_meta: None,
            speaker_id: None,
            source_type: TranscriptSourceType::Asr,
            translation_status: TranscriptTranslationStatus::Translated,
            tts: None,
        }
    }
    fn project(width: u32, height: u32, rect: (f64, f64, f64, f64)) -> Project {
        Project {
            schema_version: 3,
            id: "p".into(),
            name: "Test".into(),
            created_at: String::new(),
            updated_at: String::new(),
            source: ProjectSource {
                video_path: Some("media/source/v.mp4".into()),
                original_name: None,
                duration: Some(10.0),
                width: Some(width),
                height: Some(height),
                fps: None,
                video_codec: None,
                video_profile: None,
                video_pixel_format: None,
                audio_codec: None,
                file_size: None,
                checksum_sha256: None,
                audio_streams: vec![],
                subtitle_tracks: vec![],
            },
            transcript: None,
            translation: TranslationSettings {
                source_language: "en".into(),
                target_language: "vi".into(),
                custom_prompt: String::new(),
                segments: vec![],
            },
            subtitle: SubtitleSettings::default(),
            dubbing: DubbingSettings::default(),
            blur_regions: vec![BlurRegion {
                id: "sub".into(),
                x: rect.0,
                y: rect.1,
                width: rect.2,
                height: rect.3,
                start_ms: 0,
                end_ms: None,
                enabled: true,
                strength: 24.0,
                role: BlurRegionRole::Subtitle,
                edge_blend: 12,
                pixel_noise: 10,
            }],
            logos: vec![],
            transcript_settings: crate::project_service::TranscriptSettings::default(),
        }
    }
    fn transcript(text: &str) -> Transcript {
        Transcript {
            schema_version: 1,
            project_id: "p".into(),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: None,
            video_checksum: None,
            segments: vec![segment(text)],
        }
    }

    #[test]
    fn normalized_rectangles_resolve_in_landscape_portrait_and_square() {
        assert_eq!(
            normalized_rect_to_pixels(0.08, 0.78, 0.84, 0.14, 1080, 1920).unwrap(),
            PixelRect {
                x: 86,
                y: 1497,
                width: 908,
                height: 270
            }
        );
        assert_eq!(
            normalized_rect_to_pixels(0.25, 0.25, 0.5, 0.5, 1920, 1080).unwrap(),
            PixelRect {
                x: 480,
                y: 270,
                width: 960,
                height: 540
            }
        );
        assert_eq!(
            normalized_rect_to_pixels(0.1, 0.1, 0.8, 0.8, 1000, 1000).unwrap(),
            PixelRect {
                x: 100,
                y: 100,
                width: 800,
                height: 800
            }
        );
        assert!(normalized_rect_to_pixels(0.9, 0.0, 0.2, 0.1, 100, 100).is_err());
    }
    #[test]
    fn subtitle_region_is_mandatory_and_generic_blur_never_substitutes() {
        let mut p = project(1920, 1080, (0.1, 0.8, 0.8, 0.15));
        let tr = transcript("Xin chào");
        assert!(build_render_specification(&p, &tr, 10.0, &ExportOptions::default()).is_ok());
        p.blur_regions[0].role = BlurRegionRole::Generic;
        assert!(
            build_render_specification(&p, &tr, 10.0, &ExportOptions::default())
                .unwrap_err()
                .contains("Subtitle Blur")
        );
    }
    #[test]
    fn subtitle_wrap_preserves_manual_lines_and_vietnamese_and_fits_portrait() {
        let settings = SubtitleSettings::default();
        let rect = normalized_rect_to_pixels(0.08, 0.78, 0.84, 0.14, 1080, 1920).unwrap();
        let layout = layout_subtitle(
            "Xin chào mọi người\nĐây là phụ đề tiếng Việt dài",
            rect,
            &settings,
        )
        .unwrap();
        assert!(layout.lines.len() >= 2);
        assert!(layout.text.contains('\n'));
        assert!(layout.font_size <= settings.font_size);
    }

    #[test]
    fn vietnamese_text_uses_realistic_width_and_keeps_requested_font_size() {
        let mut settings = SubtitleSettings::default();
        settings.font_size = 52;
        settings.min_font_size = 28;
        let region = PixelRect {
            x: 80,
            y: 1400,
            width: 920,
            height: 280,
        };
        let text = "Đây là phụ đề tiếng Việt được dịch tự động, cần hiển thị rõ ràng.";
        let layout = layout_subtitle(text, region, &settings).unwrap();
        assert!(layout.fits);
        assert!(
            layout.font_size >= 42,
            "unexpected shrink to {} px",
            layout.font_size
        );

        settings.font_size = 68;
        settings.min_font_size = 68;
        let larger = layout_subtitle("Đây là phụ đề tiếng Việt.", region, &settings).unwrap();
        assert_eq!(larger.font_size, 68);
        assert!(larger.fits);
    }
    #[test]
    fn tiny_region_reports_overflow_without_truncation() {
        let mut settings = SubtitleSettings::default();
        settings.font_size = 42;
        settings.min_font_size = 42;
        settings.padding_x = 5;
        settings.padding_y = 5;
        let layout = layout_subtitle(
            "một câu cực kỳ dài không thể nào vừa vào một vùng bé",
            PixelRect {
                x: 0,
                y: 0,
                width: 32,
                height: 14,
            },
            &settings,
        )
        .unwrap();
        assert!(!layout.fits);
        assert!(layout.text.contains("một"));
    }
    #[test]
    fn ass_output_has_region_anchor_and_unicode_and_escapes_markup() {
        let p = project(1920, 1080, (0.1, 0.75, 0.8, 0.2));
        let tr = transcript("Xin chào {mọi}\\người\ndòng hai");
        let spec = build_render_specification(&p, &tr, 10.0, &ExportOptions::default()).unwrap();
        let ass = generate_ass(&spec).unwrap();
        assert!(ass.contains("\\pos("));
        assert!(ass.contains("Xin chào "));
        assert!(ass.contains("\\{"));
        assert!(ass.contains("\\N"));
    }

    #[test]
    fn configured_font_size_is_written_to_the_ass_event() {
        let mut p = project(1920, 1080, (0.1, 0.75, 0.8, 0.2));
        p.subtitle.font_size = 76;
        p.subtitle.min_font_size = 76;
        p.subtitle.auto_fit = false;
        let spec = build_render_specification(
            &p,
            &transcript("Xin chào"),
            10.0,
            &ExportOptions::default(),
        )
        .unwrap();
        let ass = generate_ass(&spec).unwrap();
        assert!(
            ass.contains("\\fs76"),
            "configured 76px was not emitted: {ass}"
        );
    }
    #[test]
    fn subtitle_disabled_does_not_require_region_but_blur_model_remains() {
        let mut p = project(1080, 1920, (0.08, 0.78, 0.84, 0.14));
        p.blur_regions[0].role = BlurRegionRole::Generic;
        p.subtitle.enabled = false;
        let mut options = ExportOptions::default();
        options.burn_subtitles = false;
        let spec = build_render_specification(&p, &transcript("test"), 10.0, &options).unwrap();
        assert!(!spec.subtitle.enabled);
        assert_eq!(spec.blur_regions.len(), 1);
    }
}
