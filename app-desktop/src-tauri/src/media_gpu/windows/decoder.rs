use super::Session;
use std::path::Path;
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Graphics::{Direct3D11::*, Dxgi::Common::*},
        Media::MediaFoundation::*,
    },
};
pub struct Decoder {
    reader: IMFSourceReader,
    pub width: u32,
    pub height: u32,
    pub fps: u64,
    pub matrix: crate::media_gpu::frame::Matrix,
    pub range: crate::media_gpu::frame::Range,
    first: Option<i64>,
    ended: bool,
    device: ID3D11Device,
    adapter: crate::media_gpu::frame::AdapterId,
    _lifecycle: std::rc::Rc<super::device::Lifecycle>,
}
pub struct Frame {
    pub(crate) _sample: IMFSample,
    pub(crate) texture: ID3D11Texture2D,
    pub(crate) subresource: u32,
    pub metadata: crate::media_gpu::frame::FrameMetadata,
    _lifecycle: std::rc::Rc<super::device::Lifecycle>,
}
impl Decoder {
    pub fn new(session: &Session, path: &Path) -> Result<Self, String> {
        unsafe {
            Self::open(session, path).map_err(|e| {
                log::error!("MF decoder init: {e:?}");
                "Bộ giải mã video trên GPU chưa sẵn sàng. Hãy thử chế độ tương thích.".into()
            })
        }
    }
    unsafe fn open(session: &Session, path: &Path) -> windows::core::Result<Self> {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 4)?;
        let a = attributes.unwrap();
        a.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &session.manager)?;
        a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        a.SetUINT32(&MF_SOURCE_READER_DISABLE_DXVA, 0)?;
        let wide = super::path::mf_url(path);
        let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &a)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, true)?;
        let ty = MFCreateMediaType()?;
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        ty.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &ty)?;
        let ty = reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)?;
        let (width, height) = visible_size(&ty)?;
        let (matrix, range) = color_info(&ty, height);
        Ok(Self {
            reader,
            width,
            height,
            fps: ty.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or((30u64 << 32) | 1),
            matrix,
            range,
            first: None,
            ended: false,
            device: session.device.clone(),
            adapter: session.adapter,
            _lifecycle: session.lifecycle.clone(),
        })
    }
    pub fn seek(&mut self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || seconds < 0.0 { return Err("Timestamp seek không hợp lệ.".into()); }
        let time = (seconds * 10_000_000.0) as i64 + self.first.unwrap_or(0);
        unsafe {
            self.reader.Flush(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)
                .and_then(|_| self.reader.SetCurrentPosition(&windows::core::GUID::zeroed(), &windows::core::PROPVARIANT::from(time)))
                .map_err(|e| super::native_error("Không seek được video.", e))?;
        }
        self.ended = false;
        Ok(())
    }
    pub fn next(&mut self) -> Result<Option<Frame>, String> {
        unsafe {
            self.read().map_err(|e| {
                log::error!("MF decode: {e:?}");
                "Không giải mã được video bằng GPU. Hãy thử chế độ tương thích.".into()
            })
        }
    }
    unsafe fn read(&mut self) -> windows::core::Result<Option<Frame>> {
        if self.ended {
            return Ok(None);
        }
        loop {
            let (mut flags, mut pts, mut sample) = (0, 0, None);
            self.reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                Some(&mut pts),
                Some(&mut sample),
            )?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                self.ended = true;
                if sample.is_none() {
                    return Ok(None);
                }
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                let ty = self
                    .reader
                    .GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)?;
                let (width, height) = visible_size(&ty)?;
                if width != self.width
                    || height != self.height
                    || ty.GetGUID(&MF_MT_SUBTYPE)? != MFVideoFormat_NV12
                {
                    return Err(windows::core::Error::new(
                        MF_E_INVALIDMEDIATYPE,
                        format!(
                            "Dynamic video dimensions {width}x{height}, expected {}x{}",
                            self.width, self.height
                        ),
                    ));
                }
                let (matrix, range) = color_info(&ty, self.height);
                if self.first.is_some() && (matrix != self.matrix || range != self.range) {
                    return Err(windows::core::Error::from_hresult(MF_E_INVALIDMEDIATYPE));
                }
                self.matrix = matrix;
                self.range = range;
            }
            let Some(sample) = sample else { continue };
            let buffer = sample.GetBufferByIndex(0)?.cast::<IMFDXGIBuffer>()?;
            let mut texture = None;
            buffer.GetResource(&ID3D11Texture2D::IID, &mut texture as *mut _ as *mut _)?;
            let texture: ID3D11Texture2D = texture
                .ok_or_else(|| windows::core::Error::from_hresult(MF_E_UNSUPPORTED_D3D_TYPE))?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let subresource = buffer.GetSubresourceIndex()?;
            if texture.GetDevice()?.as_raw() != self.device.as_raw()
                || desc.BindFlags & D3D11_BIND_DECODER.0 as u32 == 0
                || desc.Format != DXGI_FORMAT_NV12
                || desc.Width < self.width
                || desc.Height < self.height
                || desc.MipLevels != 1
                || desc.SampleDesc.Count != 1
                || subresource >= desc.ArraySize
            {
                return Err(windows::core::Error::from_hresult(
                    MF_E_UNSUPPORTED_D3D_TYPE,
                ));
            }
            let first = *self.first.get_or_insert(pts);
            let duration = sample.GetSampleDuration().unwrap_or(0);
            return Ok(Some(Frame {
                _sample: sample,
                texture,
                subresource,
                metadata: crate::media_gpu::frame::FrameMetadata {
                    width: self.width,
                    height: self.height,
                    pts_100ns: pts - first,
                    duration_100ns: duration,
                    format: crate::media_gpu::frame::SurfaceFormat::Nv12,
                    adapter: self.adapter,
                    matrix: self.matrix,
                    range: self.range,
                },
                _lifecycle: self._lifecycle.clone(),
            }));
        }
    }
}
unsafe fn visible_size(ty: &IMFMediaType) -> windows::core::Result<(u32, u32)> {
    let size = ty.GetUINT64(&MF_MT_FRAME_SIZE)?;
    let coded = ((size >> 32) as u32, size as u32);
    for key in [MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_GEOMETRIC_APERTURE] {
        if ty.GetBlobSize(&key).ok() == Some(std::mem::size_of::<MFVideoArea>() as u32) {
            let mut blob = vec![0; std::mem::size_of::<MFVideoArea>()];
            ty.GetBlob(&key, &mut blob, None)?;
            // SAFETY: MFVideoArea is the documented POD ABI for these attributes;
            // read_unaligned does not assume alignment of the byte vector.
            let area = std::ptr::read_unaligned(blob.as_ptr().cast::<MFVideoArea>());
            if area.OffsetX.value != 0
                || area.OffsetX.fract != 0
                || area.OffsetY.value != 0
                || area.OffsetY.fract != 0
                || area.Area.cx <= 0
                || area.Area.cy <= 0
                || area.Area.cx as u32 > coded.0
                || area.Area.cy as u32 > coded.1
            {
                return Err(windows::core::Error::from_hresult(MF_E_INVALIDMEDIATYPE));
            }
            return Ok((area.Area.cx as u32, area.Area.cy as u32));
        }
    }
    Ok(coded)
}
unsafe fn color_info(
    ty: &IMFMediaType,
    height: u32,
) -> (
    crate::media_gpu::frame::Matrix,
    crate::media_gpu::frame::Range,
) {
    use crate::media_gpu::frame::{Matrix, Range};
    let matrix = match ty.GetUINT32(&MF_MT_YUV_MATRIX).unwrap_or(0) {
        1 => Matrix::Bt709,
        2 => Matrix::Bt601,
        4 | 5 => Matrix::Bt2020,
        _ => {
            if height >= 720 {
                Matrix::Bt709
            } else {
                Matrix::Bt601
            }
        }
    };
    let range = if ty.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE).unwrap_or(2) == 1 {
        Range::Full
    } else {
        Range::Limited
    };
    (matrix, range)
}
