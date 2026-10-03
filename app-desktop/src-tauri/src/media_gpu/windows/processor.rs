//! NV12 <-> BGRA conversion/scaling exclusively on the D3D11 video processor.
use super::{decoder::Frame, device::Session};
use crate::media_gpu::frame::{Matrix, Range};
use std::{collections::HashMap, mem::ManuallyDrop};
use windows::{
    core::Interface,
    Win32::{
        Foundation::RECT,
        Graphics::{Direct3D11::*, Dxgi::Common::*},
    },
};
pub struct Processor {
    device: ID3D11VideoDevice,
    context: ID3D11VideoContext1,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    inputs: HashMap<(usize, u32), ID3D11VideoProcessorInputView>,
    outputs: HashMap<(usize, u32), ID3D11VideoProcessorOutputView>,
}
impl Processor {
    pub fn new(
        s: &Session,
        src: (u32, u32),
        dst: (u32, u32),
        matrix: Matrix,
        range: Range,
        to_rgb: bool,
    ) -> Result<Self, String> {
        unsafe {
            (|| -> windows::core::Result<Self> {
                let device: ID3D11VideoDevice = s.device.cast()?;
                let context: ID3D11VideoContext1 = s.context.cast()?;
                let enumerator =
                    device.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                        InputFrameRate: DXGI_RATIONAL {
                            Numerator: 30,
                            Denominator: 1,
                        },
                        InputWidth: src.0,
                        InputHeight: src.1,
                        OutputFrameRate: DXGI_RATIONAL {
                            Numerator: 30,
                            Denominator: 1,
                        },
                        OutputWidth: dst.0,
                        OutputHeight: dst.1,
                        Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
                    })?;
                let processor = device.CreateVideoProcessor(&enumerator, 0)?;
                let source = RECT {
                    left: 0,
                    top: 0,
                    right: src.0 as i32,
                    bottom: src.1 as i32,
                };
                let dest = RECT {
                    left: 0,
                    top: 0,
                    right: dst.0 as i32,
                    bottom: dst.1 as i32,
                };
                context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&source));
                context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dest));
                context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dest));
                context.VideoProcessorSetStreamFrameFormat(
                    &processor,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
                let yuv = match (matrix, range) {
                    (Matrix::Bt601, Range::Full) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601,
                    (Matrix::Bt601, Range::Limited) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
                    (Matrix::Bt709, Range::Full) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
                    (Matrix::Bt709, Range::Limited) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                    (Matrix::Bt2020, Range::Full) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P2020,
                    (Matrix::Bt2020, Range::Limited) => {
                        DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P2020
                    }
                };
                let rgb = DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709;
                let e1: ID3D11VideoProcessorEnumerator1 = enumerator.cast()?;
                let (inf, inc, outf, outc) = if to_rgb {
                    (DXGI_FORMAT_NV12, yuv, DXGI_FORMAT_B8G8R8A8_UNORM, rgb)
                } else {
                    (DXGI_FORMAT_B8G8R8A8_UNORM, rgb, DXGI_FORMAT_NV12, yuv)
                };
                if !e1
                    .CheckVideoProcessorFormatConversion(inf, inc, outf, outc)?
                    .as_bool()
                {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_NOTIMPL,
                    ));
                }
                context.VideoProcessorSetStreamColorSpace1(&processor, 0, inc);
                context.VideoProcessorSetOutputColorSpace1(&processor, outc);
                Ok(Self {
                    device,
                    context,
                    enumerator,
                    processor,
                    inputs: HashMap::new(),
                    outputs: HashMap::new(),
                })
            })()
            .map_err(|e| super::native_error("GPU không hỗ trợ chuyển đổi màu video này.", e))
        }
    }
    pub fn decode(&mut self, frame: &Frame, target: &ID3D11Texture2D) -> Result<(), String> {
        self.convert(&frame.texture, frame.subresource, target, 0)
    }
    pub fn convert(
        &mut self,
        src: &ID3D11Texture2D,
        slice: u32,
        dst: &ID3D11Texture2D,
        dst_slice: u32,
    ) -> Result<(), String> {
        unsafe {
            (|| -> windows::core::Result<()> {
                let key = (src.as_raw() as usize, slice);
                let out = (dst.as_raw() as usize, dst_slice);
                if !self.inputs.contains_key(&key) {
                    if self.inputs.len() >= 64 {
                        self.inputs.clear();
                    }
                    let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                        FourCC: 0,
                        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_VPIV {
                                MipSlice: 0,
                                ArraySlice: slice,
                            },
                        },
                    };
                    let mut view = None;
                    self.device.CreateVideoProcessorInputView(
                        src,
                        &self.enumerator,
                        &desc,
                        Some(&mut view),
                    )?;
                    self.inputs.insert(key, view.unwrap());
                }
                if !self.outputs.contains_key(&out) {
                    if self.outputs.len() >= 8 {
                        self.outputs.clear();
                    }
                    let mut td = D3D11_TEXTURE2D_DESC::default();
                    dst.GetDesc(&mut td);
                    let desc = if td.ArraySize > 1 {
                        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                                Texture2DArray: D3D11_TEX2D_ARRAY_VPOV {
                                    MipSlice: 0,
                                    FirstArraySlice: dst_slice,
                                    ArraySize: 1,
                                },
                            },
                        }
                    } else {
                        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                            },
                        }
                    };
                    let mut view = None;
                    self.device.CreateVideoProcessorOutputView(
                        dst,
                        &self.enumerator,
                        &desc,
                        Some(&mut view),
                    )?;
                    self.outputs.insert(out, view.unwrap());
                }
                let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                    Enable: true.into(),
                    pInputSurface: ManuallyDrop::new(Some(self.inputs[&key].clone())),
                    ..Default::default()
                };
                let result = self.context.VideoProcessorBlt(
                    &self.processor,
                    &self.outputs[&out],
                    0,
                    std::slice::from_ref(&stream),
                );
                ManuallyDrop::drop(&mut stream.pInputSurface);
                result
            })()
            .map_err(|e| super::native_error("Không chuyển đổi được khung hình video trên GPU.", e))
        }
    }
}
