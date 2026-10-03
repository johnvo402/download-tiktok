//! Task 5 decoder, conversion and fences reused by the live preview target.
use super::{
    decoder::Frame,
    interop::{SharedTexture, Synchronizer},
    processor::Processor,
    Decoder, Session,
};
use crate::render::renderer::Renderer;
use std::{path::Path, sync::atomic::AtomicU8};
pub struct PreviewSource {
    next: Option<Frame>,
    held: Option<Frame>,
    fence: u64,
    decoder: Decoder,
    sync: Synchronizer,
    processor: Processor,
    input: SharedTexture,
    bind: wgpu::BindGroup,
    _session: Session,
    pub dropped: u64,
}
impl PreviewSource {
    pub fn new(renderer: &Renderer, path: &Path) -> Result<Self, String> {
        let session = Session::new(&renderer.device())?;
        let mut decoder = Decoder::new(&session, path)?;
        let first = decoder.next()?.ok_or("Video không có frame.")?;
        let processor = Processor::new(
            &session,
            (decoder.width, decoder.height),
            (renderer.scene.width, renderer.scene.height),
            decoder.matrix,
            decoder.range,
            true,
        )?;
        let input = SharedTexture::new(
            &session,
            &renderer.device(),
            renderer.scene.width,
            renderer.scene.height,
        )?;
        let bind = renderer.prepare_gpu_input(&input.texture)?;
        let sync = Synchronizer::new(&session)?;
        Ok(Self {
            next: Some(first),
            held: None,
            fence: 0,
            decoder,
            sync,
            processor,
            input,
            bind,
            _session: session,
            dropped: 0,
        })
    }
    pub fn seek(&mut self, time: f64) -> Result<(), String> {
        self.sync.wait_slot(self.fence, &AtomicU8::new(0))?;
        self.held = None;
        self.next = None;
        self.decoder.seek(time)?;
        self.next = self.decoder.next()?;
        Ok(())
    }
    pub fn select(&mut self, time: f64, stale: impl Fn() -> bool) -> Result<Option<f64>, String> {
        let mut selected = None;
        while self
            .next
            .as_ref()
            .is_some_and(|f| f.metadata.pts_100ns as f64 / 1e7 <= time + 0.0001)
        {
            if stale() {
                return Ok(None);
            }
            if selected.is_some() {
                self.dropped += 1;
            }
            selected = self.next.take();
            self.next = self.decoder.next()?;
        }
        if selected.is_none() && self.held.is_none() {
            selected = self.next.take();
            self.next = self.decoder.next()?;
        }
        if let Some(frame) = selected {
            self.sync.wait_slot(self.fence, &AtomicU8::new(0))?;
            self.held = Some(frame);
            self.processor
                .decode(self.held.as_ref().unwrap(), &self.input.native)?;
            self.sync.before_render()?;
        }
        Ok(self
            .held
            .as_ref()
            .map(|f| f.metadata.pts_100ns as f64 / 1e7))
    }
    pub fn present(
        &mut self,
        renderer: &mut Renderer,
        view: &wgpu::TextureView,
        size: (u32, u32),
        pts: f64,
        guide: Option<crate::render_spec::PixelRect>,
    ) -> Result<(), String> {
        renderer.present_preview(
            Some((&self.input.texture, &self.bind)),
            view,
            size,
            pts,
            guide,
        )?;
        self.sync.after_render()?;
        self.fence = self.sync.after_conversion()?;
        Ok(())
    }
}
impl Drop for PreviewSource {
    fn drop(&mut self) {
        let _ = self.sync.drain();
    }
}
