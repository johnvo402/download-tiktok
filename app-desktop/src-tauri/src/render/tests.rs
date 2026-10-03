use super::*;
use crate::project_service::{BlurEffect, BlurRegionRole, SubtitleSettings, TextRemovalMode};
use crate::render_spec::{BlurRenderSpec, LogoRenderSpec, SubtitleEvent, TextOverlayRenderSpec};
#[test]
#[ignore = "Opt-in real graphics adapter, pixel checks and screenshot artifacts"]
fn gpu_pixels_passthrough_effects_alpha_timing_and_cache() {
    let width = 320;
    let height = 180;
    let mut pixels = vec![0_u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let at = ((y * width + x) * 4) as usize;
            let c = if (x / 4 + y / 4) % 2 == 0 { 32 } else { 220 };
            pixels[at..at + 4].copy_from_slice(&[c, c, c, 255]);
        }
    }
    let Ok(mut plain) = Renderer::new(RenderScene::empty(width, height), &[]) else {
        eprintln!("Skipping GPU pixels: adapter unavailable");
        return;
    };
    let mut pass = vec![0; pixels.len()];
    plain
        .render_frame(
            VideoFrame {
                width,
                height,
                pts_seconds: 0.0,
                format: PixelFormat::Rgba8,
                data: &pixels,
            },
            &mut pass,
        )
        .unwrap();
    assert_eq!(pass, pixels, "passthrough must preserve RGBA bytes");
    drop(plain);
    let mut scene = RenderScene::empty(width, height);
    let rect = PixelRect {
        x: 30,
        y: 95,
        width: 260,
        height: 75,
    };
    scene.blurs.push(BlurRenderSpec {
        id: "subtitle".into(),
        effect: BlurEffect::Blur,
        role: BlurRegionRole::Subtitle,
        rect,
        enabled: true,
        strength_px: 50,
        edge_blend: 40,
        pixel_noise: 0,
        text_removal_scan_fps: 2,
        text_removal_mode: TextRemovalMode::OcrGuided,
        start_ms: 100,
        end_ms: Some(900),
    });
    scene.blurs.push(BlurRenderSpec {
        id: "generic".into(),
        rect: PixelRect {
            x: 0,
            y: 0,
            width: 60,
            height: 40,
        },
        role: BlurRegionRole::Generic,
        ..scene.blurs[0].clone()
    });
    scene.subtitle.enabled = true;
    scene.subtitle.region = Some(rect);
    scene.subtitle.configured_font_size = 24;
    scene.subtitle.min_font_size = 16;
    let mut style = SubtitleSettings::default();
    style.font_size = 24;
    style.min_font_size = 16;
    style.padding_x = 4;
    style.padding_y = 5;
    scene.subtitle.events.push(SubtitleEvent {
        segment_id: "a".into(),
        start: 0.2,
        end: 0.8,
        layout: crate::render_spec::layout_subtitle("Ma Đế trở về\n修仙", rect, &style).unwrap(),
    });
    scene.text.push(TextOverlayRenderSpec {
        id: "title".into(),
        text: "Cảnh giới Đại Đế".into(),
        rect: PixelRect {
            x: 65,
            y: 5,
            width: 240,
            height: 50,
        },
        enabled: true,
        font_name: "Segoe UI".into(),
        font_size: 24,
        font_weight: 600,
        color: "#ffff00".into(),
        opacity: 0.75,
        alignment: "center".into(),
    });
    let folder = tempfile::tempdir().unwrap();
    let logo_path = folder.path().join("logo.png");
    image::RgbaImage::from_pixel(20, 20, image::Rgba([255, 0, 0, 128]))
        .save(&logo_path)
        .unwrap();
    scene.logos.push(LogoRenderSpec {
        id: "logo".into(),
        asset_path: "validated".into(),
        rect: PixelRect {
            x: 260,
            y: 120,
            width: 20,
            height: 20,
        },
        enabled: true,
        opacity: 1.0,
        maintain_aspect_ratio: true,
        start_ms: 200,
        end_ms: Some(800),
    });
    let mut renderer = Renderer::new(scene, &[logo_path]).unwrap();
    let mut output = vec![0; pixels.len()];
    renderer
        .render_frame(
            VideoFrame {
                width,
                height,
                pts_seconds: 0.0,
                format: PixelFormat::Rgba8,
                data: &pixels,
            },
            &mut output,
        )
        .unwrap();
    // Timed blur/subtitle/logo inactive at 0. Outside static text stays original.
    assert_eq!(
        &output[(130 * width * 4) as usize..(131 * width * 4) as usize],
        &pixels[(130 * width * 4) as usize..(131 * width * 4) as usize]
    );
    renderer
        .render_frame(
            VideoFrame {
                width,
                height,
                pts_seconds: 0.3,
                format: PixelFormat::Rgba8,
                data: &pixels,
            },
            &mut output,
        )
        .unwrap();
    let outside = ((70 * width + 150) * 4) as usize;
    assert_eq!(&output[outside..outside + 4], &pixels[outside..outside + 4]);
    // Gaussian centre changes; feather edge has a smooth transition.
    let blurred = ((110 * width + 60) * 4) as usize;
    assert!((output[blurred] as i32 - pixels[blurred] as i32).abs() > 30);
    let border = ((95 * width + 60) * 4) as usize;
    assert!((output[border] as i32 - pixels[border] as i32).abs() < 10);
    // Logo last: a transparent red layer atop blurred background remains red/sharp.
    let logo = ((130 * width + 270) * 4) as usize;
    assert!(output[logo] > output[logo + 1] + 50);
    assert!((output[logo + 1] as i32 - output[logo + 2] as i32).abs() < 3);
    let count = renderer.text.len();
    let bytes = renderer.text_bytes;
    let first_output = output.clone();
    renderer
        .render_frame(
            VideoFrame {
                width,
                height,
                pts_seconds: 0.31,
                format: PixelFormat::Rgba8,
                data: &pixels,
            },
            &mut output,
        )
        .unwrap();
    assert_eq!(count, renderer.text.len());
    assert_eq!(bytes, renderer.text_bytes);
    assert_eq!(
        first_output, output,
        "noise and cached layers must be deterministic"
    );
    assert!(
        output.chunks_exact(4).enumerate().any(|(i, p)| {
            let y = i as u32 / width;
            y > 95 && y < 170 && p[0] > 240 && p[1] > 240 && p[2] > 240
        }),
        "translated glyphs must remain sharp on blur"
    );
    let index = renderer
        .register_patch(PixelRect {
            x: 120,
            y: 65,
            width: 10,
            height: 10,
        })
        .unwrap();
    let patch = vec![255; 10 * 10 * 4];
    renderer
        .render_frame_with_patches(
            VideoFrame {
                width,
                height,
                pts_seconds: 0.31,
                format: PixelFormat::Rgba8,
                data: &pixels,
            },
            &[(index, &patch)],
            &mut output,
        )
        .unwrap();
    assert_eq!(output[((70 * width + 125) * 4) as usize], 255);
    if let Ok(path) = std::env::var("JC_GPU_TEST_ARTIFACTS") {
        let root = PathBuf::from(path);
        std::fs::create_dir_all(&root).unwrap();
        image::save_buffer(
            root.join("input.png"),
            &pixels,
            width,
            height,
            image::ColorType::Rgba8,
        )
        .unwrap();
        image::save_buffer(
            root.join("gpu-effects.png"),
            &output,
            width,
            height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    println!("GPU pixel effects passed on {}", renderer.adapter_name());
    renderer.scene.text[0].text = "Nội dung mới".into();
    renderer
        .render_frame(
            VideoFrame {
                width,
                height,
                pts_seconds: 0.31,
                format: PixelFormat::Rgba8,
                data: &pixels,
            },
            &mut output,
        )
        .unwrap();
    assert_eq!(
        renderer.text.len(),
        count + 1,
        "same ID with changed text needs new pixels"
    );
    assert_ne!(
        &output[..(width * 50 * 4) as usize],
        &first_output[..(width * 50 * 4) as usize]
    );
    assert!(renderer
        .register_patch(PixelRect {
            x: u32::MAX,
            y: 0,
            width: 10,
            height: 10
        })
        .is_err());
}
#[test]
#[ignore = "Opt-in warm 1080p benchmark; no performance assertion in CI"]
fn gpu_render_1080p_resource_reuse() {
    let (width, height) = (1920, 1080);
    let mut scene = RenderScene::empty(width, height);
    let rect = PixelRect {
        x: 160,
        y: 810,
        width: 1600,
        height: 210,
    };
    scene.blurs.push(BlurRenderSpec {
        id: "bench".into(),
        rect,
        effect: BlurEffect::Blur,
        role: BlurRegionRole::Subtitle,
        enabled: true,
        strength_px: 80,
        edge_blend: 20,
        pixel_noise: 10,
        text_removal_scan_fps: 2,
        text_removal_mode: TextRemovalMode::OcrGuided,
        start_ms: 0,
        end_ms: None,
    });
    scene.subtitle.enabled = true;
    scene.subtitle.region = Some(rect);
    scene.subtitle.events.push(SubtitleEvent {
        segment_id: "bench-sub".into(),
        start: 0.0,
        end: 10.0,
        layout: crate::render_spec::layout_subtitle(
            "Cảnh giới Đại Đế — Ma Đế trở về\n修仙",
            rect,
            &SubtitleSettings::default(),
        )
        .unwrap(),
    });
    scene.text.push(TextOverlayRenderSpec {
        id: "bench-title".into(),
        text: "JC AI".into(),
        rect: PixelRect {
            x: 30,
            y: 30,
            width: 300,
            height: 80,
        },
        enabled: true,
        font_name: "Segoe UI".into(),
        font_size: 42,
        font_weight: 600,
        color: "#ffffff".into(),
        opacity: 0.8,
        alignment: "left".into(),
    });
    let folder = tempfile::tempdir().unwrap();
    let logo = folder.path().join("logo.png");
    image::RgbaImage::from_pixel(128, 128, image::Rgba([200, 255, 20, 128]))
        .save(&logo)
        .unwrap();
    scene.logos.push(LogoRenderSpec {
        id: "bench-logo".into(),
        asset_path: "validated".into(),
        rect: PixelRect {
            x: 1720,
            y: 20,
            width: 128,
            height: 128,
        },
        enabled: true,
        opacity: 0.8,
        maintain_aspect_ratio: true,
        start_ms: 0,
        end_ms: None,
    });
    let Ok(mut renderer) = Renderer::new(scene, &[logo]) else {
        eprintln!("Skipping 1080p render-only benchmark: no adapter");
        return;
    };
    let mut input = vec![0; byte_len(width, height).unwrap()];
    for (i, pixel) in input.chunks_exact_mut(4).enumerate() {
        let c = if (i as u32 % width / 4 + i as u32 / width / 4) % 2 == 0 {
            32
        } else {
            220
        };
        pixel.copy_from_slice(&[c, c, c, 255]);
    }
    let mut output = vec![0; input.len()];
    let frame = |pts| VideoFrame {
        width,
        height,
        pts_seconds: pts,
        format: PixelFormat::Rgba8,
        data: &input,
    };
    renderer.render_frame(frame(0.0), &mut output).unwrap();
    let count = renderer.text.len();
    let bytes = renderer.text_bytes;
    let started = std::time::Instant::now();
    let frames = 60;
    for n in 0..frames {
        renderer
            .render_frame(frame(n as f64 / 30.0), &mut output)
            .unwrap();
    }
    let seconds = started.elapsed().as_secs_f64();
    assert_eq!(count, renderer.text.len());
    assert_eq!(bytes, renderer.text_bytes);
    println!("wgpu warm 1080p: adapter={}; backend={}; frames={frames}; elapsed={seconds:.3}s; fps={:.1}; ms/frame={:.2}; text_cache={} bytes",
        renderer.adapter_name(),renderer.adapter_backend(),frames as f64/seconds,1000.0*seconds/frames as f64,bytes);
}
