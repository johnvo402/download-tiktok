//! Public wgpu-hal DX12 import, never transmute wgpu/COM layouts.
//! All devices use one LUID; shared textures return to COMMON before D3D11 access.
use super::device::Session;
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::{Duration, Instant},
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{CloseHandle, GENERIC_ALL, HANDLE},
        Graphics::{Direct3D11::*, Direct3D12::*, Dxgi::Common::*, Dxgi::*},
    },
};
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub struct SharedTexture {
    pub(crate) native: ID3D11Texture2D,
    pub texture: wgpu::Texture,
}
impl SharedTexture {
    pub fn new(
        session: &Session,
        gpu: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        // SAFETY: same LUID, owned COM handles and a matching one-mip BGRA descriptor.
        unsafe {
            (|| -> windows::core::Result<Self> {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
                    MiscFlags: D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 as u32
                        | D3D11_RESOURCE_MISC_SHARED.0 as u32,
                    ..Default::default()
                };
                let mut native = None;
                session
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut native))?;
                let native = native.unwrap();
                // create_texture_from_hal requires initialized memory. Clear once on GPU;
                // the first before_render fence also orders this write before DX12 access.
                let mut rtv = None;
                session
                    .device
                    .CreateRenderTargetView(&native, None, Some(&mut rtv))?;
                session
                    .context
                    .ClearRenderTargetView(&rtv.unwrap(), &[0.0, 0.0, 0.0, 1.0]);
                let handle = OwnedHandle(native.cast::<IDXGIResource1>()?.CreateSharedHandle(
                    None,
                    DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
                    None,
                )?);
                let mut resource: Option<ID3D12Resource> = None;
                session.dx12.OpenSharedHandle(handle.0, &mut resource)?;
                let size = wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                };
                let hal = wgpu::hal::dx12::Device::texture_from_raw(
                    resource.unwrap(),
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::TextureDimension::D2,
                    size,
                    1,
                    1,
                );
                let texture = gpu.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("shared video surface"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    },
                );
                Ok(Self { native, texture })
            })()
            .map_err(|e| {
                log::warn!("Shared video texture: {e:?}");
                "Không chia sẻ được khung hình video trên GPU.".into()
            })
        }
    }
}
pub struct Synchronizer {
    fence: ID3D12Fence,
    shared: ID3D11Fence,
    context: ID3D11DeviceContext4,
    queue: ID3D12CommandQueue,
    value: u64,
}
impl Synchronizer {
    pub fn new(session: &Session) -> Result<Self, String> {
        unsafe {
            (|| -> windows::core::Result<Self> {
                let fence: ID3D12Fence = session.dx12.CreateFence(0, D3D12_FENCE_FLAG_SHARED)?;
                let handle = OwnedHandle(session.dx12.CreateSharedHandle(
                    &fence,
                    None,
                    GENERIC_ALL.0,
                    None,
                )?);
                let mut shared = None;
                session
                    .device
                    .cast::<ID3D11Device5>()?
                    .OpenSharedFence(handle.0, &mut shared)?;
                Ok(Self {
                    fence,
                    shared: shared.unwrap(),
                    context: session.context.cast()?,
                    queue: session.queue.clone(),
                    value: 0,
                })
            })()
            .map_err(|e| super::native_error("Không đồng bộ được các bộ xử lý video.", e))
        }
    }
    pub fn before_render(&mut self) -> Result<(), String> {
        unsafe {
            self.value += 1;
            self.context.Signal(&self.shared, self.value).map_err(|e| {
                super::native_error("Không đồng bộ được khung hình video trên GPU.", e)
            })?;
            self.context.Flush();
            self.queue.Wait(&self.fence, self.value).map_err(|e| {
                super::native_error("Không đồng bộ được khung hình video trên GPU.", e)
            })
        }
    }
    pub fn after_render(&mut self) -> Result<(), String> {
        unsafe {
            self.value += 1;
            self.queue.Signal(&self.fence, self.value).map_err(|e| {
                super::native_error("Không đồng bộ được khung hình video trên GPU.", e)
            })?;
            self.context.Wait(&self.shared, self.value).map_err(|e| {
                super::native_error("Không đồng bộ được khung hình video trên GPU.", e)
            })
        }
    }
    pub fn after_conversion(&mut self) -> Result<u64, String> {
        unsafe {
            self.value += 1;
            self.context.Signal(&self.shared, self.value).map_err(|e| {
                super::native_error("Không đồng bộ được khung hình video trên GPU.", e)
            })?;
            self.context.Flush();
            Ok(self.value)
        }
    }
    pub fn wait_slot(&self, value: u64, state: &AtomicU8) -> Result<(), String> {
        let start = Instant::now();
        loop {
            let completed = unsafe { self.fence.GetCompletedValue() };
            if completed == u64::MAX {
                return Err("Mất kết nối với GPU khi xử lý video.".into());
            }
            if completed >= value {
                break;
            }
            if state.load(Ordering::SeqCst) == 2 {
                return Err("Đã hủy export.".into());
            }
            if start.elapsed() > Duration::from_secs(15) {
                return Err("GPU không hoàn thành xử lý khung hình.".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
    /// Drain once at teardown, including submissions which failed before a slot
    /// received its final fence. Not a per-frame GPU-idle barrier.
    pub fn drain(&mut self) -> Result<(), String> {
        self.after_render()?;
        let value = self.after_conversion()?;
        self.wait_slot(value, &AtomicU8::new(0))
    }
}
