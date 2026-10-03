use super::super::*;
impl Renderer {
    pub(in crate::render::renderer) fn build_logos(
        gpu: &GpuDevice,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        scene: &RenderScene,
        logo_paths: &[PathBuf],
    ) -> Result<Vec<Layer>, String> {
        let d = &gpu.device;
        if logo_paths.len() != scene.logos.len() {
            return Err("Danh sách logo không khớp cấu hình render.".into());
        }
        let mut images: HashMap<PathBuf, Arc<Texture>> = HashMap::new();
        let mut logos = Vec::new();
        for (logo, path) in scene.logos.iter().zip(logo_paths) {
            let texture = if let Some(texture) = images.get(path) {
                texture.clone()
            } else {
                let pixels = image::open(path)
                    .map_err(|e| format!("Không đọc được ảnh logo: {e}"))?
                    .into_rgba8();
                if pixels.width() > d.limits().max_texture_dimension_2d
                    || pixels.height() > d.limits().max_texture_dimension_2d
                    || pixels.len() > 64 * 1024 * 1024
                {
                    return Err("Ảnh logo quá lớn để render GPU.".into());
                }
                let texture = Arc::new(Texture::new(
                    d,
                    pixels.width(),
                    pixels.height(),
                    "cached logo",
                ));
                texture.upload(&gpu.queue, &pixels);
                images.insert(path.clone(), texture.clone());
                texture
            };
            let rect = graph::fitted_image_rect(
                logo.rect,
                texture.width,
                texture.height,
                logo.maintain_aspect_ratio,
            );
            let bind = Self::layer_bind(d, layout, sampler, &texture, rect, logo.opacity as f32);
            logos.push(Layer {
                _texture: texture,
                bind,
                rect,
                bytes: 0,
            });
        }
        Ok(logos)
    }
}
