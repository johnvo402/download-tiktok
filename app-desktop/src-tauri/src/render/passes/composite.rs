use super::super::*;
use wgpu::util::DeviceExt;
impl Renderer {
    pub(in crate::render::renderer) fn bind(
        d: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: &Texture,
        params: Params,
    ) -> wgpu::BindGroup {
        Self::bind_uniform(d, layout, sampler, texture, params).bind
    }
    pub(in crate::render::renderer) fn bind_uniform(
        d: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: &Texture,
        params: Params,
    ) -> PassBind {
        let buffer = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("visual pass parameters"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        PassBind {
            bind,
            params: buffer,
        }
    }
    pub(in crate::render::renderer) fn layer_bind(
        d: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: &Texture,
        rect: PixelRect,
        opacity: f32,
    ) -> PassBind {
        Self::bind_uniform(
            d,
            layout,
            sampler,
            texture,
            Params {
                rect: [0.0, 0.0, rect.width as f32, rect.height as f32],
                effect: [1.0, 0.0, 0.0, opacity],
                ..Default::default()
            },
        )
    }
    pub(in crate::render::renderer) fn draw(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
        target: &Texture,
        rect: Option<PixelRect>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("visual pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if rect.is_some() {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        if let Some(rect) = rect {
            pass.set_viewport(
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(rect.x, rect.y, rect.width, rect.height);
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
