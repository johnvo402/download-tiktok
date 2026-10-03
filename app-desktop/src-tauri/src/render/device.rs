use std::sync::{Arc, Mutex};

pub struct GpuDevice {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub error: Arc<Mutex<Option<String>>>,
}
pub fn create_preferred(vendor: Option<u32>) -> Result<GpuDevice, String> {
    // Windows prefers DX12; portable Vulkan/Metal follow. No CUDA/DXC SDK install.
    for backends in [
        wgpu::Backends::DX12,
        wgpu::Backends::VULKAN | wgpu::Backends::METAL,
    ] {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let selected = if let Some(vendor) = vendor {
            instance
                .enumerate_adapters(backends)
                .into_iter()
                .find(|a| {
                    a.get_info().vendor == vendor
                        && a.get_info().device_type != wgpu::DeviceType::Cpu
                })
                .ok_or(())
        } else {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            }))
            .map_err(|_| ())
        };
        let Ok(adapter) = selected else {
            continue;
        };
        let info = adapter.get_info();
        if matches!(info.device_type, wgpu::DeviceType::Cpu) {
            continue;
        }
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("JC AI visuals"),
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                ..Default::default()
            }))
        else {
            continue;
        };
        let error = Arc::new(Mutex::new(None));
        let receiver = error.clone();
        device.on_uncaptured_error(Arc::new(move |cause| {
            log::error!("wgpu uncaptured error: {cause}");
            if let Ok(mut slot) = receiver.lock() {
                *slot = Some(
                    "GPU gặp lỗi khi render. Hãy thử lại hoặc dùng chế độ tương thích.".into(),
                );
            }
        }));
        let lost = error.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason == wgpu::DeviceLostReason::Destroyed {
                return;
            }
            log::error!("wgpu device lost: {reason:?}; {message}");
            if let Ok(mut slot) = lost.lock() {
                *slot = Some(
                    "Mất kết nối với GPU khi render. Hãy thử lại hoặc dùng chế độ tương thích."
                        .into(),
                );
            }
        });
        log::info!("Visual renderer: wgpu; adapter={}; backend={:?}; type={:?}; driver={} {}; format=RGBA8 SDR; max_texture={}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info, device.limits().max_texture_dimension_2d);
        return Ok(GpuDevice {
            device,
            queue,
            info,
            error,
        });
    }
    Err(
        "Không khởi tạo được bộ render GPU. Kiểm tra driver đồ họa hoặc dùng chế độ tương thích."
            .into(),
    )
}
