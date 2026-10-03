#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Nv12,
    P010,
}

/// Timestamp comes from the decoder, not an assumed frame index/FPS.
pub struct VideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub pts_seconds: f64,
    pub format: PixelFormat,
    pub data: &'a [u8],
}
pub fn byte_len(width: u32, height: u32) -> Result<usize, String> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|_| width > 0 && height > 0)
        .ok_or_else(|| "Kích thước frame không hợp lệ.".into())
}
