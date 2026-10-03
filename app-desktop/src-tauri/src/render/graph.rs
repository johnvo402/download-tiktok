use crate::project_service::BlurEffect;
use crate::render_spec::{
    BlurRenderSpec, LogoRenderSpec, PixelRect, RenderSpecification, SubtitleRenderSpec,
    TextOverlayRenderSpec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    CleanPatch,
    Blur,
    Subtitle,
    Text,
    Logo,
}
pub const ORDER: [Stage; 5] = [
    Stage::CleanPatch,
    Stage::Blur,
    Stage::Subtitle,
    Stage::Text,
    Stage::Logo,
];

/// Same resolved video pixel rectangles as legacy export. Assets are resolved by
/// the caller before entering the renderer, so imported projects cannot read arbitrary files.
#[derive(Clone)]
pub struct RenderScene {
    pub width: u32,
    pub height: u32,
    /// Pixel units remain relative to the design/source resolution. Export uses 1.
    pub pixel_scale: f32,
    pub blurs: Vec<BlurRenderSpec>,
    pub logos: Vec<LogoRenderSpec>,
    pub text: Vec<TextOverlayRenderSpec>,
    pub subtitle: SubtitleRenderSpec,
}
impl RenderScene {
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixel_scale: 1.0,
            blurs: Vec::new(),
            logos: Vec::new(),
            text: Vec::new(),
            subtitle: SubtitleRenderSpec {
                enabled: false,
                region_id: None,
                region: None,
                font_name: "Segoe UI".into(),
                bold: false,
                font_weight: 400,
                line_height: 1.2,
                background_color: "#000000".into(),
                background_opacity: 0.0,
                primary_colour: "&H00FFFFFF".into(),
                outline_colour: "&H00000000".into(),
                outline_width: 2,
                outline_enabled: true,
                alignment: "center".into(),
                vertical_alignment: "middle".into(),
                padding_x: 4,
                padding_y: 10,
                configured_font_size: 30,
                auto_fit: true,
                min_font_size: 20,
                events: Vec::new(),
                warnings: Vec::new(),
            },
        }
    }
    pub fn from_spec(spec: &RenderSpecification) -> Self {
        Self {
            width: spec.width,
            height: spec.height,
            pixel_scale: 1.0,
            blurs: spec
                .blur_regions
                .iter()
                .filter(|r| r.enabled && r.effect == BlurEffect::Blur)
                .cloned()
                .collect(),
            logos: spec.logos.iter().filter(|r| r.enabled).cloned().collect(),
            text: spec
                .text_overlays
                .iter()
                .filter(|r| r.enabled && !r.text.trim().is_empty())
                .cloned()
                .collect(),
            subtitle: spec.subtitle.clone(),
        }
    }
    /// Target adaptation of the canonical scene, shared with the existing passes.
    /// It never changes project data, timings, text content or normalized placement.
    pub fn scaled(&self, scale: f32) -> Self {
        if scale == 1.0 {
            return self.clone();
        }
        let scale = scale.clamp(0.25, 1.0);
        let mut scene = self.clone();
        scene.width = (self.width as f32 * scale).round().max(1.0) as u32;
        scene.height = (self.height as f32 * scale).round().max(1.0) as u32;
        scene.pixel_scale = self.pixel_scale * scale;
        let rect = |r: PixelRect| {
            let x = (r.x as f32 * scale).round() as u32;
            let y = (r.y as f32 * scale).round() as u32;
            PixelRect {
                x: x.min(scene.width - 1),
                y: y.min(scene.height - 1),
                width: ((r.width as f32 * scale).round().max(1.0) as u32)
                    .min(scene.width - x.min(scene.width - 1)),
                height: ((r.height as f32 * scale).round().max(1.0) as u32)
                    .min(scene.height - y.min(scene.height - 1)),
            }
        };
        let font = |n: u16| (n as f32 * scale).round().max(1.0) as u16;
        for r in &mut scene.blurs {
            r.rect = rect(r.rect);
        }
        for r in &mut scene.logos {
            r.rect = rect(r.rect);
        }
        for r in &mut scene.text {
            r.rect = rect(r.rect);
            r.font_size = font(r.font_size);
        }
        scene.subtitle.region = scene.subtitle.region.map(rect);
        scene.subtitle.configured_font_size = font(scene.subtitle.configured_font_size);
        scene.subtitle.min_font_size = font(scene.subtitle.min_font_size);
        scene.subtitle.outline_width = (scene.subtitle.outline_width as f32 * scale).round() as u16;
        for event in &mut scene.subtitle.events {
            event.layout.font_size = font(event.layout.font_size);
            event.layout.x = (event.layout.x as f32 * scale).round() as u32;
            event.layout.y = (event.layout.y as f32 * scale).round() as u32;
        }
        scene
    }
}
pub fn active(pts: f64, start_ms: u64, end_ms: Option<u64>) -> bool {
    pts.is_finite()
        && pts >= start_ms as f64 / 1000.0
        && end_ms.is_none_or(|end| pts < end as f64 / 1000.0)
}
/// Source-video pixel sigma shared with CSS approximation and legacy gblur.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisualContract {
    blur_sigma_scale: f32,
    full_strength_threshold: u32,
    maximum_edge_blend: u8,
    feather_fraction: f32,
}
fn contract() -> &'static VisualContract {
    static CONTRACT: std::sync::OnceLock<VisualContract> = std::sync::OnceLock::new();
    CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!("../../../src/shared/visual-effects.json"))
            .expect("checked visual contract")
    })
}
pub fn blur_sigma(strength: u32) -> f32 {
    strength.clamp(1, 100) as f32 * contract().blur_sigma_scale
}
pub fn feather_px(rect: PixelRect, strength: u32, edge_blend: u8) -> f32 {
    if strength >= contract().full_strength_threshold {
        0.0
    } else {
        rect.width.min(rect.height) as f32
            * (edge_blend.min(contract().maximum_edge_blend) as f32
                / contract().maximum_edge_blend as f32)
            * contract().feather_fraction
    }
}
pub fn fitted_image_rect(rect: PixelRect, width: u32, height: u32, keep_aspect: bool) -> PixelRect {
    if !keep_aspect {
        return rect;
    }
    let ratio =
        (rect.width as f64 / width.max(1) as f64).min(rect.height as f64 / height.max(1) as f64);
    let w = (width as f64 * ratio).round().max(1.0) as u32;
    let h = (height as f64 * ratio).round().max(1.0) as u32;
    PixelRect {
        x: rect.x + rect.width.saturating_sub(w) / 2,
        y: rect.y + rect.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_and_timing() {
        assert_eq!(
            ORDER,
            [
                Stage::CleanPatch,
                Stage::Blur,
                Stage::Subtitle,
                Stage::Text,
                Stage::Logo
            ]
        );
        assert!(active(1.0, 1000, Some(2000)));
        assert!(!active(2.0, 1000, Some(2000)));
        assert!(!active(f64::NAN, 0, None));
        assert!(active(999.0, 0, None));
    }
    #[test]
    fn blur_mapping_and_aspect() {
        let rect = PixelRect {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert_eq!(blur_sigma(100), 200.0);
        assert_eq!(feather_px(rect, 100, 50), 0.0);
        assert_eq!(feather_px(rect, 30, 50), 5.0);
        assert_eq!(
            fitted_image_rect(rect, 100, 100, true),
            PixelRect {
                x: 35,
                y: 20,
                width: 50,
                height: 50
            }
        );
    }
}
