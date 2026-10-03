#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preference {
    Auto,
    ZeroCopy,
    GpuCopy,
    Cpu,
}
impl Preference {
    pub fn configured() -> Result<Self, String> {
        Self::parse(&std::env::var("JC_MEDIA_PIPELINE").unwrap_or_else(|_| "auto".into()))
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "zero-copy" => Ok(Self::ZeroCopy),
            "gpu-copy" => Ok(Self::GpuCopy),
            "cpu" => Ok(Self::Cpu),
            _ => Err("Chế độ xử lý video không hợp lệ.".into()),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Counters {
    pub frames: u64,
    pub upload_bytes: u64,
    pub readback_bytes: u64,
    /// Full-frame GPU transport/conversion passes, excluding ROI visual effects.
    /// Tier B currently has two video-processor conversions and two renderer draws.
    pub gpu_copies: u64,
}

/// Preflight for the native SDR path. Unsupported visual metadata falls back,
/// never silently loses rotation/HDR/aspect ratio. VFR is intentionally allowed.
pub fn preflight(source: &std::path::Path) -> Result<(), String> {
    let output=crate::runtime_paths::ffprobe_command()?.args([
        "-v","error","-select_streams","v:0","-show_entries",
        "stream=codec_name,pix_fmt,color_transfer,sample_aspect_ratio,field_order:stream_side_data=rotation","-of","json"
    ]).arg(source).output().map_err(|_|"Không kiểm tra được định dạng nguồn.")?;
    if !output.status.success() {
        return Err("Không đọc được video nguồn.".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "Metadata nguồn không hợp lệ.")?;
    validate_metadata(&value)
}
fn validate_metadata(value: &serde_json::Value) -> Result<(), String> {
    let v = value["streams"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or("Không có video stream.")?;
    if !matches!(v["codec_name"].as_str(), Some("h264" | "hevc"))
        || !matches!(v["pix_fmt"].as_str(), Some("yuv420p" | "yuvj420p" | "nv12"))
    {
        return Err("Định dạng này cần đường xử lý video tương thích.".into());
    }
    if matches!(
        v["color_transfer"].as_str(),
        Some("smpte2084" | "arib-std-b67")
    ) || v["sample_aspect_ratio"]
        .as_str()
        .is_some_and(|a| !matches!(a, "1:1" | "0:1" | "N/A"))
        || v["field_order"]
            .as_str()
            .is_some_and(|a| !matches!(a, "progressive" | "unknown"))
        || v["side_data_list"].as_array().is_some_and(|a| {
            a.iter()
                .any(|s| s["rotation"].as_f64().is_some_and(|r| r.abs() > 0.1))
        })
    {
        return Err(
            "Video HDR, xoay, xen kẽ hoặc có tỷ lệ pixel đặc biệt cần bộ render tương thích."
                .into(),
        );
    }
    Ok(())
}

pub fn bitrate(
    source: &std::path::Path,
    spec: &crate::render_spec::RenderSpecification,
) -> Result<u32, String> {
    if let Some(kbps) = spec.video_bitrate_kbps {
        return Ok(kbps.saturating_mul(1000));
    }
    let probe = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=bit_rate",
            "-of",
            "json",
        ])
        .arg(source)
        .output()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap_or_default();
    let fps = spec.fps.unwrap_or(30.0).clamp(1.0, 120.0);
    let bpp = match spec.quality.as_str() {
        "high" => 0.16,
        "compact" => 0.065,
        _ => 0.10,
    };
    let estimate = spec.width as f64 * spec.height as f64 * fps * bpp;
    let source_rate = json["streams"][0]["bit_rate"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok());
    let target = source_rate.map_or(estimate, |r| r.max(estimate * 0.6).min(estimate * 1.3));
    let codec = if spec.output.video_codec == crate::hardware_acceleration::VideoCodec::Hevc {
        0.7
    } else {
        1.0
    };
    Ok((target * codec).clamp(500_000.0, 50_000_000.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_modes() {
        for (v, p) in [
            ("auto", Preference::Auto),
            ("zero-copy", Preference::ZeroCopy),
            ("gpu-copy", Preference::GpuCopy),
            ("cpu", Preference::Cpu),
        ] {
            assert_eq!(Preference::parse(v).unwrap(), p);
        }
        assert!(Preference::parse("cuda").is_err());
    }
    #[test]
    fn native_metadata_contract() {
        let mut v = serde_json::json!({"streams":[{"codec_name":"h264","pix_fmt":"yuv420p","avg_frame_rate":"24000/1001","r_frame_rate":"30/1"}]});
        assert!(validate_metadata(&v).is_ok()); // VFR accepted, timing comes from decoded PTS.
        for (key, val) in [
            ("pix_fmt", "yuv420p10le"),
            ("color_transfer", "smpte2084"),
            ("sample_aspect_ratio", "4:3"),
            ("field_order", "tt"),
        ] {
            let mut bad = v.clone();
            bad["streams"][0][key] = val.into();
            assert!(validate_metadata(&bad).is_err());
        }
        v["streams"][0]["side_data_list"] = serde_json::json!([{"rotation":90}]);
        assert!(validate_metadata(&v).is_err());
    }
}
