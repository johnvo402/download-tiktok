use crate::media_gpu::frame::AdapterId;
use windows::{
    core::Interface,
    Win32::{
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN, Direct3D10::ID3D10Multithread, Direct3D11::*,
            Direct3D12::*, Dxgi::*,
        },
        Media::MediaFoundation::*,
        System::Com::*,
    },
};

pub struct Session {
    pub(crate) device: ID3D11Device,
    pub(crate) context: ID3D11DeviceContext,
    pub(crate) manager: IMFDXGIDeviceManager,
    pub(crate) dx12: ID3D12Device,
    pub(crate) queue: ID3D12CommandQueue,
    pub adapter: AdapterId,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
    pub(crate) lifecycle: std::rc::Rc<Lifecycle>,
}
impl Session {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        unsafe {
            Self::create(device).map_err(|e| {
                log::error!("Native media session: {e:?}");
                "Không khởi tạo được tăng tốc video trên thiết bị đồ họa.".into()
            })
        }
    }
    // SAFETY: one MTA/MF lifecycle per session on its worker; COM clones own references.
    unsafe fn create(gpu: &wgpu::Device) -> windows::core::Result<Self> {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            CoUninitialize();
            return Err(e);
        }
        let result = (|| {
            let raw = gpu.as_hal::<wgpu::hal::api::Dx12>().ok_or_else(|| {
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_NOINTERFACE)
            })?;
            let dx12 = raw.raw_device().clone();
            let queue = raw.raw_queue().clone();
            let luid = dx12.GetAdapterLuid();
            let factory: IDXGIFactory4 = CreateDXGIFactory1()?;
            let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(luid)?;
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                None,
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = device.unwrap();
            let context = context.unwrap();
            let _ = device
                .cast::<ID3D10Multithread>()?
                .SetMultithreadProtected(true);
            let mut manager = None;
            let mut token = 0;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.unwrap();
            manager.ResetDevice(&device, token)?;
            Ok(Self {
                device,
                context,
                manager,
                dx12,
                queue,
                adapter: AdapterId {
                    high: luid.HighPart,
                    low: luid.LowPart,
                },
                _thread: std::marker::PhantomData,
                lifecycle: std::rc::Rc::new(Lifecycle),
            })
        })();
        if result.is_err() {
            let _ = MFShutdown();
            CoUninitialize();
        }
        result
    }
}
pub(crate) struct Lifecycle;
impl Drop for Lifecycle {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
