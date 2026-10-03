//! Reusable offscreen visual renderer. No project storage, audio, or encoder policy.
pub(crate) mod device;
pub mod frame;
pub mod graph;
pub mod renderer;
pub mod stream;
pub mod text;
mod texture;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderBackend {
    Auto,
    Wgpu,
    Legacy,
}
impl RenderBackend {
    pub fn configured() -> Result<Self, String> {
        match std::env::var("JC_RENDER_BACKEND")
            .unwrap_or_else(|_| "auto".into())
            .as_str()
        {
            "auto" => Ok(Self::Auto),
            "gpu" | "wgpu" => Ok(Self::Wgpu),
            "legacy" | "ffmpegLegacy" => Ok(Self::Legacy),
            _ => Err("Chế độ render không hợp lệ. Chọn auto, wgpu hoặc legacy.".into()),
        }
    }
}
