use super::*;
use crate::{hardware_acceleration::VideoCodec, render::graph::RenderScene};
use std::{
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
};
fn probe(path: &Path) -> serde_json::Value {
    let out = crate::runtime_paths::ffprobe_command()
        .unwrap()
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_frames",
            "-show_entries",
            "frame=best_effort_timestamp_time",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}
fn times(path: &Path) -> Vec<f64> {
    probe(path)["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            f["best_effort_timestamp_time"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect()
}
#[test]
#[ignore = "Requires Windows hardware decoder/encoder, FFmpeg and actual DX12 adapter"]
fn native_gpu_vfr_bframes_nonzero_pts_cancel_and_cleanup() {
    let dir = tempfile::Builder::new()
        .prefix("native-字幕-")
        .tempdir()
        .unwrap();
    let source = dir.path().join("nguồn-VFR.mp4");
    let output = dir.path().join("output.mp4");
    let fixture = crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=30:d=2",
            "-vf",
            "select='not(mod(n,3))+not(mod(n,5))',setpts=PTS+2/TB",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-bf",
            "3",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(fixture.status.success());
    crate::media_gpu::pipeline::preflight(&source).unwrap();
    let original = times(&source);
    assert!(original[0] > 1.9);
    assert!(original
        .windows(3)
        .any(|s| ((s[1] - s[0]) - (s[2] - s[1])).abs() > 0.01));
    let mut p = NativePipeline::new(
        &source,
        &output,
        RenderScene::empty(320, 240),
        &[],
        VideoCodec::H264,
        1_500_000,
        Some(0x8086),
    )
    .unwrap();
    let stats = p.run(&AtomicU8::new(0), |_, _| {}).unwrap();
    drop(p);
    assert_eq!(stats.upload_bytes + stats.readback_bytes, 0);
    assert_eq!(stats.frames, original.len() as u64);
    let encoded = times(&output);
    assert_eq!(encoded.len(), original.len());
    for (a, b) in original.iter().zip(encoded) {
        assert!((a - original[0] - b).abs() < 0.002, "PTS drift: {a} vs {b}");
    }
    let cancelled = dir.path().join("cancelled.mp4");
    let mut p = NativePipeline::new(
        &source,
        &cancelled,
        RenderScene::empty(320, 240),
        &[],
        VideoCodec::H264,
        1_500_000,
        Some(0x8086),
    )
    .unwrap();
    let state = AtomicU8::new(0);
    let result = p.run(&state, |_, frames| {
        if frames == 4 {
            state.store(2, Ordering::SeqCst);
        }
    });
    assert!(result.unwrap_err().contains("hủy"));
    drop(p);
    // The caller's TempPath owns unpublished output; driver handles must be closed.
    std::fs::remove_file(cancelled).unwrap();
    std::fs::remove_file(output).unwrap();
}

#[test]
#[ignore = "Opt-in 10s 1080p real native/staging transport benchmark; no performance CI threshold"]
fn native_gpu_1080p_transport_benchmark() {
    use crate::project_service::{BlurEffect, BlurRegionRole, SubtitleSettings, TextRemovalMode};
    use crate::render::{renderer::Renderer, stream};
    use crate::render_spec::{BlurRenderSpec, LogoRenderSpec, PixelRect, SubtitleEvent};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let generated = crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=30:d=10",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(generated.status.success());
    let mut scene = RenderScene::empty(1920, 1080);
    scene.blurs.push(BlurRenderSpec {
        id: "blur".into(),
        effect: BlurEffect::Blur,
        role: BlurRegionRole::Generic,
        rect: PixelRect {
            x: 200,
            y: 880,
            width: 1500,
            height: 160,
        },
        enabled: true,
        strength_px: 40,
        edge_blend: 20,
        pixel_noise: 0,
        text_removal_scan_fps: 1,
        text_removal_mode: TextRemovalMode::OcrGuided,
        start_ms: 0,
        end_ms: None,
    });
    let region = scene.blurs[0].rect;
    scene.subtitle.enabled = true;
    scene.subtitle.region = Some(region);
    let mut style = SubtitleSettings::default();
    style.font_size = 42;
    for n in 0..10 {
        scene.subtitle.events.push(SubtitleEvent {
            segment_id: format!("segment-{n}"),
            start: n as f64,
            end: n as f64 + 0.9,
            layout: crate::render_spec::layout_subtitle(
                &format!("Ma Đế trở về · Đoạn {n}"),
                region,
                &style,
            )
            .unwrap(),
        });
    }
    let logo = dir.path().join("logo.png");
    image::RgbaImage::from_pixel(80, 80, image::Rgba([255, 0, 0, 160]))
        .save(&logo)
        .unwrap();
    scene.logos.push(LogoRenderSpec {
        id: "logo".into(),
        asset_path: "validated".into(),
        rect: PixelRect {
            x: 1700,
            y: 100,
            width: 80,
            height: 80,
        },
        enabled: true,
        opacity: 0.8,
        maintain_aspect_ratio: true,
        start_ms: 0,
        end_ms: None,
    });
    let state = std::sync::Arc::new(AtomicU8::new(0));
    let before = std::time::Instant::now();
    let mut native_host = super::metrics::HostMetrics::new();
    let native = dir.path().join("native.mp4");
    let mut p = NativePipeline::new(
        &source,
        &native,
        scene.clone(),
        &[logo.clone()],
        VideoCodec::H264,
        4_000_000,
        Some(0x8086),
    )
    .unwrap();
    let native_counts = p
        .run(&state, |_, _| native_host.sample_throttled())
        .unwrap();
    drop(p);
    let native_seconds = before.elapsed().as_secs_f64();
    let native_cpu = native_host.total_cpu_percent(0.0);
    println!("{}", native_host.report("Native measurement"));
    assert_eq!(native_counts.frames, 300);
    assert_eq!(native_counts.upload_bytes + native_counts.readback_bytes, 0);
    let before = std::time::Instant::now();
    let mut staging_host = std::cell::RefCell::new(super::metrics::HostMetrics::new());
    let mut renderer = Renderer::new_preferred(scene, &[logo], Some(0x8086)).unwrap();
    let staging = dir.path().join("staging.mp4");
    let mut encoder = crate::runtime_paths::ffmpeg_command().unwrap();
    encoder
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-video_size",
            "1920x1080",
            "-framerate",
            "30",
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "h264_qsv",
            "-b:v",
            "4M",
            "-pix_fmt",
            "nv12",
        ])
        .arg(&staging)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let counts = stream::run(
        &mut renderer,
        stream::decoder_command(&source).unwrap(),
        encoder,
        30.0,
        &[],
        &state,
        |_, _| staging_host.borrow_mut().sample_throttled(),
    )
    .unwrap();
    let staging_seconds = before.elapsed().as_secs_f64();
    let staging_cpu = counts
        .child_cpu_seconds
        .and_then(|children| staging_host.get_mut().total_cpu_percent(children));
    println!("Whole video pipeline CPU, Rust + FFmpeg children: native={native_cpu:?}%; staging={staging_cpu:?}%");
    println!("{}", staging_host.get_mut().report("Staging measurement"));
    assert_eq!(counts.frames, 300);
    assert_eq!(counts.upload_bytes, 300 * 1920 * 1080 * 4);
    assert_eq!(counts.readback_bytes, counts.upload_bytes);
    assert_eq!(times(&native).len(), 300);
    assert_eq!(times(&staging).len(), 300);
    println!("1080p/300 frames Intel hardware, same wgpu blur/subtitle/logo: native={native_seconds:.3}s ({:.1}fps); staging={staging_seconds:.3}s ({:.1}fps); native upload/readback=0/0; staging upload/readback={}/{}",300.0/native_seconds,300.0/staging_seconds,counts.upload_bytes,counts.readback_bytes);
}

