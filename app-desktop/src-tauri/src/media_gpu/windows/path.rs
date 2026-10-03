//! MF interprets \\?\ paths as network URLs. ProjectStore canonical paths
//! stay unchanged; only the UTF-16 URL passed to MF is normalized here.
use std::{os::windows::ffi::OsStrExt, path::Path};
pub(super) fn mf_url(path: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.starts_with(&[92, 92, 63, 92]) {
        wide.drain(..4);
        if wide.len() >= 4
            && wide[..4]
                .iter()
                .zip([85, 78, 67, 92])
                .all(|(a, b)| *a == b || (b != 92 && *a == b + 32))
        {
            wide.drain(..4);
            wide.splice(..0, [92, 92]);
        }
    }
    wide.push(0);
    wide
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_unicode_and_unc_urls() {
        for (a, b) in [
            (r"\\?\D:\字幕\video.mp4", r"D:\字幕\video.mp4"),
            (r"\\?\UNC\server\共享\v.mp4", r"\\server\共享\v.mp4"),
            (r"D:\plain.mp4", r"D:\plain.mp4"),
        ] {
            assert_eq!(
                mf_url(Path::new(a)),
                b.encode_utf16().chain(Some(0)).collect::<Vec<_>>()
            );
        }
    }
}
