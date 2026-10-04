use super::{
    device::{self, GpuDevice},
    frame::{byte_len, PixelFormat, VideoFrame},
    graph::{self, RenderScene},
    text::{ass_rgb, rgb, TextRasterizer, TextStyle},
    texture::{Texture, FORMAT},
};
use crate::render_spec::PixelRect;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    rect: [f32; 4],
    step: [f32; 4],
    effect: [f32; 4],
}
impl Default for Params {
    fn default() -> Self {
        Self {
            rect: [0.0, 0.0, 1.0, 1.0],
            step: [0.0; 4],
            effect: [1.0, 0.0, 0.0, 1.0],
        }
    }
}
#[derive(Clone)]
struct PassBind {
    bind: wgpu::BindGroup,
    params: wgpu::Buffer,
}
impl std::ops::Deref for PassBind {
    type Target = wgpu::BindGroup;
    fn deref(&self) -> &Self::Target {
        &self.bind
    }
}
struct BlurResources {
    a: Texture,
    b: Texture,
    crop: PassBind,
    horizontal: PassBind,
    vertical: PassBind,
    layer: PassBind,
}
struct Layer {
    _texture: Arc<Texture>,
    bind: PassBind,
    rect: PixelRect,
    bytes: usize,
}

/// Cached GPU target view; COM/native handles stay inside media_gpu/windows.
pub struct GpuTarget {
    inner: Texture,
}
pub struct Renderer {
    gpu: GpuDevice,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    copy: wgpu::RenderPipeline,
    copy_bgra: wgpu::RenderPipeline,
    output_bind: wgpu::BindGroup,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    source: Texture,
    output: Texture,
    copy_bind: wgpu::BindGroup,
    readback: Option<wgpu::Buffer>,
    stride: u32,
    blurs: Vec<BlurResources>,
    logos: Vec<Layer>,
    raster: TextRasterizer,
    text: HashMap<String, Layer>,
    lru: VecDeque<String>,
    text_bytes: usize,
    pub(crate) scene: RenderScene,
    patches: Vec<Layer>,
    preview_patches: Vec<usize>,
    selection: Option<Arc<Texture>>,
}
impl Renderer {
    pub fn new(scene: RenderScene, logo_paths: &[PathBuf]) -> Result<Self, String> {
        Self::new_preferred(scene, logo_paths, None)
    }
    pub fn new_preferred(
        scene: RenderScene,
        logo_paths: &[PathBuf],
        vendor: Option<u32>,
    ) -> Result<Self, String> {
        Self::with_device(scene, logo_paths, device::create_preferred(vendor)?)
    }
    pub fn with_device(
        scene: RenderScene,
        logo_paths: &[PathBuf],
        gpu: GpuDevice,
    ) -> Result<Self, String> {
        for rect in scene
            .blurs
            .iter()
            .map(|r| r.rect)
            .chain(scene.logos.iter().map(|r| r.rect))
            .chain(scene.text.iter().map(|r| r.rect))
            .chain(scene.subtitle.region)
        {
            if rect.width == 0
                || rect.height == 0
                || rect
                    .x
                    .checked_add(rect.width)
                    .is_none_or(|r| r > scene.width)
                || rect
                    .y
                    .checked_add(rect.height)
                    .is_none_or(|b| b > scene.height)
            {
                return Err("Vùng hiệu ứng vượt kích thước video.".into());
            }
        }
        let d = &gpu.device;
        if scene.width > d.limits().max_texture_dimension_2d
            || scene.height > d.limits().max_texture_dimension_2d
        {
            return Err("Độ phân giải vượt khả năng của GPU.".into());
        }
        byte_len(scene.width, scene.height)?;
        d.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("visual pass"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("visual WGSL"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/effects.wgsl").into()),
        });
        let pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry: &str, blend, format| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let copy = pipeline("copy", None, FORMAT);
        let copy_bgra = pipeline("copy", None, wgpu::TextureFormat::Bgra8Unorm);
        let blur = pipeline("blur", None, FORMAT);
        let composite = pipeline("layer", Some(wgpu::BlendState::ALPHA_BLENDING), FORMAT);
        if let Some(error) = pollster::block_on(d.pop_error_scope()) {
            log::error!("GPU shader/pipeline: {error}");
            return Err("Không biên dịch được hiệu ứng GPU. Hãy dùng chế độ tương thích.".into());
        }
        let source = Texture::new(d, scene.width, scene.height, "source frame");
        let output = Texture::new(d, scene.width, scene.height, "composite target");
        let output_bind = Self::bind(d, &layout, &sampler, &output, Params::default());
        let copy_bind = Self::bind(d, &layout, &sampler, &source, Params::default());
        let stride = (scene.width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        // Native surface rendering never needs a CPU-readable full-frame buffer.
        let readback = None;
        let blurs = Self::build_blurs(&gpu, &layout, &sampler, &output, &scene);
        let logos = Self::build_logos(&gpu, &layout, &sampler, &scene, logo_paths)?;
        Ok(Self {
            gpu,
            layout,
            sampler,
            copy,
            copy_bgra,
            output_bind,
            blur,
            composite,
            source,
            output,
            copy_bind,
            readback,
            stride,
            blurs,
            logos,
            raster: TextRasterizer::default(),
            text: HashMap::new(),
            lru: VecDeque::new(),
            text_bytes: 0,
            scene,
            patches: Vec::new(),
            preview_patches: Vec::new(),
            selection: None,
        })
    }
    /// Borrowed frame API permits a future GPU-surface input without tying scenes to export jobs.
    pub fn register_patch(&mut self, rect: PixelRect) -> Result<usize, String> {
        if rect.width == 0
            || rect.height == 0
            || rect
                .x
                .checked_add(rect.width)
                .is_none_or(|r| r > self.scene.width)
            || rect
                .y
                .checked_add(rect.height)
                .is_none_or(|b| b > self.scene.height)
        {
            return Err("Vùng phục hồi vượt video.".into());
        }
        let texture = Arc::new(Texture::new(
            &self.gpu.device,
            rect.width,
            rect.height,
            "streaming clean patch",
        ));
        let bind = Self::layer_bind(
            &self.gpu.device,
            &self.layout,
            &self.sampler,
            &texture,
            rect,
            1.0,
        );
        let index = self.patches.len();
        self.patches.push(Layer {
            _texture: texture,
            bind,
            rect,
            bytes: 0,
        });
        Ok(index)
    }
    pub fn clear_patches(&mut self) {
        self.patches.clear();
        self.preview_patches.clear();
    }
    pub fn upload_clean_preview(&mut self, patches: &[(usize, &[u8])]) -> Result<(), String> {
        self.preview_patches.clear();
        for (index, pixels) in patches {
            let layer=self.patches.get(*index).ok_or("Patch preview chưa đăng ký.")?;
            if pixels.len()!=byte_len(layer.rect.width,layer.rect.height)?{return Err("Kích thước patch preview không hợp lệ.".into())}
            layer._texture.upload(&self.gpu.queue,pixels);
            self.preview_patches.push(*index);
        }
        Ok(())
    }
    pub fn render_frame(&mut self, frame: VideoFrame<'_>, output: &mut [u8]) -> Result<(), String> {
        self.render_frame_with_patches(frame, &[], output)
    }
    pub fn render_frame_with_patches(
        &mut self,
        frame: VideoFrame<'_>,
        patches: &[(usize, &[u8])],
        output: &mut [u8],
    ) -> Result<(), String> {
        let len = byte_len(self.scene.width, self.scene.height)?;
        if frame.width != self.scene.width
            || frame.height != self.scene.height
            || frame.format != PixelFormat::Rgba8
            || frame.data.len() != len
            || output.len() != len
            || !frame.pts_seconds.is_finite()
        {
            return Err("Frame đầu vào GPU không hợp lệ.".into());
        }
        let pts = frame.pts_seconds;
        let keys = self.prepare_text(pts)?;
        self.source.upload(&self.gpu.queue, frame.data);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        Self::draw(
            &mut encoder,
            &self.copy,
            &self.copy_bind,
            &self.output,
            None,
        );
        for (index, data) in patches {
            let layer = self
                .patches
                .get(*index)
                .ok_or("Patch không được đăng ký.")?;
            if data.len() != byte_len(layer.rect.width, layer.rect.height)? {
                return Err("Frame phục hồi không hợp lệ.".into());
            }
            layer._texture.upload(&self.gpu.queue, data);
            Self::draw(
                &mut encoder,
                &self.composite,
                &layer.bind,
                &self.output,
                Some(layer.rect),
            );
        }
        self.encode_visual(&mut encoder, pts, keys);
        let readback = self.readback.get_or_insert_with(|| {
            self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame readback"),
                size: self.stride as u64 * self.scene.height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        encoder.copy_texture_to_buffer(
            self.output.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: Some(self.scene.height),
                },
            },
            wgpu::Extent3d {
                width: self.scene.width,
                height: self.scene.height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(10)),
            })
            .map_err(|e| format!("GPU không hoàn thành frame: {e}"))?;
        rx.recv_timeout(Duration::from_secs(1))
            .map_err(|_| "GPU không trả frame.")?
            .map_err(|_| "Không đọc được frame từ GPU.")?;
        let mapped = readback.slice(..).get_mapped_range();
        for (src, dst) in mapped
            .chunks(self.stride as usize)
            .zip(output.chunks_mut(self.scene.width as usize * 4))
        {
            dst.copy_from_slice(&src[..dst.len()]);
        }
        drop(mapped);
        readback.unmap();
        if let Some(error) = self
            .gpu
            .error
            .lock()
            .map_err(|_| "Không đọc được trạng thái GPU.")?
            .take()
        {
            return Err(error);
        }
        Ok(())
    }

