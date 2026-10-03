#![cfg(windows)]
use super::Bounds;
use windows::{
    core::w,
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
            WindowsAndMessaging::*,
        },
    },
};
type PointerSink = Box<dyn Fn(&str, f64, f64, u32)>;
struct Context {
    pointer: PointerSink,
    drop_files: Box<dyn Fn(Vec<String>)>,
    dpr: f64,
}
// Also destroys HWND on any early wgpu/capabilities error, before Context is freed.
struct WindowOwner(HWND);
impl Drop for WindowOwner {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let context = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context;
    if msg == WM_DROPFILES {
        let drop = HDROP(wp.0 as *mut _);
        let mut paths = Vec::new();
        if !context.is_null() {
            for index in 0..DragQueryFileW(drop, u32::MAX, None).min(128) {
                let length = DragQueryFileW(drop, index, None);
                if length > 0 && length < 32768 {
                    let mut name = vec![0; length as usize + 1];
                    DragQueryFileW(drop, index, Some(&mut name));
                    paths.push(String::from_utf16_lossy(&name[..length as usize]));
                }
            }
            (&*context).drop_files.as_ref()(paths);
        }
        DragFinish(drop);
        return LRESULT(0);
    }
    if !context.is_null() && matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE) {
        let c = &*context;
        if msg == WM_LBUTTONDOWN {
            SetCapture(hwnd);
        } else if msg == WM_LBUTTONUP {
            let _ = ReleaseCapture();
        }
        let kind = if msg == WM_LBUTTONDOWN {
            "pointerdown"
        } else if msg == WM_LBUTTONUP {
            "pointerup"
        } else {
            "pointermove"
        };
        let x = (lp.0 as u16 as i16) as f64 / c.dpr;
        let y = ((lp.0 >> 16) as u16 as i16) as f64 / c.dpr;
        (c.pointer)(kind, x, y, if wp.0 & 1 != 0 { 1 } else { 0 });
        return LRESULT(0);
    }
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
pub struct NativeSurface {
    pub surface: std::mem::ManuallyDrop<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    hwnd: HWND,
    context: Box<Context>,
}
impl NativeSurface {
    pub fn new(
        parent: isize,
        pointer: PointerSink,
        drop_files: Box<dyn Fn(Vec<String>)>,
        gpu: &crate::render::device::GpuDevice,
        bounds: Bounds,
    ) -> Result<Self, String> {
        unsafe {
            let module = GetModuleHandleW(None).map_err(|_| "Không tạo được vùng preview.")?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: HINSTANCE(module.0),
                lpszClassName: w!("JC_AI_GPU_PREVIEW"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let mut context = Box::new(Context {
                pointer,
                drop_files,
                dpr: bounds.dpr,
            });
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("JC_AI_GPU_PREVIEW"),
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                HWND(parent as *mut _),
                None,
                HINSTANCE(module.0),
                Some((&mut *context as *mut Context).cast()),
            )
            .map_err(|e| format!("Không tạo được vùng preview: {e}"))?;
            let window = WindowOwner(hwnd);
            DragAcceptFiles(hwnd, true);
            let handle = raw_window_handle::Win32WindowHandle::new(
                std::num::NonZeroIsize::new(hwnd.0 as isize)
                    .ok_or("Preview handle không hợp lệ.")?,
            );
            let surface = gpu
                .instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: raw_window_handle::RawDisplayHandle::Windows(
                        raw_window_handle::WindowsDisplayHandle::new(),
                    ),
                    raw_window_handle: raw_window_handle::RawWindowHandle::Win32(handle),
                })
                .map_err(|e| format!("Không tạo được surface: {e}"))?;
            let caps = surface.get_capabilities(&gpu.adapter);
            if !caps.formats.contains(&wgpu::TextureFormat::Bgra8Unorm) {
                return Err("Surface chưa hỗ trợ màu SDR.".into());
            }
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: wgpu::TextureFormat::Bgra8Unorm,
                width: 1,
                height: 1,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
            };
            std::mem::forget(window);
            let mut value = Self {
                surface: std::mem::ManuallyDrop::new(surface),
                config,
                hwnd,
                context,
            };
            value.resize(gpu, bounds)?;
            Ok(value)
        }
    }
    pub fn resize(
        &mut self,
        gpu: &crate::render::device::GpuDevice,
        b: Bounds,
    ) -> Result<(), String> {
        b.validate()?;
        self.context.dpr = b.dpr;
        let width = (b.width * b.dpr).round().max(1.0) as u32;
        let height = (b.height * b.dpr).round().max(1.0) as u32;
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                (b.x * b.dpr).round() as i32,
                (b.y * b.dpr).round() as i32,
                width as i32,
                height as i32,
                SWP_NOACTIVATE,
            )
            .map_err(|_| "Không cập nhật được vị trí preview.")?;
            let _ = ShowWindow(self.hwnd, if b.visible { SW_SHOWNA } else { SW_HIDE });
        }
        if width != self.config.width || height != self.config.height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&gpu.device, &self.config);
        }
        Ok(())
    }
    pub fn pump(&self) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
impl Drop for NativeSurface {
    fn drop(&mut self) {
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self.surface);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
