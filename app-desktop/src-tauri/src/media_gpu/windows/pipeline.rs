use super::{
    decoder::{Decoder, Frame},
    device::Session,
    encoder::Encoder,
    interop::{SharedTexture, Synchronizer},
    processor::Processor,
};
use crate::{
    hardware_acceleration::VideoCodec,
    media_gpu::pipeline::Counters,
    render::{graph::RenderScene, renderer::Renderer},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
    time::Instant,
};
struct Slot {
    input: SharedTexture,
    output: SharedTexture,
    bind: wgpu::BindGroup,
    target: crate::render::renderer::GpuTarget,
    fence: u64,
    held: Option<Frame>,
}
pub struct NativePipeline {
    slots: Vec<Slot>,
    decoder: Decoder,
    encoder: Encoder,
    sync: Synchronizer,
    to_rgb: Processor,
    to_nv12: Processor,
    renderer: Renderer,
    first: Option<Frame>,
    _session: Session,
}
impl NativePipeline {
    pub fn new(
        source: &Path,
        output: &Path,
        scene: RenderScene,
        logos: &[PathBuf],
        codec: VideoCodec,
        bitrate: u32,
        vendor: Option<u32>,
    ) -> Result<Self, String> {
        let renderer = Renderer::new_preferred(scene, logos, vendor)?;
        if renderer.adapter_backend() != "Dx12" {
            return Err("Thiết bị chưa hỗ trợ chia sẻ texture video Windows.".into());
        }
        let session = Session::new(&renderer.device())?;
        let mut decoder = Decoder::new(&session, source)?;
        let first = decoder.next()?.ok_or("Video không có khung hình.")?;
        let size = (renderer.scene.width, renderer.scene.height);
        let encoder = Encoder::new(
            &session,
            output,
            size,
            decoder.fps,
            codec,
            bitrate,
            decoder.matrix,
            decoder.range,
        )?;
        let sync = Synchronizer::new(&session)?;
        let to_rgb = Processor::new(
            &session,
            (decoder.width, decoder.height),
            size,
            decoder.matrix,
            decoder.range,
            true,
        )?;
        let to_nv12 = Processor::new(&session, size, size, decoder.matrix, decoder.range, false)?;
        let mut slots = Vec::new();
        for _ in 0..3 {
            let input = SharedTexture::new(&session, &renderer.device(), size.0, size.1)?;
            let output = SharedTexture::new(&session, &renderer.device(), size.0, size.1)?;
            let bind = renderer.prepare_gpu_input(&input.texture)?;
            let target = renderer.prepare_gpu_target(&output.texture)?;
            slots.push(Slot {
                input,
                output,
                bind,
                target,
                fence: 0,
                held: None,
            });
        }
        log::info!("Media pipeline: Tier B; decode=MF D3D11 hardware; format=NV12; renderer=wgpu DX12 {}; adapter={:?}; encode={}; color={:?}/{:?}; transport=shared GPU surfaces; CPU full-frame upload/readback=0; ring=3, encoder pool=6",
   renderer.adapter_name(),session.adapter,encoder.name,decoder.matrix,decoder.range);
        let pool_bytes = size.0 as u64 * size.1 as u64 * (6 * 4 + 6 * 3 / 2 + 2 * 4);
        log::info!("Native fixed surfaces estimate={}MiB; excludes decoder pool, ROI, fonts and driver memory", pool_bytes / 1048576);
        Ok(Self {
            slots,
            decoder,
            encoder,
            sync,
            to_rgb,
            to_nv12,
            renderer,
            first: Some(first),
            _session: session,
        })
    }
    pub fn run(
        &mut self,
        state: &AtomicU8,
        mut progress: impl FnMut(f64, u64),
    ) -> Result<Counters, String> {
        let plan =
            crate::media_gpu::capability::resolve(&crate::media_gpu::capability::Capabilities {
                hardware_decode: true,
                gpu_render: true,
                shared_surfaces: true,
                hardware_encode: true,
                direct_surfaces: false,
                same_adapter: true,
            });
        debug_assert_eq!(plan, crate::media_gpu::capability::Transport::GpuCopy);
        let started = Instant::now();
        let mut host = super::metrics::HostMetrics::new();
        let (mut decode_time, mut render_time, mut encode_time) = (
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        );
        let mut counts = Counters::default();
        let mut last_pts = None;
        let mut frame = self.first.take();
        while let Some(mut current) = frame {
            if state.load(Ordering::SeqCst) == 2 {
                return Err("Đã hủy export.".into());
            }
            if last_pts.is_some_and(|previous| current.metadata.pts_100ns <= previous) {
                return Err("Timestamp video không tăng theo thứ tự phát.".into());
            }
            let step = Instant::now();
            frame = self.decoder.next()?;
            decode_time += step.elapsed();
            // One-frame lookahead preserves VFR durations and delayed decoder frames.
            if let Some(next) = &frame {
                current.metadata.duration_100ns =
                    next.metadata.pts_100ns - current.metadata.pts_100ns;
            }
            if current.metadata.duration_100ns <= 0 {
                return Err("Khung hình video thiếu thời lượng hợp lệ.".into());
            }
            let slot = &mut self.slots[counts.frames as usize % 3];
            self.sync.wait_slot(slot.fence, state)?;
            slot.held.take();
            let sample = self.encoder.allocate(state)?;
            let (pts, duration) = (current.metadata.pts_100ns, current.metadata.duration_100ns);
            slot.held = Some(current); // hold before submitting any GPU read, including error paths
            let step = Instant::now();
            self.to_rgb
                .decode(slot.held.as_ref().unwrap(), &slot.input.native)?;
            self.sync.before_render()?;
            self.renderer.render_surfaces(
                &slot.input.texture,
                &slot.bind,
                &slot.target,
                pts as f64 / 10_000_000.0,
            )?;
            self.sync.after_render()?;
            self.to_nv12
                .convert(&slot.output.native, 0, &sample.texture, sample.slice)?;
            slot.fence = self.sync.after_conversion()?;
            render_time += step.elapsed();
            let step = Instant::now();
            self.encoder.write(sample, pts, duration)?;
            encode_time += step.elapsed();
            last_pts = Some(pts);
            let pts = pts as f64 / 10_000_000.0;
            counts.frames += 1;
            counts.gpu_copies += 4;
            host.sample_throttled();
            progress(pts, counts.frames);
        }
        for slot in &mut self.slots {
            self.sync.wait_slot(slot.fence, state)?;
            slot.held.take();
        }
        if state.load(Ordering::SeqCst) == 2 {
            return Err("Đã hủy export.".into());
        }
        let step = Instant::now();
        self.encoder.finish()?;
        encode_time += step.elapsed();
        self.renderer.check_health()?;
        // Host wait/submission time; async GPU stages overlap and these are NOT GPU FPS.
        log::info!("Native host stage seconds: decode/read={:.3}; convert/render-submit={:.3}; encode/write/flush={:.3}", decode_time.as_secs_f64(), render_time.as_secs_f64(), encode_time.as_secs_f64());
        log::info!("{}", host.report("Native video"));
        log::info!("Native GPU export: frames={}; full-frame upload_bytes={}; readback_bytes={}; gpu_transport_passes={}; wall={:.3}s; fps={:.1}",counts.frames,counts.upload_bytes,counts.readback_bytes,counts.gpu_copies,started.elapsed().as_secs_f64(),counts.frames as f64/started.elapsed().as_secs_f64());
        Ok(counts)
    }
}
impl Drop for NativePipeline {
    fn drop(&mut self) {
        // Keep decoder surfaces owned until the last queued read is complete, even on cancel.
        if let Err(error) = self.sync.drain() {
            log::warn!("GPU teardown: {error}");
        }
    }
}
