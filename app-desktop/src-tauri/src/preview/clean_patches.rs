//! Completed AI spans share source PTS and compositing with export. No AI work here.
use crate::{
    render::renderer::Renderer, render_spec::PixelRect, text_removal_cache::PatchReader,
    text_removal_service::AiTextRemovalPatch,
};

pub struct CleanPatches {
    readers: Vec<PatchReader>,
    patches: Vec<AiTextRemovalPatch>,
    layers: Vec<usize>,
    rects: Vec<PixelRect>,
}
impl CleanPatches {
    pub fn new(
        renderer: &mut Renderer,
        patches: Vec<AiTextRemovalPatch>,
        scale: f32,
    ) -> Result<Self, String> {
        renderer.clear_patches();
        let mut rects = vec![];
        let mut layers = vec![];
        let mut readers = vec![];
        for patch in &patches {
            let r = patch.rect;
            let x = ((r.x as f32 * scale).round() as u32).min(renderer.scene.width - 1);
            let y = ((r.y as f32 * scale).round() as u32).min(renderer.scene.height - 1);
            let rect = PixelRect {
                x,
                y,
                width: ((r.width as f32 * scale).round().max(1.) as u32)
                    .min(renderer.scene.width - x),
                height: ((r.height as f32 * scale).round().max(1.) as u32)
                    .min(renderer.scene.height - y),
            };
            let layer = if let Some(i) = rects.iter().position(|r| *r == rect) {
                layers[i]
            } else {
                renderer.register_patch(rect)?
            };
            rects.push(rect);
            layers.push(layer);
            readers.push(PatchReader::open(patch)?);
        }
        Ok(Self {
            readers,
            patches,
            layers,
            rects,
        })
    }
    pub fn upload(&mut self, renderer: &mut Renderer, time: f64) -> Result<usize, String> {
        let mut active = vec![];
        for (i, reader) in self.readers.iter_mut().enumerate() {
            if let Some(pixels) = reader.frame(time)? {
                let (r, scaled) = (self.patches[i].rect, self.rects[i]);
                let image = image::RgbaImage::from_raw(r.width, r.height, pixels.to_vec())
                    .ok_or("Patch pixels không hợp lệ.")?;
                let image = if (r.width, r.height) != (scaled.width, scaled.height) {
                    image::imageops::resize(
                        &image,
                        scaled.width,
                        scaled.height,
                        image::imageops::FilterType::Triangle,
                    )
                } else {
                    image
                };
                active.push((self.layers[i], image.into_raw()));
            }
        }
        renderer.upload_clean_preview(
            &active
                .iter()
                .map(|(i, p)| (*i, p.as_slice()))
                .collect::<Vec<_>>(),
        )?;
        Ok(active.len())
    }
}
