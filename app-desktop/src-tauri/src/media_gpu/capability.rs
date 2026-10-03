use crate::hardware_acceleration::VideoEncoder;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    ZeroCopy,
    GpuCopy,
    CpuStaging,
}
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub hardware_decode: bool,
    pub gpu_render: bool,
    pub shared_surfaces: bool,
    pub hardware_encode: bool,
    pub direct_surfaces: bool,
    pub same_adapter: bool,
}
pub fn resolve(c: &Capabilities) -> Transport {
    if c.hardware_decode && c.gpu_render && c.hardware_encode && c.same_adapter {
        if c.direct_surfaces {
            return Transport::ZeroCopy;
        }
        if c.shared_surfaces {
            return Transport::GpuCopy;
        }
    }
    Transport::CpuStaging
}
pub fn vendor(encoder: VideoEncoder) -> Option<u32> {
    match encoder {
        VideoEncoder::Intel => Some(0x8086),
        VideoEncoder::Nvidia => Some(0x10de),
        VideoEncoder::Amd => Some(0x1002),
        _ => None,
    }
}
pub fn select(
    preference: super::pipeline::Preference,
    available: Transport,
) -> Result<Transport, String> {
    use super::pipeline::Preference;
    match preference {
        Preference::Auto => Ok(available),
        Preference::Cpu => Ok(Transport::CpuStaging),
        Preference::ZeroCopy if available == Transport::ZeroCopy => Ok(available),
        Preference::GpuCopy if available != Transport::CpuStaging => Ok(Transport::GpuCopy),
        _ => Err("Thiết bị chưa hỗ trợ chế độ xử lý video đã yêu cầu. Hãy chọn Auto.".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiers_and_hybrid_adapter() {
        let mut c = Capabilities {
            hardware_decode: true,
            gpu_render: true,
            shared_surfaces: true,
            hardware_encode: true,
            direct_surfaces: true,
            same_adapter: true,
        };
        assert_eq!(resolve(&c), Transport::ZeroCopy);
        c.direct_surfaces = false;
        assert_eq!(resolve(&c), Transport::GpuCopy);
        c.same_adapter = false;
        assert_eq!(resolve(&c), Transport::CpuStaging);
        c.same_adapter = true;
        c.hardware_decode = false;
        assert_eq!(resolve(&c), Transport::CpuStaging);
        assert_eq!(vendor(VideoEncoder::Intel), Some(0x8086));
        assert_eq!(vendor(VideoEncoder::Nvidia), Some(0x10de));
        assert_eq!(vendor(VideoEncoder::Amd), Some(0x1002));
        assert_eq!(vendor(VideoEncoder::Cpu), None);
        assert!(select(
            super::super::pipeline::Preference::ZeroCopy,
            Transport::GpuCopy
        )
        .is_err());
        assert!(select(
            super::super::pipeline::Preference::GpuCopy,
            Transport::CpuStaging
        )
        .is_err());
        assert_eq!(
            select(
                super::super::pipeline::Preference::Auto,
                Transport::CpuStaging
            )
            .unwrap(),
            Transport::CpuStaging
        );
    }
}