    fn prepare_text(&mut self, pts: f64) -> Result<Vec<String>, String> {
        let mut requests = Vec::new();
        if self.scene.subtitle.enabled {
            let sub = &self.scene.subtitle;
            let rect = sub.region.ok_or("Thiếu vùng phụ đề.")?;
            for event in sub.events.iter().filter(|e| pts >= e.start && pts < e.end) {
                let key = format!("sub:{}", event.segment_id);
                let style = TextStyle {
                    font: sub.font_name.clone(),
                    size: event.layout.font_size,
                    weight: sub.font_weight,
                    color: ass_rgb(&sub.primary_colour)?,
                    outline: ass_rgb(&sub.outline_colour)?,
                    outline_width: sub.outline_width,
                    alignment: sub.alignment.clone(),
                    vertical: sub.vertical_alignment.clone(),
                    line_height: sub.line_height as f32,
                    padding_x: rect.width * sub.padding_x as u32 / 100,
                    padding_y: rect.height * sub.padding_y as u32 / 100,
                    background: rgb(&sub.background_color, sub.background_opacity)?,
                    strict_fit: true,
                };
                requests.push((
                    key,
                    event.layout.text.clone(),
                    rect,
                    style,
                    if sub.auto_fit {
                        sub.min_font_size
                    } else {
                        event.layout.font_size
                    },
                ));
            }
        }
        for text in &self.scene.text {
            let key = format!("text:{}", text.id);
            requests.push((
                key,
                text.text.clone(),
                text.rect,
                TextStyle {
                    font: text.font_name.clone(),
                    size: text.font_size,
                    weight: text.font_weight,
                    color: rgb(&text.color, text.opacity)?,
                    outline: [0; 4],
                    outline_width: 0,
                    alignment: text.alignment.clone(),
                    vertical: "middle".into(),
                    line_height: 1.2,
                    padding_x: 0,
                    padding_y: 0,
                    background: [0; 4],
                    strict_fit: false,
                },
                text.font_size,
            ));
        }
        let required: usize = requests
            .iter()
            .map(|(_, _, rect, _, _)| rect.width as usize * rect.height as usize * 4)
            .sum();
        if required > 64 * 1024 * 1024 || requests.len() > 64 {
            return Err("Quá nhiều lớp chữ hiển thị đồng thời để render GPU.".into());
        }
        let mut keys = Vec::with_capacity(requests.len());
        for (key, text, rect, style, minimum) in requests {
            keys.push(self.cache_text(key, &text, rect, style, minimum)?);
        }
        Ok(keys)
    }
    fn encode_visual(&self, encoder: &mut wgpu::CommandEncoder, pts: f64, keys: Vec<String>) {
        // Explicit order: clean patches -> blur -> subtitle -> text -> logo.
        for (region, pass) in self.scene.blurs.iter().zip(&self.blurs) {
            if graph::active(pts, region.start_ms, region.end_ms) {
                Self::draw(encoder, &self.copy, &pass.crop, &pass.a, None);
                Self::draw(encoder, &self.blur, &pass.horizontal, &pass.b, None);
                Self::draw(encoder, &self.blur, &pass.vertical, &pass.a, None);
                Self::draw(
                    encoder,
                    &self.composite,
                    &pass.layer,
                    &self.output,
                    Some(region.rect),
                );
            }
        }
        for key in keys {
            let layer = &self.text[&key];
            Self::draw(
                encoder,
                &self.composite,
                &layer.bind,
                &self.output,
                Some(layer.rect),
            );
        }
        for (logo, layer) in self.scene.logos.iter().zip(&self.logos) {
            if graph::active(pts, logo.start_ms, logo.end_ms) {
                Self::draw(
                    encoder,
                    &self.composite,
                    &layer.bind,
                    &self.output,
                    Some(layer.rect),
                );
            }
        }
    }
    /// Reuses the same visual passes with GPU textures. No full video CPU upload/readback.
    pub fn prepare_gpu_input(&self, texture: &wgpu::Texture) -> Result<wgpu::BindGroup, String> {
        if texture.width() != self.scene.width
            || texture.height() != self.scene.height
            || texture.format() != wgpu::TextureFormat::Bgra8Unorm
        {
            return Err("Texture video không tương thích với bộ render.".into());
        }
        let wrapper = Texture {
            texture: texture.clone(),
            view: texture.create_view(&Default::default()),
            width: texture.width(),
            height: texture.height(),
        };
        Ok(Self::bind(
            &self.gpu.device,
            &self.layout,
            &self.sampler,
            &wrapper,
            Params::default(),
        ))
    }
    pub fn prepare_gpu_target(&self, target: &wgpu::Texture) -> Result<GpuTarget, String> {
        if target.width() != self.scene.width
            || target.height() != self.scene.height
            || target.format() != wgpu::TextureFormat::Bgra8Unorm
        {
            return Err("Texture đích video không hợp lệ.".into());
        }
        Ok(GpuTarget {
            inner: Texture {
                texture: target.clone(),
                view: target.create_view(&Default::default()),
                width: target.width(),
                height: target.height(),
            },
        })
    }
    pub fn render_surfaces(
        &mut self,
        input: &wgpu::Texture,
        input_bind: &wgpu::BindGroup,
        target: &GpuTarget,
        pts: f64,
    ) -> Result<(), String> {
        let texture = &target.inner.texture;
        if !pts.is_finite()
            || texture.width() != self.scene.width
            || texture.height() != self.scene.height
            || texture.format() != wgpu::TextureFormat::Bgra8Unorm
        {
            return Err("Texture đích video không hợp lệ.".into());
        }
        let keys = self.prepare_text(pts)?;
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        Self::draw(&mut encoder, &self.copy, input_bind, &self.output, None);
        self.encode_visual(&mut encoder, pts, keys);
        Self::draw(
            &mut encoder,
            &self.copy_bgra,
            &self.output_bind,
            &target.inner,
            None,
        );
        // COMMON is required before the D3D11 video processor accesses a shared surface.
        encoder.transition_resources(
            std::iter::empty(),
            [input, texture]
                .into_iter()
                .map(|texture| wgpu::TextureTransition {
                    texture,
                    selector: None,
                    state: wgpu::TextureUses::PRESENT,
                }),
        );
        self.gpu.queue.submit([encoder.finish()]);
        self.check_health()
    }
    pub fn check_health(&self) -> Result<(), String> {
        self.gpu
            .error
            .lock()
            .map_err(|_| "Không đọc được trạng thái GPU.")?
            .clone()
            .map_or(Ok(()), Err)
    }

    pub fn device(&self) -> wgpu::Device {
        self.gpu.device.clone()
    }
    pub fn queue(&self) -> wgpu::Queue {
        self.gpu.queue.clone()
    }
    pub fn adapter_name(&self) -> &str {
        &self.gpu.info.name
    }
    pub fn adapter_backend(&self) -> String {
        format!("{:?}", self.gpu.info.backend)
    }
}

#[path = "passes/mod.rs"]
mod passes;
#[path = "preview_target.rs"]
mod preview_target;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