#[test]
#[ignore = "Real native GPU encode pixel parity: color, blur, subtitle sharpness, logo z-order"]
fn native_gpu_visual_pixels_match_staging_renderer() {
    use crate::{
        project_service::{BlurEffect, BlurRegionRole, SubtitleSettings, TextRemovalMode},
        render::{
            frame::{PixelFormat, VideoFrame},
            renderer::Renderer,
        },
        render_spec::{BlurRenderSpec, LogoRenderSpec, PixelRect, SubtitleEvent},
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("checker.mp4");
    let out = dir.path().join("native.mp4");
    let result=crate::runtime_paths::ffmpeg_command().unwrap().args(["-v","error","-y","-f","lavfi","-i","nullsrc=s=320x240:r=10:d=1,geq=lum='if(mod(floor(X/4)+floor(Y/4),2),32,220)':cb=128:cr=128","-c:v","libx264","-pix_fmt","yuv420p"]).arg(&source).output().unwrap();
    assert!(result.status.success());
    let original = std::fs::read(&source).unwrap();
    let mut scene = RenderScene::empty(320, 240);
    let rect = PixelRect {
        x: 30,
        y: 150,
        width: 260,
        height: 80,
    };
    scene.blurs.push(BlurRenderSpec {
        id: "subtitle".into(),
        effect: BlurEffect::Blur,
        role: BlurRegionRole::Subtitle,
        rect,
        enabled: true,
        strength_px: 40,
        edge_blend: 0,
        pixel_noise: 0,
        text_removal_scan_fps: 1,
        text_removal_mode: TextRemovalMode::OcrGuided,
        start_ms: 0,
        end_ms: None,
    });
    let mut style = SubtitleSettings::default();
    style.font_size = 26;
    style.min_font_size = 18;
    scene.subtitle.enabled = true;
    scene.subtitle.region = Some(rect);
    scene.subtitle.events.push(SubtitleEvent {
        segment_id: "sub".into(),
        start: 0.2,
        end: 0.8,
        layout: crate::render_spec::layout_subtitle("Ma Đế trở về", rect, &style).unwrap(),
    });
    let logo = dir.path().join("logo.png");
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))
        .save(&logo)
        .unwrap();
    scene.logos.push(LogoRenderSpec {
        id: "logo".into(),
        asset_path: "validated".into(),
        rect: PixelRect {
            x: 230,
            y: 170,
            width: 30,
            height: 30,
        },
        enabled: true,
        opacity: 0.8,
        maintain_aspect_ratio: true,
        start_ms: 0,
        end_ms: None,
    });
    let mut p = NativePipeline::new(
        &source,
        &out,
        scene.clone(),
        &[logo.clone()],
        VideoCodec::H264,
        4_000_000,
        Some(0x8086),
    )
    .unwrap();
    let counts = p.run(&AtomicU8::new(0), |_, _| {}).unwrap();
    drop(p);
    assert_eq!(counts.frames, 10);
    let decode = |path: &Path| {
        let o = crate::runtime_paths::ffmpeg_command()
            .unwrap()
            .args(["-v", "error", "-ss", "0.3", "-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(o.status.success());
        o.stdout
    };
    let input = decode(&source);
    let encoded = decode(&out);
    assert_eq!(input.len(), 320 * 240 * 4);
    assert_eq!(encoded.len(), input.len());
    let mut r = Renderer::new_preferred(scene, &[logo], Some(0x8086)).unwrap();
    let mut expected = vec![0; input.len()];
    r.render_frame(
        VideoFrame {
            width: 320,
            height: 240,
            pts_seconds: 0.3,
            format: PixelFormat::Rgba8,
            data: &input,
        },
        &mut expected,
    )
    .unwrap();
    let mae = expected
        .iter()
        .zip(&encoded)
        .map(|(a, b)| a.abs_diff(*b) as u64)
        .sum::<u64>() as f64
        / input.len() as f64;
    assert!(mae < 8.0, "native/staging color/pixel drift MAE={mae}");
    let pixel = |x: usize, y: usize| &encoded[(y * 320 + x) * 4..(y * 320 + x) * 4 + 4];
    assert!(
        pixel(240, 180)[0] > pixel(240, 180)[1] + 80,
        "logo must be sharp above blur/subtitle"
    );
    assert!(
        (pixel(40, 165)[0] as i32 - pixel(44, 165)[0] as i32).abs() < 15,
        "old checker texture must be blurred"
    );
    let glyphs = encoded
        .chunks_exact(4)
        .enumerate()
        .filter(|(i, c)| {
            let (x, y) = (i % 320, i / 320);
            (70..220).contains(&x)
                && (160..225).contains(&y)
                && c[0] > 220
                && c[1] > 220
                && c[2] > 220
        })
        .count();
    assert!(glyphs > 50, "subtitle glyphs must remain sharp");
    assert_eq!(
        original,
        std::fs::read(&source).unwrap(),
        "source must remain unchanged"
    );
    if let Ok(root) = std::env::var("JC_GPU_TEST_ARTIFACTS") {
        std::fs::create_dir_all(&root).unwrap();
        image::save_buffer(
            Path::new(&root).join("native-parity.png"),
            &encoded,
            320,
            240,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    println!("Native pixel parity MAE={mae:.3}; Unicode subtitle and sharp logo verified");
}

#[test]
#[ignore = "Real hardware SDR matrix/range and GPU scaling checks"]
fn native_gpu_color_range_and_scaling() {
    let dir = tempfile::tempdir().unwrap();
    for (matrix, range) in [("bt709", "pc"), ("bt709", "tv"), ("smpte170m", "tv")] {
        let source = dir.path().join(format!("{matrix}-{range}.mp4"));
        let output = dir.path().join("output.mp4");
        let result = crate::runtime_paths::ffmpeg_command()
            .unwrap()
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=640x480:r=10:d=1",
                "-vf",
                &format!(
                    "scale=out_color_matrix={}:out_range={}",
                    if matrix == "bt709" { "bt709" } else { "bt601" },
                    if range == "pc" { "full" } else { "limited" }
                ),
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-colorspace",
                matrix,
                "-color_range",
                range,
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(result.status.success());
        let mut p = NativePipeline::new(
            &source,
            &output,
            RenderScene::empty(320, 240),
            &[],
            VideoCodec::H264,
            4_000_000,
            Some(0x8086),
        )
        .unwrap();
        p.run(&AtomicU8::new(0), |_, _| {}).unwrap();
        drop(p);
        let decode = |path: &Path| {
            let result = crate::runtime_paths::ffmpeg_command()
                .unwrap()
                .args(["-v", "error", "-ss", "0.3", "-i"])
                .arg(path)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=320:240:flags=bilinear",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(result.status.success());
            result.stdout
        };
        let original = decode(&source);
        let encoded = decode(&output);
        assert_eq!(original.len(), encoded.len());
        let mae = original
            .iter()
            .zip(&encoded)
            .map(|(a, b)| a.abs_diff(*b) as u64)
            .sum::<u64>() as f64
            / original.len() as f64;
        println!("Native color {matrix}/{range} + GPU scale: MAE={mae:.2}");
        assert!(mae < 6.0, "incorrect color/range conversion");
    }
}
