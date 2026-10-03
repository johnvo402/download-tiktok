use super::super::*;
impl Renderer {
    pub(in crate::render::renderer) fn build_blurs(
        gpu: &GpuDevice,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        output: &Texture,
        scene: &RenderScene,
    ) -> Vec<BlurResources> {
        let d = &gpu.device;
        scene
            .blurs
            .iter()
            .map(|r| {
                let factor = ((graph::blur_sigma(r.strength_px) * scene.pixel_scale) / 4.0)
                    .ceil()
                    .max(1.0);
                let w = (r.rect.width as f32 / factor).ceil().max(1.0) as u32;
                let h = (r.rect.height as f32 / factor).ceil().max(1.0) as u32;
                let a = Texture::new(d, w, h, "ROI blur A");
                let b = Texture::new(d, w, h, "ROI blur B");
                let crop = Self::bind_uniform(
                    d,
                    layout,
                    sampler,
                    output,
                    Params {
                        rect: [
                            r.rect.x as f32 / scene.width as f32,
                            r.rect.y as f32 / scene.height as f32,
                            r.rect.width as f32 / scene.width as f32,
                            r.rect.height as f32 / scene.height as f32,
                        ],
                        ..Default::default()
                    },
                );
                let horizontal = Self::bind_uniform(
                    d,
                    layout,
                    sampler,
                    &a,
                    Params {
                        step: [1.0 / w as f32, 0.0, 0.0, 0.0],
                        effect: [
                            (graph::blur_sigma(r.strength_px) * scene.pixel_scale) / factor,
                            0.0,
                            0.0,
                            1.0,
                        ],
                        ..Default::default()
                    },
                );
                let vertical = Self::bind_uniform(
                    d,
                    layout,
                    sampler,
                    &b,
                    Params {
                        step: [0.0, 1.0 / h as f32, 0.0, 0.0],
                        effect: [
                            (graph::blur_sigma(r.strength_px) * scene.pixel_scale) / factor,
                            0.0,
                            0.0,
                            1.0,
                        ],
                        ..Default::default()
                    },
                );
                let layer = Self::bind_uniform(
                    d,
                    layout,
                    sampler,
                    &a,
                    Params {
                        rect: [0.0, 0.0, r.rect.width as f32, r.rect.height as f32],
                        effect: [
                            1.0,
                            graph::feather_px(r.rect, r.strength_px, r.edge_blend),
                            r.pixel_noise as f32 / 100.0,
                            1.0,
                        ],
                        ..Default::default()
                    },
                );
                BlurResources {
                    a,
                    b,
                    crop,
                    horizontal,
                    vertical,
                    layer,
                }
            })
            .collect()
    }
}
