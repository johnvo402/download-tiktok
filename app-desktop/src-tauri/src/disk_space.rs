//! Conservative preflight only; writes still handle disk-full errors because
//! other applications can consume space after this check.
use std::path::Path;

pub fn require(directory: &Path, estimated_bytes: u64) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = directory
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut available = 0_u64;
        unsafe {
            windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                windows::core::PCWSTR(wide.as_ptr()),
                Some(&mut available),
                None,
                None,
            )
        }
        .map_err(|e| format!("Không kiểm tra được dung lượng ổ đĩa: {e}"))?;
        validate(available, estimated_bytes)?;
    }
    #[cfg(not(windows))]
    let _ = (directory, estimated_bytes);
    Ok(())
}
fn validate(available: u64, estimated: u64) -> Result<(), String> {
    let needed = estimated.saturating_add(64 * 1024 * 1024);
    if available < needed {
        return Err(format!("Ổ đĩa không đủ dung lượng. Cần khoảng {} MB, còn {} MB; hãy giải phóng dung lượng hoặc chọn thư mục khác.",needed.div_ceil(1024*1024),available/(1024*1024)));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insufficient_space_fails_before_processing() {
        assert!(validate(10, 100).is_err());
        assert!(validate(128 * 1024 * 1024, 1024).is_ok());
        assert!(validate(u64::MAX, u64::MAX).is_ok());
    }
}
