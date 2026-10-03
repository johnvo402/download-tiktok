//! Hardware MF sink writer with an allocator-owned bounded NV12 surface pool.
use super::device::Session;
use crate::{
    hardware_acceleration::VideoCodec,
    media_gpu::frame::{Matrix, Range},
};
use std::{
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
    time::{Duration, Instant},
};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{Graphics::Direct3D11::*, Media::MediaFoundation::*},
};
pub struct Encoder {
    writer: IMFSinkWriter,
    allocator: IMFVideoSampleAllocatorEx,
    index: u32,
    pub name: String,
    _lifecycle: std::rc::Rc<super::device::Lifecycle>,
}
pub struct Sample {
    pub(crate) sample: IMFSample,
    pub(crate) texture: ID3D11Texture2D,
    pub(crate) slice: u32,
}
impl Encoder {
    pub fn new(
        session: &Session,
        path: &Path,
        size: (u32, u32),
        fps: u64,
        codec: VideoCodec,
        bitrate: u32,
        matrix: Matrix,
        range: Range,
    ) -> Result<Self, String> {
        unsafe {
            Self::create(session, path, size, fps, codec, bitrate, matrix, range).map_err(|e| {
                log::warn!("MF hardware encoder: {e:?}");
                "Bộ mã hóa phần cứng chưa hỗ trợ chế độ xử lý này. Hãy chọn Auto.".into()
            })
        }
    }
    unsafe fn create(
        s: &Session,
        path: &Path,
        size: (u32, u32),
        fps: u64,
        codec: VideoCodec,
        bitrate: u32,
        matrix: Matrix,
        range: Range,
    ) -> windows::core::Result<Self> {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 4)?;
        let attrs = attrs.unwrap();
        attrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &s.manager)?;
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        // Hardware-transform selection conflicts with DISABLE_CONVERTERS in MF.
        // Input is explicitly NV12; the real encoder MFT is verified below.
        let wide = super::path::mf_url(path);
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, &attrs)
            .map_err(|e| stage(e, "CreateSinkWriter"))?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, ((size.0 as u64) << 32) | size.1 as u64)?;
        input.SetUINT64(&MF_MT_FRAME_RATE, fps)?;
        input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        let m = match matrix {
            Matrix::Bt601 => MFVideoTransferMatrix_BT601,
            Matrix::Bt709 => MFVideoTransferMatrix_BT709,
            Matrix::Bt2020 => MFVideoTransferMatrix_BT2020_10,
        };
        input.SetUINT32(&MF_MT_YUV_MATRIX, m.0 as u32)?;
        input.SetUINT32(
            &MF_MT_VIDEO_PRIMARIES,
            match matrix {
                Matrix::Bt601 => MFVideoPrimaries_SMPTE170M,
                Matrix::Bt709 => MFVideoPrimaries_BT709,
                Matrix::Bt2020 => MFVideoPrimaries_BT2020,
            }
            .0 as u32,
        )?;
        input.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        input.SetUINT32(
            &MF_MT_VIDEO_NOMINAL_RANGE,
            match range {
                Range::Limited => MFNominalRange_16_235,
                Range::Full => MFNominalRange_0_255,
            }
            .0 as u32,
        )?;
        let output = MFCreateMediaType()?;
        input.CopyAllItems(&output)?;
        output.SetGUID(
            &MF_MT_SUBTYPE,
            &match codec {
                VideoCodec::H264 => MFVideoFormat_H264,
                VideoCodec::Hevc => MFVideoFormat_HEVC,
            },
        )?;
        output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        output.SetUINT32(
            &MF_MT_MPEG2_PROFILE,
            match codec {
                VideoCodec::H264 => 100,
                VideoCodec::Hevc => 1,
            },
        )?;
        let index = writer
            .AddStream(&output)
            .map_err(|e| stage(e, "AddStream"))?;
        writer
            .SetInputMediaType(index, &input, None)
            .map_err(|e| stage(e, "SetInputMediaType"))?;
        writer
            .BeginWriting()
            .map_err(|e| stage(e, "BeginWriting"))?;
        // A successful sink writer is NOT evidence of hardware encoding. Inspect its
        // real encoder MFT; reject silently selected software transforms before frames.
        let ex: IMFSinkWriterEx = writer.cast()?;
        let mut transform = None;
        let mut category = windows::core::GUID::zeroed();
        ex.GetTransformForStream(index, 0, Some(&mut category), &mut transform)
            .map_err(|e| stage(e, "GetTransformForStream"))?;
        let transform = transform.unwrap();
        let ta = transform.GetAttributes()?;
        if category != MFT_CATEGORY_VIDEO_ENCODER
            || ta.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) != 1
            || transform.GetInputCurrentType(0)?.GetGUID(&MF_MT_SUBTYPE)? != MFVideoFormat_NV12
        {
            return Err(windows::core::Error::from_hresult(
                MF_E_UNSUPPORTED_D3D_TYPE,
            ));
        }
        let n = ta
            .GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute)
            .map_err(|e| stage(e, "HardwareURL"))?;
        if n == 0 {
            return Err(windows::core::Error::from_hresult(
                MF_E_UNSUPPORTED_D3D_TYPE,
            ));
        }
        if let Ok(luid) = ta.GetUINT64(&MFT_ENUM_ADAPTER_LUID) {
            let expected = ((s.adapter.high as u32 as u64) << 32) | s.adapter.low as u64;
            if luid != expected {
                return Err(windows::core::Error::from_hresult(
                    MF_E_UNSUPPORTED_D3D_TYPE,
                ));
            }
        }
        let name = "Media Foundation hardware encoder".to_string();
        let mut allocator: Option<IMFVideoSampleAllocatorEx> = None;
        MFCreateVideoSampleAllocatorEx(
            &IMFVideoSampleAllocatorEx::IID,
            &mut allocator as *mut _ as *mut _,
        )?;
        let allocator = allocator.unwrap();
        allocator.SetDirectXManager(&s.manager)?;
        let mut aa = None;
        MFCreateAttributes(&mut aa, 2)?;
        let aa = aa.unwrap();
        aa.SetUINT32(&MF_SA_D3D11_BINDFLAGS, D3D11_BIND_RENDER_TARGET.0 as u32)?;
        aa.SetUINT32(&MF_SA_D3D11_USAGE, D3D11_USAGE_DEFAULT.0 as u32)?;
        allocator
            .InitializeSampleAllocatorEx(6, 6, &aa, &input)
            .map_err(|e| stage(e, "NV12 pool"))?;
        Ok(Self {
            writer,
            allocator,
            index,
            name,
            _lifecycle: s.lifecycle.clone(),
        })
    }
    pub fn allocate(&self, state: &AtomicU8) -> Result<Sample, String> {
        let start = Instant::now();
        loop {
            if state.load(Ordering::SeqCst) == 2 {
                return Err("Đã hủy export.".into());
            }
            let result = unsafe { self.allocator.AllocateSample() };
            match result {
                Ok(sample) => {
                    return unsafe {
                        (|| -> windows::core::Result<Sample> {
                            let buffer = sample.GetBufferByIndex(0)?.cast::<IMFDXGIBuffer>()?;
                            let mut texture: Option<ID3D11Texture2D> = None;
                            buffer.GetResource(
                                &ID3D11Texture2D::IID,
                                &mut texture as *mut _ as *mut _,
                            )?;
                            Ok(Sample {
                                sample,
                                texture: texture.unwrap(),
                                slice: buffer.GetSubresourceIndex()?,
                            })
                        })()
                        .map_err(|e| {
                            super::native_error("Bộ mã hóa không cấp được khung hình GPU.", e)
                        })
                    }
                }
                Err(e)
                    if e.code() == MF_E_SAMPLEALLOCATOR_EMPTY
                        && start.elapsed() < Duration::from_secs(15) =>
                {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(e) => {
                    return Err(super::native_error(
                        "Bộ mã hóa không nhận thêm khung hình. Hãy thử lại.",
                        e,
                    ))
                }
            }
        }
    }
    pub fn write(&self, sample: Sample, pts: i64, duration: i64) -> Result<(), String> {
        unsafe {
            (|| -> windows::core::Result<()> {
                sample.sample.SetSampleTime(pts)?;
                sample.sample.SetSampleDuration(duration)?;
                sample.sample.SetUINT32(&MFSampleExtension_Interlaced, 0)?;
                let buffer = sample.sample.GetBufferByIndex(0)?;
                let length = buffer.GetMaxLength()?;
                buffer.SetCurrentLength(length)?;
                self.writer
                    .WriteSample(self.index, &sample.sample)
                    .map_err(|e| stage(e, "WriteSample NV12"))
            })()
            .map_err(|e| super::native_error("Không mã hóa được khung hình GPU.", e))
        }
    }
    pub fn finish(&self) -> Result<(), String> {
        unsafe {
            self.writer
                .NotifyEndOfSegment(self.index)
                .and_then(|_| self.writer.Finalize())
                .map_err(|e| super::native_error("Không hoàn tất được video đã mã hóa.", e))
        }
    }
}
fn stage(e: windows::core::Error, name: &str) -> windows::core::Error {
    log::error!("MF {name}: {e:?}");
    windows::core::Error::new(e.code(), format!("{name}: {e}"))
}
