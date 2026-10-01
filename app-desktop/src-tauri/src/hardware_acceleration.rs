//! Shared video encoding policy. AI acceleration and decoding are independent.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VideoEncoder {
    #[default]
    Auto,
    Cpu,
    Nvidia,
    Intel,
    Amd,
}

impl VideoEncoder {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Tự động",
            Self::Cpu => "CPU · Software",
            Self::Nvidia => "NVIDIA NVENC",
            Self::Intel => "Intel Quick Sync",
            Self::Amd => "AMD AMF",
        }
    }
    pub fn hardware(self) -> bool {
        matches!(self, Self::Nvidia | Self::Intel | Self::Amd)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
}
impl VideoCodec {
    pub fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
        }
    }
}

pub fn ffmpeg_encoder(codec: VideoCodec, backend: VideoEncoder) -> Result<&'static str, String> {
    Ok(match (codec, backend) {
        (VideoCodec::H264, VideoEncoder::Cpu) => "libx264",
        (VideoCodec::Hevc, VideoEncoder::Cpu) => "libx265",
        (VideoCodec::H264, VideoEncoder::Nvidia) => "h264_nvenc",
        (VideoCodec::Hevc, VideoEncoder::Nvidia) => "hevc_nvenc",
        (VideoCodec::H264, VideoEncoder::Intel) => "h264_qsv",
        (VideoCodec::Hevc, VideoEncoder::Intel) => "hevc_qsv",
        (VideoCodec::H264, VideoEncoder::Amd) => "h264_amf",
        (VideoCodec::Hevc, VideoEncoder::Amd) => "hevc_amf",
        (_, VideoEncoder::Auto) => {
            return Err("Cần chọn bộ mã hóa thực tế trước khi chạy FFmpeg.".into())
        }
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCapability {
    pub id: VideoEncoder,
    pub label: String,
    pub vendor: String,
    pub hardware: bool,
    pub h264: bool,
    pub hevc: bool,
    pub h264_message: Option<String>,
    pub hevc_message: Option<String>,
}
impl EncoderCapability {
    pub fn supports(&self, codec: VideoCodec) -> bool {
        match codec {
            VideoCodec::H264 => self.h264,
            VideoCodec::Hevc => self.hevc,
        }
    }
    fn message(&self, codec: VideoCodec) -> Option<&str> {
        match codec {
            VideoCodec::H264 => self.h264_message.as_deref(),
            VideoCodec::Hevc => self.hevc_message.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportCapabilities {
    pub encoders: Vec<EncoderCapability>,
    pub recommended_encoders: HashMap<String, Option<VideoEncoder>>,
}

// Policy is separate from detection, so device preferences/benchmarks can replace it.
pub const DEFAULT_PRIORITY: &[VideoEncoder] = &[
    VideoEncoder::Nvidia,
    VideoEncoder::Intel,
    VideoEncoder::Amd,
    VideoEncoder::Cpu,
];
pub fn resolve_with_priority(
    capabilities: &[EncoderCapability],
    codec: VideoCodec,
    requested: VideoEncoder,
    priority: &[VideoEncoder],
) -> Result<VideoEncoder, String> {
    if requested == VideoEncoder::Auto {
        return priority
            .iter()
            .copied()
            .find(|backend| {
                capabilities
                    .iter()
                    .any(|item| item.id == *backend && item.supports(codec))
            })
            .ok_or_else(|| {
                format!(
                    "Không có bộ mã hóa {} khả dụng. Hãy kiểm tra lại FFmpeg và driver.",
                    codec.label()
                )
            });
    }
    let item = capabilities.iter().find(|item| item.id == requested);
    if item.is_some_and(|item| item.supports(codec)) {
        return Ok(requested);
    }
    Err(item
        .and_then(|item| item.message(codec))
        .map(str::to_owned)
        .unwrap_or_else(|| unavailable_message(requested, codec)))
}

fn unavailable_message(backend: VideoEncoder, codec: VideoCodec) -> String {
    if backend.hardware() {
        format!(
            "{} không khởi tạo được cho {}. Kiểm tra driver/GPU hoặc chọn Tự động/CPU.",
            backend.label(),
            codec.label()
        )
    } else {
        format!(
            "Bộ mã hóa {} trên CPU không khả dụng. Hãy kiểm tra hoặc cài lại ứng dụng.",
            codec.label()
        )
    }
}

struct CachedCapabilities {
    executable: PathBuf,
    value: ExportCapabilities,
}
static CACHE: OnceLock<Mutex<Option<CachedCapabilities>>> = OnceLock::new();
pub fn capabilities(refresh: bool) -> Result<ExportCapabilities, String> {
    let executable = crate::runtime_paths::resolve_ffmpeg()?;
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Không đọc được khả năng mã hóa video.")?;
    if !refresh {
        if let Some(cached) = cache
            .as_ref()
            .filter(|cached| cached.executable == executable)
        {
            return Ok(cached.value.clone());
        }
    }
    let mut encoders = Vec::new();
    for backend in DEFAULT_PRIORITY {
        let h264 = probe_encoder(VideoCodec::H264, *backend);
        let hevc = probe_encoder(VideoCodec::Hevc, *backend);
        encoders.push(EncoderCapability {
            id: *backend,
            label: backend.label().into(),
            vendor: match backend {
                VideoEncoder::Cpu => "software",
                VideoEncoder::Nvidia => "nvidia",
                VideoEncoder::Intel => "intel",
                VideoEncoder::Amd => "amd",
                VideoEncoder::Auto => unreachable!(),
            }
            .into(),
            hardware: backend.hardware(),
            h264: h264.is_ok(),
            hevc: hevc.is_ok(),
            h264_message: h264.err(),
            hevc_message: hevc.err(),
        });
    }
    let recommended_encoders = [VideoCodec::H264, VideoCodec::Hevc]
        .into_iter()
        .map(|codec| {
            (
                codec.name().into(),
                resolve_with_priority(&encoders, codec, VideoEncoder::Auto, DEFAULT_PRIORITY).ok(),
            )
        })
        .collect();
    let value = ExportCapabilities {
        encoders,
        recommended_encoders,
    };
    *cache = Some(CachedCapabilities {
        executable,
        value: value.clone(),
    });
    Ok(value)
}

pub fn resolve(codec: VideoCodec, requested: VideoEncoder) -> Result<VideoEncoder, String> {
    resolve_with_priority(
        &capabilities(false)?.encoders,
        codec,
        requested,
        DEFAULT_PRIORITY,
    )
}

/// Only Auto may try another encoder after an encoder initialization failure.
/// Do not retry a broken filter/input, disk failure, or a failed running encode.
pub fn fallback_candidates(
    codec: VideoCodec,
    requested: VideoEncoder,
    resolved: VideoEncoder,
) -> Result<Vec<VideoEncoder>, String> {
    if requested != VideoEncoder::Auto {
        return Ok(vec![resolved]);
    }
    Ok(retry_candidates(
        &capabilities(false)?.encoders,
        codec,
        resolved,
    ))
}

fn retry_candidates(
    caps: &[EncoderCapability],
    codec: VideoCodec,
    resolved: VideoEncoder,
) -> Vec<VideoEncoder> {
    let mut candidates = vec![resolved];
    candidates.extend(DEFAULT_PRIORITY.iter().copied().filter(|backend| {
        *backend != resolved
            && caps
                .iter()
                .any(|item| item.id == *backend && item.supports(codec))
    }));
    candidates
}
pub fn is_encoder_initialization_error(stderr: &str) -> bool {
    let text = stderr.to_lowercase();
    [
        "error while opening encoder",
        "error opening encoder",
        "cannot load nvcuda",
        "no capable devices",
        "initializeencoder",
        "mfx",
        "amf failed",
        "amfencoder",
        "no nvenc capable",
        "openencodesession",
        "unsupported device",
        "cannot load amfrt",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}
pub fn encoding_error(codec: VideoCodec, backend: VideoEncoder, stderr: &str) -> String {
    log::error!("Video encode failed: backend={backend:?}, codec={codec:?}: {stderr}");
    if is_encoder_initialization_error(stderr) {
        unavailable_message(backend, codec)
    } else {
        "Không thể mã hóa video. Kiểm tra tệp nguồn, vùng hiệu ứng và dung lượng ổ đĩa; xem nhật ký ứng dụng để biết chi tiết.".into()
    }
}

#[derive(Clone, Copy)]
pub enum EncodingProfile {
    Export,
    Merge,
}

/// Encoder-private options must not leak across vendors. Merge keeps its existing quality.
pub fn encoder_arguments(
    codec: VideoCodec,
    backend: VideoEncoder,
    quality: &str,
    bitrate_kbps: Option<u32>,
    profile: EncodingProfile,
) -> Result<Vec<String>, String> {
    let name = ffmpeg_encoder(codec, backend)?;
    let mut args = vec!["-c:v".into(), name.into()];
    let merge = matches!(profile, EncodingProfile::Merge);
    let q = match (codec, quality) {
        (VideoCodec::H264, "fast") => "25",
        (VideoCodec::H264, "high") => "18",
        (VideoCodec::H264, _) => "23",
        (VideoCodec::Hevc, "fast") => "30",
        (VideoCodec::Hevc, "high") => "23",
        (VideoCodec::Hevc, _) => "27",
    };
    let mut add = |pairs: &[&str]| args.extend(pairs.iter().map(|value| (*value).into()));
    match backend {
        VideoEncoder::Cpu => {
            let preset = if merge {
                if codec == VideoCodec::H264 {
                    "veryfast"
                } else {
                    "fast"
                }
            } else {
                match quality {
                    "fast" => "ultrafast",
                    "high" => "slow",
                    _ => "medium",
                }
            };
            add(&["-preset", preset]);
            if bitrate_kbps.is_none() {
                add(&[
                    "-crf",
                    if merge && codec == VideoCodec::H264 {
                        "19"
                    } else {
                        q
                    },
                ]);
            }
        }
        VideoEncoder::Nvidia => {
            let preset = if merge {
                "p5"
            } else {
                match quality {
                    "fast" => "p2",
                    "high" => "p6",
                    _ => "p5",
                }
            };
            add(&["-preset", preset, "-rc:v", "vbr"]);
            if bitrate_kbps.is_none() {
                let cq = if merge {
                    if codec == VideoCodec::H264 {
                        "21"
                    } else {
                        "25"
                    }
                } else {
                    q
                };
                add(&["-cq:v", cq, "-b:v", "0"]);
            }
        }
        VideoEncoder::Intel => {
            add(&[
                "-preset",
                match quality {
                    "fast" => "veryfast",
                    "high" => "slow",
                    _ => "medium",
                },
            ]);
            // QSV uses ICQ when global_quality is set and look-ahead is off.
            if bitrate_kbps.is_none() {
                add(&["-global_quality:v", q]);
                if codec == VideoCodec::H264 {
                    add(&["-look_ahead", "0"]);
                }
            }
        }
        VideoEncoder::Amd => {
            add(&[
                "-quality",
                match quality {
                    "fast" => "speed",
                    "high" => "quality",
                    _ => "balanced",
                },
            ]);
            if bitrate_kbps.is_some() {
                add(&["-rc:v", "vbr_peak"]);
            } else {
                add(&["-rc:v", "cqp", "-qp_i", q, "-qp_p", q]);
            }
        }
        VideoEncoder::Auto => unreachable!(),
    }
    if let Some(bitrate) = bitrate_kbps {
        args.extend([
            "-b:v".into(),
            format!("{bitrate}k"),
            "-maxrate:v".into(),
            format!("{}k", (f64::from(bitrate) * 1.15).round() as u32),
            "-bufsize:v".into(),
            format!("{}k", bitrate.saturating_mul(2)),
        ]);
    }
    args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    if codec == VideoCodec::Hevc {
        args.extend(["-tag:v".into(), "hvc1".into()]);
    }
    Ok(args)
}

fn probe_encoder(codec: VideoCodec, backend: VideoEncoder) -> Result<(), String> {
    let result = (|| {
        let mut command = crate::runtime_paths::ffmpeg_command()?;
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=256x256:r=1:d=1",
                "-frames:v",
                "1",
            ])
            .args(encoder_arguments(
                codec,
                backend,
                "balanced",
                None,
                EncodingProfile::Export,
            )?)
            .args(["-f", "null", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Không đọc được kết quả kiểm tra FFmpeg.")?;
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let reader_tail = tail.clone();
        let reader = std::thread::Builder::new()
            .name("video-encoder-probe".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Ok(mut tail) = reader_tail.lock() {
                        tail.push_back(line);
                        if tail.len() > 16 {
                            tail.pop_front();
                        }
                    }
                }
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                format!("Cannot start encoder probe reader: {error}")
            })?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                other => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(match other {
                        Err(error) => error.to_string(),
                        _ => "Encoder probe timed out (5s)".into(),
                    });
                }
            }
        };
        let _ = reader.join();
        if status?.success() {
            return Ok(());
        }
        Err(tail
            .lock()
            .map(|tail| tail.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default())
    })();
    match result {
        Ok(()) => {
            log::info!("Video encoder probe: {backend:?}/{codec:?} ready");
            Ok(())
        }
        Err(details) => {
            log::info!("Video encoder probe: {backend:?}/{codec:?} unavailable: {details}");
            let lower = details.to_lowercase();
            Err(if lower.contains("timed out") {
                format!(
                    "Kiểm tra {} / {} quá lâu. Hãy kiểm tra lại hoặc chọn CPU.",
                    backend.label(),
                    codec.label()
                )
            } else if lower.contains("unknown encoder") || lower.contains("unrecognized option") {
                format!("FFmpeg không hỗ trợ bộ mã hóa/cấu hình {} / {}. Hãy cập nhật hoặc cài lại ứng dụng.", backend.label(), codec.label())
            } else {
                unavailable_message(backend, codec)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cap(id: VideoEncoder, h264: bool, hevc: bool) -> EncoderCapability {
        EncoderCapability {
            id,
            label: id.label().into(),
            vendor: "test".into(),
            hardware: id.hardware(),
            h264,
            hevc,
            h264_message: None,
            hevc_message: None,
        }
    }
    #[test]
    fn mapping_covers_all_codecs_and_vendors() {
        for (backend, h264, hevc) in [
            (VideoEncoder::Cpu, "libx264", "libx265"),
            (VideoEncoder::Nvidia, "h264_nvenc", "hevc_nvenc"),
            (VideoEncoder::Intel, "h264_qsv", "hevc_qsv"),
            (VideoEncoder::Amd, "h264_amf", "hevc_amf"),
        ] {
            assert_eq!(ffmpeg_encoder(VideoCodec::H264, backend).unwrap(), h264);
            assert_eq!(ffmpeg_encoder(VideoCodec::Hevc, backend).unwrap(), hevc);
        }
        assert!(ffmpeg_encoder(VideoCodec::H264, VideoEncoder::Auto).is_err());
    }
    #[test]
    fn auto_respects_codec_priority_and_software_fallback() {
        let caps = [
            cap(VideoEncoder::Nvidia, false, false),
            cap(VideoEncoder::Intel, true, false),
            cap(VideoEncoder::Amd, true, true),
            cap(VideoEncoder::Cpu, true, true),
        ];
        assert_eq!(
            resolve_with_priority(
                &caps,
                VideoCodec::H264,
                VideoEncoder::Auto,
                DEFAULT_PRIORITY
            )
            .unwrap(),
            VideoEncoder::Intel
        );
        assert_eq!(
            resolve_with_priority(
                &caps,
                VideoCodec::Hevc,
                VideoEncoder::Auto,
                DEFAULT_PRIORITY
            )
            .unwrap(),
            VideoEncoder::Amd
        );
        assert!(resolve_with_priority(
            &caps,
            VideoCodec::Hevc,
            VideoEncoder::Intel,
            DEFAULT_PRIORITY
        )
        .is_err());
        assert_eq!(
            resolve_with_priority(
                &caps[3..],
                VideoCodec::Hevc,
                VideoEncoder::Auto,
                DEFAULT_PRIORITY
            )
            .unwrap(),
            VideoEncoder::Cpu
        );
        assert!(
            resolve_with_priority(&[], VideoCodec::H264, VideoEncoder::Auto, DEFAULT_PRIORITY)
                .is_err()
        );
        assert_eq!(
            resolve_with_priority(
                &caps,
                VideoCodec::H264,
                VideoEncoder::Auto,
                &[VideoEncoder::Amd, VideoEncoder::Intel, VideoEncoder::Cpu]
            )
            .unwrap(),
            VideoEncoder::Amd
        );
    }
    #[test]
    fn legacy_options_and_new_default() {
        for (value, encoder) in [("cpu", VideoEncoder::Cpu), ("nvidia", VideoEncoder::Nvidia)] {
            let options: crate::render_spec::ExportOptions =
                serde_json::from_value(serde_json::json!({"encoder":value})).unwrap();
            assert_eq!(options.encoder, encoder);
        }
        let options: crate::render_spec::ExportOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(options.encoder, VideoEncoder::Auto);
    }
    #[test]
    fn private_quality_and_bitrate_options_do_not_cross_vendors() {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            for backend in DEFAULT_PRIORITY {
                for quality in ["fast", "balanced", "high"] {
                    for bitrate in [None, Some(3000)] {
                        let args = encoder_arguments(
                            codec,
                            *backend,
                            quality,
                            bitrate,
                            EncodingProfile::Export,
                        )
                        .unwrap();
                        assert_eq!(
                            args.contains(&"-crf".into()),
                            *backend == VideoEncoder::Cpu && bitrate.is_none()
                        );
                        assert_eq!(
                            args.contains(&"-cq:v".into()),
                            *backend == VideoEncoder::Nvidia && bitrate.is_none()
                        );
                        assert_eq!(
                            args.contains(&"-global_quality:v".into()),
                            *backend == VideoEncoder::Intel && bitrate.is_none()
                        );
                        assert_eq!(
                            args.contains(&"-qp_i".into()),
                            *backend == VideoEncoder::Amd && bitrate.is_none()
                        );
                        assert_eq!(args.contains(&"hvc1".into()), codec == VideoCodec::Hevc);
                    }
                }
            }
        }
    }
    #[test]
    fn retry_list_is_bounded_and_manual_never_falls_back() {
        let caps = [
            cap(VideoEncoder::Nvidia, true, false),
            cap(VideoEncoder::Intel, true, true),
            cap(VideoEncoder::Amd, false, true),
            cap(VideoEncoder::Cpu, true, true),
        ];
        assert_eq!(
            retry_candidates(&caps, VideoCodec::Hevc, VideoEncoder::Intel),
            vec![VideoEncoder::Intel, VideoEncoder::Amd, VideoEncoder::Cpu]
        );
        assert_eq!(
            fallback_candidates(VideoCodec::Hevc, VideoEncoder::Intel, VideoEncoder::Intel)
                .unwrap(),
            vec![VideoEncoder::Intel]
        );
    }

    #[test]
    fn fallback_is_only_for_encoder_initialization_errors() {
        assert!(is_encoder_initialization_error(
            "Error while opening encoder - maybe incorrect parameters"
        ));
        assert!(!is_encoder_initialization_error("No space left on device"));
        assert!(!is_encoder_initialization_error(
            "Error initializing filter 'ass'"
        ));
    }
    #[test]
    #[ignore = "Requires FFmpeg and probes the actual drivers on this machine"]
    fn runtime_encodes_available_backends_to_valid_mp4() {
        let first = capabilities(true).unwrap();
        let cached = capabilities(false).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&cached).unwrap()
        );
        println!("CAPABILITIES {}", serde_json::to_string(&first).unwrap());
        let directory = tempfile::Builder::new()
            .prefix("jc-ai-mã-hóa-")
            .tempdir()
            .unwrap();
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            assert!(
                first
                    .encoders
                    .iter()
                    .any(|item| item.id == VideoEncoder::Cpu && item.supports(codec)),
                "CPU encoder missing"
            );
            for backend in DEFAULT_PRIORITY {
                if !first
                    .encoders
                    .iter()
                    .any(|item| item.id == *backend && item.supports(codec))
                {
                    continue;
                }
                for profile in [EncodingProfile::Export, EncodingProfile::Merge] {
                    for quality in ["fast", "balanced", "high"] {
                        // Both quality mode and constrained VBR must produce valid MP4 + AAC.
                        for bitrate in [None, Some(1500)] {
                            let output_path = directory.path().join("kết-quả.mp4");
                            let args =
                                encoder_arguments(codec, *backend, quality, bitrate, profile)
                                    .unwrap();
                            let output = crate::runtime_paths::ffmpeg_command()
                                .unwrap()
                                .args([
                                    "-hide_banner",
                                    "-loglevel",
                                    "error",
                                    "-nostdin",
                                    "-y",
                                    "-f",
                                    "lavfi",
                                    "-i",
                                    "color=c=blue:s=256x256:r=24:d=0.25",
                                    "-f",
                                    "lavfi",
                                    "-i",
                                    "sine=frequency=440:sample_rate=48000:duration=0.25",
                                ])
                                .args(args)
                                .args(["-c:a", "aac", "-movflags", "+faststart", "-shortest"])
                                .arg(&output_path)
                                .output()
                                .unwrap();
                            assert!(
                                output.status.success(),
                                "{backend:?}/{codec:?}/{quality}/{bitrate:?}: {}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                            let metadata = crate::runtime_paths::ffprobe_command()
                                .unwrap()
                                .args([
                                    "-v",
                                    "error",
                                    "-show_streams",
                                    "-show_format",
                                    "-of",
                                    "json",
                                ])
                                .arg(&output_path)
                                .output()
                                .unwrap();
                            assert!(metadata.status.success());
                            let json: serde_json::Value =
                                serde_json::from_slice(&metadata.stdout).unwrap();
                            let streams = json["streams"].as_array().unwrap();
                            let video = streams
                                .iter()
                                .find(|item| item["codec_type"] == "video")
                                .unwrap();
                            assert_eq!(video["codec_name"], codec.name());
                            assert_eq!(video["pix_fmt"], "yuv420p");
                            assert_eq!(video["width"], 256);
                            assert_eq!(video["height"], 256);
                            if codec == VideoCodec::Hevc {
                                assert_eq!(video["codec_tag_string"], "hvc1");
                            }
                            assert!(streams.iter().any(|item| item["codec_name"] == "aac"));
                            let duration = json["format"]["duration"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap();
                            assert!((duration - 0.25).abs() < 0.08);
                        }
                    }
                }
                println!("ENCODE {backend:?}/{codec:?}: quality, bitrate, AAC, 4:2:0, MP4, Unicode path verified");
            }
        }
    }
}
