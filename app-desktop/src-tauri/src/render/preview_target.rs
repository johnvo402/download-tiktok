//! Preview target adapter. Visual passes remain in Renderer, exactly as export.
use super::*;
impl Renderer {
    pub fn update_scene(&mut self, scene: RenderScene, paths: &[PathBuf]) -> Result<(), String> {
        if scene.width != self.scene.width || scene.height != self.scene.height {
            return Err("Kích thước nguồn thay đổi; hãy mở lại preview.".into());
        }
        if paths.len() != scene.logos.len() {
            return Err("Danh sách logo không khớp.".into());
        }
        let old = std::mem::take(&mut self.logos);
        let mut logos = Vec::new();
        for (logo, path) in scene.logos.iter().zip(paths) {
            let cached = self
                .scene
                .logos
                .iter()
                .zip(&old)
                .find(|(prev, _)| prev.id == logo.id && prev.asset_path == logo.asset_path);
            if let Some((_, layer)) = cached {
                let texture = layer._texture.clone();
                let rect = graph::fitted_image_rect(
                    logo.rect,
                    texture.width,
                    texture.height,
                    logo.maintain_aspect_ratio,
                );
                let bind = layer.bind.clone();
                self.gpu.queue.write_buffer(
                    &bind.params,
                    0,
                    bytemuck::bytes_of(&Params {
                        rect: [0.0, 0.0, rect.width as f32, rect.height as f32],
                        effect: [1.0, 0.0, 0.0, logo.opacity as f32],
                        ..Default::default()
                    }),
                );
                logos.push(Layer {
                    _texture: texture,
                    rect,
                    bind,
                    bytes: 0,
                });
            } else {
                let mut one = scene.clone();
                one.logos = vec![logo.clone()];
                logos.extend(Self::build_logos(
                    &self.gpu,
                    &self.layout,
                    &self.sampler,
                    &one,
                    &[path.clone()],
                )?);
            }
        }
        self.logos = logos;
        if serde_json::to_value(&scene.blurs).ok() != serde_json::to_value(&self.scene.blurs).ok() {
            let mut resources = std::mem::take(&mut self.blurs).into_iter();
            let old_regions = self.scene.blurs.clone();
            let mut next = Vec::new();
            for (index, r) in scene.blurs.iter().enumerate() {
                let factor = ((graph::blur_sigma(r.strength_px) * scene.pixel_scale) / 4.0)
                    .ceil()
                    .max(1.0);
                let w = (r.rect.width as f32 / factor).ceil().max(1.0) as u32;
                let h = (r.rect.height as f32 / factor).ceil().max(1.0) as u32;
                let old = resources.next();
                if let Some(pass) = old.filter(|pass| {
                    old_regions.get(index).is_some_and(|prev| prev.id == r.id)
                        && pass.a.width == w
                        && pass.a.height == h
                }) {
                    let q = &self.gpu.queue;
                    q.write_buffer(
                        &pass.crop.params,
                        0,
                        bytemuck::bytes_of(&Params {
                            rect: [
                                r.rect.x as f32 / scene.width as f32,
                                r.rect.y as f32 / scene.height as f32,
                                r.rect.width as f32 / scene.width as f32,
                                r.rect.height as f32 / scene.height as f32,
                            ],
                            ..Default::default()
                        }),
                    );
                    q.write_buffer(
                        &pass.horizontal.params,
                        0,
                        bytemuck::bytes_of(&Params {
                            step: [1.0 / w as f32, 0.0, 0.0, 0.0],
                            effect: [
                                (graph::blur_sigma(r.strength_px) * scene.pixel_scale) / factor,
                                0.0,
                                0.0,
                                1.0,
                            ],
                            ..Default::default()
                        }),
                    );
                    q.write_buffer(
                        &pass.vertical.params,
                        0,
                        bytemuck::bytes_of(&Params {
                            step: [0.0, 1.0 / h as f32, 0.0, 0.0],
                            effect: [
                                (graph::blur_sigma(r.strength_px) * scene.pixel_scale) / factor,
                                0.0,
                                0.0,
                                1.0,
                            ],
                            ..Default::default()
                        }),
                    );
                    q.write_buffer(
                        &pass.layer.params,
                        0,
                        bytemuck::bytes_of(&Params {
                            rect: [0.0, 0.0, r.rect.width as f32, r.rect.height as f32],
                            effect: [
                                1.0,
                                graph::feather_px(r.rect, r.strength_px, r.edge_blend),
                                r.pixel_noise as f32 / 100.0,
                                1.0,
                            ],
                            ..Default::default()
                        }),
                    );
                    next.push(pass);
                } else {
                    let mut one = scene.clone();
                    one.blurs = vec![r.clone()];
                    next.extend(Self::build_blurs(
                        &self.gpu,
                        &self.layout,
                        &self.sampler,
                        &self.output,
                        &one,
                    ));
                }
            }
            self.blurs = next;
        }
        self.scene = scene;
        Ok(())
    }
    pub fn upload_preview_frame(&self, rgba: &[u8]) -> Result<(), String> {
        if rgba.len() != byte_len(self.scene.width, self.scene.height)? {
            return Err("Frame preview không hợp lệ.".into());
        }
        self.source.upload(&self.gpu.queue, rgba);
        Ok(())
    }
    pub fn present_preview(
        &mut self,
        input: Option<(&wgpu::Texture, &wgpu::BindGroup)>,
        view: &wgpu::TextureView,
        size: (u32, u32),
        pts: f64,
        guide: Option<PixelRect>,
    ) -> Result<(), String> {
        let keys = self.prepare_text(pts)?;
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        Self::draw(
            &mut encoder,
            &self.copy,
            input.map(|i| i.1).unwrap_or(&self.copy_bind),
            &self.output,
            None,
        );
        for index in &self.preview_patches {
            let layer=&self.patches[*index];
            Self::draw(&mut encoder,&self.composite,&layer.bind,&self.output,Some(layer.rect));
        }
        self.encode_visual(&mut encoder, pts, keys);
        // Interaction guides are GPU-only UI. They never enter the export scene.
        if let Some(rect) = guide {
            let color = self
                .selection
                .get_or_insert_with(|| {
                    let color = Arc::new(Texture::new(&self.gpu.device, 1, 1, "selection guide"));
                    color.upload(&self.gpu.queue, &[209, 239, 91, 255]);
                    color
                })
                .clone();
            let thick = (self.scene.width as f32 / size.0.max(1) as f32 * 2.0).ceil() as u32;
            let handle = (thick * 4).min(rect.width).min(rect.height);
            for edge in [
                PixelRect {
                    height: thick.min(rect.height),
                    ..rect
                },
                PixelRect {
                    y: rect.y + rect.height.saturating_sub(thick),
                    height: thick.min(rect.height),
                    ..rect
                },
                PixelRect {
                    width: thick.min(rect.width),
                    ..rect
                },
                PixelRect {
                    x: rect.x + rect.width.saturating_sub(thick),
                    width: thick.min(rect.width),
                    ..rect
                },
                PixelRect {
                    x: rect.x + rect.width - handle,
                    y: rect.y + rect.height - handle,
                    width: handle,
                    height: handle,
                },
            ] {
                let bind = Self::layer_bind(
                    &self.gpu.device,
                    &self.layout,
                    &self.sampler,
                    &color,
                    edge,
                    1.0,
                );
                Self::draw(
                    &mut encoder,
                    &self.composite,
                    &bind,
                    &self.output,
                    Some(edge),
                );
            }
        }
        let scale =
            (size.0 as f32 / self.scene.width as f32).min(size.1 as f32 / self.scene.height as f32);
        let (w, h) = (
            self.scene.width as f32 * scale,
            self.scene.height as f32 * scale,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_viewport(
                (size.0 as f32 - w) / 2.0,
                (size.1 as f32 - h) / 2.0,
                w,
                h,
                0.0,
                1.0,
            );
            pass.set_pipeline(&self.copy_bgra);
            pass.set_bind_group(0, &self.output_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some((texture, _)) = input {
            encoder.transition_resources(
                std::iter::empty(),
                [wgpu::TextureTransition {
                    texture,
                    selector: None,
                    state: wgpu::TextureUses::PRESENT,
                }]
                .into_iter(),
            );
        }
        self.gpu.queue.submit([encoder.finish()]);
        self.check_health()
    }
}
