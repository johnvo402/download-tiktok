use super::*;
use crate::{
    project_service::ProjectStore,
    render::{graph::RenderScene, renderer::Renderer},
    render_spec::{build_render_specification, ExportOptions},
};
#[test]
fn patches_preserve_unrelated_segments_and_coalesce_latest() {
    let dir = tempfile::tempdir().unwrap();
    let (_, _, mut tr) = fixture(dir.path().join("projects"));
    let id = tr.segments[0].id.clone();
    let untouched = tr.segments[1].clone();
    let patch = |text: &str| manager::TranslationPatch {
        id: id.clone(),
        start: 0.0,
        end: 4.0,
        translated: text.into(),
        deleted: false,
    };
    let mut updates = manager::Updates::default();
    updates.patch_scene(1, None, Some(vec![patch("first")]));
    updates.patch_scene(3, None, Some(vec![patch("newest")]));
    updates.patch_scene(2, None, Some(vec![patch("stale")]));
    let (v, _, patches) = updates.scene.take().unwrap();
    assert_eq!(v, 3);
    assert_eq!(patches.as_ref().unwrap().len(), 1);
    scene::apply_translations(&mut tr, patches.unwrap(), 10.0).unwrap();
    assert_eq!(tr.segments[0].translated, "newest");
    assert_eq!(tr.segments[1], untouched);
    let new = manager::TranslationPatch {
        id: "66666666-6666-4666-8666-666666666666".into(),
        start: 5.0,
        end: 6.0,
        translated: "Thêm câu mới".into(),
        deleted: false,
    };
    scene::apply_translations(&mut tr, vec![new.clone()], 10.0).unwrap();
    assert!(tr.segments.iter().any(|s| s.id == new.id));
    scene::apply_translations(
        &mut tr,
        vec![manager::TranslationPatch {
            deleted: true,
            ..new
        }],
        10.0,
    )
    .unwrap();
    assert_eq!(tr.segments.len(), 2);
    let mut invalid = patch("invalid");
    invalid.end = 11.0;
    assert!(scene::apply_translations(&mut tr, vec![invalid], 10.0).is_err());
}
#[test]
fn target_scaling_preserves_timing_layout_and_blur_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let (_, p, tr) = fixture(dir.path().join("projects"));
    let full = scene::build(&p, &tr).unwrap();
    let half = full.scaled(0.5);
    assert_eq!((half.width, half.height), (320, 180));
    assert_eq!(half.pixel_scale, 0.5);
    assert_eq!(half.blurs[0].strength_px, full.blurs[0].strength_px);
    assert_eq!(half.blurs[0].rect.width, full.blurs[0].rect.width / 2);
    assert_eq!(half.text[0].font_size, full.text[0].font_size / 2);
    assert_eq!(
        half.subtitle.events[0].layout.text,
        full.subtitle.events[0].layout.text
    );
    assert_eq!(half.subtitle.events[0].start, full.subtitle.events[0].start);
    assert_eq!(half.subtitle.events[0].end, full.subtitle.events[0].end);
    assert_eq!(full.pixel_scale, 1.0);
    assert_eq!((full.width, full.height), (640, 360));
}
fn fixture(
    root: std::path::PathBuf,
) -> (
    ProjectStore,
    crate::project_service::Project,
    crate::transcript_service::Transcript,
) {
    let store = ProjectStore::new(root);
    let mut p = store.create("Native Preview QA").unwrap();
    p.source.video_path = Some("media/source/test.mp4".into());
    p.source.width = Some(640);
    p.source.height = Some(360);
    p.source.duration = Some(10.0);
    p.source.fps = Some(30.0);
    p.blur_regions=serde_json::from_value(serde_json::json!([{"id":"11111111-1111-4111-8111-111111111111","x":0.1,"y":0.65,"width":0.8,"height":0.25,"enabled":true,"startMs":0,"endMs":null,"strength":65.0,"role":"subtitle","effect":"blur","edgeBlend":10,"pixelNoise":0,"textRemovalScanFps":2,"textRemovalMode":"ocrGuided"}])).unwrap();
    p.text_overlays=serde_json::from_value(serde_json::json!([{"id":"22222222-2222-4222-8222-222222222222","text":"JC AI · Kiểm thử GPU","x":0.1,"y":0.08,"width":0.7,"height":0.15,"enabled":true,"fontFamily":"Segoe UI","fontSize":30,"fontWeight":600,"color":"#ffff00","opacity":1.0,"alignment":"left"}])).unwrap();
    let asset = "media/overlays/55555555-5555-4555-8555-555555555555.png";
    let path = store.project_directory(&p.id).unwrap().join(asset);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 0, 0, 128]))
        .save(path)
        .unwrap();
    p.logos = vec![crate::project_service::ProjectLogo {
        id: "55555555-5555-4555-8555-555555555555".into(),
        relative_path: asset.into(),
        original_name: "logo.png".into(),
        x: 0.82,
        y: 0.08,
        width: 0.1,
        height: 0.1,
        opacity: 0.7,
        ..Default::default()
    }];
    let mut tr = scene::empty_transcript(&p);
    tr.segments=serde_json::from_value(serde_json::json!([
 {"id":"33333333-3333-4333-8333-333333333333","start":0.0,"end":4.0,"source":"hello","translated":"Ma Đế trở về · 修仙","speakerId":null,"sourceType":"asr","translationStatus":"translated","tts":null},
 {"id":"44444444-4444-4444-8444-444444444444","start":4.0,"end":10.0,"source":"world","translated":"Cảnh giới Đại Đế","speakerId":null,"sourceType":"asr","translationStatus":"translated","tts":null}
 ])).unwrap();
    (store, p, tr)
}
#[test]
fn bounds_reject_nonfinite_and_extreme_dimensions() {
    let b = Bounds {
        x: 0.0,
        y: 0.0,
        width: 960.0,
        height: 540.0,
        dpr: 1.5,
        visible: true,
    };
    assert!(b.validate().is_ok());
    assert!(Bounds {
        width: f64::NAN,
        ..b
    }
    .validate()
    .is_err());
    assert!(Bounds { dpr: 0.0, ..b }.validate().is_err());
    assert!(Bounds {
        width: 40000.0,
        ..b
    }
    .validate()
    .is_err());
}
#[test]
fn exact_scene_and_timestamp_parity() {
    let dir = tempfile::tempdir().unwrap();
    let (_, p, tr) = fixture(dir.path().join("projects"));
    let preview = scene::build(&p, &tr).unwrap();
    let export = RenderScene::from_spec(
        &build_render_specification(&p, &tr, 10.0, &ExportOptions::default()).unwrap(),
    );
    assert_eq!(
        serde_json::to_value(&preview.blurs).unwrap(),
        serde_json::to_value(&export.blurs).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&preview.text).unwrap(),
        serde_json::to_value(&export.text).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&preview.subtitle).unwrap(),
        serde_json::to_value(&export.subtitle).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&preview.logos).unwrap(),
        serde_json::to_value(&export.logos).unwrap()
    );
    for pts in [0.0, 3.999, 4.0, 9.999, 10.0] {
        assert_eq!(
            preview
                .subtitle
                .events
                .iter()
                .filter(|e| pts >= e.start && pts < e.end)
                .map(|e| e.segment_id.as_str())
                .collect::<Vec<_>>(),
            export
                .subtitle
                .events
                .iter()
                .filter(|e| pts >= e.start && pts < e.end)
                .map(|e| e.segment_id.as_str())
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn incomplete_project_preview_keeps_export_validation() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut p, tr) = fixture(dir.path().join("projects"));
    p.blur_regions.clear();
    p.subtitle.region = None;
    assert!(!scene::build(&p, &tr).unwrap().subtitle.enabled);
    assert!(build_render_specification(&p, &tr, 10.0, &ExportOptions::default()).is_err());
}
#[test]
#[ignore = "Actual GPU pixel comparison; no image frame IPC"]
fn preview_export_pixel_parity() {
    let dir = tempfile::tempdir().unwrap();
    let (store, p, tr) = fixture(dir.path().join("projects"));
    let scene = scene::build(&p, &tr).unwrap();
    let paths = scene::logos(&store, &p.id, &scene).unwrap();
    let mut renderer = Renderer::new(scene, &paths).unwrap();
    let gpu = renderer.device();
    let pixels = vec![80u8; 640 * 360 * 4];
    let mut export = vec![0; pixels.len()];
    renderer
        .render_frame(
            crate::render::frame::VideoFrame {
                width: 640,
                height: 360,
                pts_seconds: 1.0,
                format: crate::render::frame::PixelFormat::Rgba8,
                data: &pixels,
            },
            &mut export,
        )
        .unwrap();
    renderer.upload_preview_frame(&pixels).unwrap();
    let texture = gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("test preview target"),
        size: wgpu::Extent3d {
            width: 640,
            height: 360,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    renderer
        .present_preview(
            None,
            &texture.create_view(&Default::default()),
            (640, 360),
            1.0,
            None,
        )
        .unwrap();
    let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test snapshot only"),
        size: pixels.len() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(640 * 4),
                rows_per_image: Some(360),
            },
        },
        wgpu::Extent3d {
            width: 640,
            height: 360,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue().submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mut preview = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    for pixel in preview.chunks_mut(4) {
        pixel.swap(0, 2);
    }
    assert_eq!(
        preview, export,
        "Preview and export RGBA must match before encode"
    );
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".release-cache/preview-validation");
    std::fs::create_dir_all(&output).unwrap();
    image::save_buffer(
        output.join("parity.png"),
        &preview,
        640,
        360,
        image::ColorType::Rgba8,
    )
    .unwrap();
}
#[test]
#[ignore = "Creates isolated UI fixture for desktop manual validation"]
fn create_desktop_preview_fixture() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".release-cache/preview-validation/data/com.ai.video-localization/projects");
    let (store, mut p, mut tr) = fixture(root);
    let folder = store.project_directory(&p.id).unwrap();
    let input = folder.join("media/source/test.mp4");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    assert!(crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=10",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-shortest",
            "-y"
        ])
        .arg(&input)
        .status()
        .unwrap()
        .success());
    p = crate::video_service::import_video(&store, &p.id, &input.to_string_lossy(), true)
        .unwrap()
        .project;
    // Import clears dependent settings; restore the test visuals using the same project fixture.
    let scratch = tempfile::tempdir().unwrap();
    let (_, template, _) = fixture(scratch.path().join("projects"));
    p.blur_regions = template.blur_regions;
    p.text_overlays = template.text_overlays;
    p.logos = template.logos;
    store.update_project(&p).unwrap();
    tr.project_id = p.id.clone();
    tr.video_checksum = p.source.checksum_sha256.clone();
    crate::transcript_service::save_transcript(&store, &p.id, tr).unwrap();
    println!("Isolated desktop fixture: {}", p.id);
}

#[cfg(windows)]
#[test]
#[ignore = "Real DX12 native swapchain, decoder seek/resize and fixed-resource exercise"]
fn native_surface_play_seek_resize_and_long_source() {
    use windows::{
        core::w,
        Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    };
    struct Host(HWND);
    impl Drop for Host {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    let host = Host(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("JC AI native preview integration"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            0,
            0,
            900,
            600,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    let dir = tempfile::tempdir().unwrap();
    let (store, p, tr) = fixture(dir.path().join("projects"));
    let scene = scene::build(&p, &tr).unwrap();
    let paths = scene::logos(&store, &p.id, &scene).unwrap();
    let gpu =
        crate::render::device::create_with_power(None, wgpu::PowerPreference::LowPower).unwrap();
    let mut renderer = Renderer::with_device(scene, &paths, gpu.clone()).unwrap();
    let b = Bounds {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 500.0,
        dpr: 1.0,
        visible: true,
    };
    let mut surface = surface::NativeSurface::new(
        host.0 .0 as isize,
        Box::new(|_, _, _, _| {}),
        Box::new(|_| {}),
        &gpu,
        b,
    )
    .unwrap();
    let input = dir.path().join("source-字幕.mp4");
    assert!(crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=10",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "30",
            "-y"
        ])
        .arg(&input)
        .status()
        .unwrap()
        .success());
    let mut source =
        crate::media_gpu::windows::preview::PreviewSource::new(&renderer, &input).unwrap();
    let mut host_metrics = crate::media_gpu::windows::metrics::HostMetrics::new();
    let started = std::time::Instant::now();
    let mut render_ms = 0.0;
    for i in 0..300 {
        surface.pump();
        let time = i as f64 / 30.0;
        let pts = source.select(time, || false).unwrap().unwrap();
        assert!(pts <= time + 0.001 && time - pts < 0.04);
        let frame = surface.surface.get_current_texture().unwrap();
        let start = std::time::Instant::now();
        source
            .present(
                &mut renderer,
                &frame.texture.create_view(&Default::default()),
                (surface.config.width, surface.config.height),
                pts,
                None,
            )
            .unwrap();
        frame.present();
        render_ms += start.elapsed().as_secs_f64() * 1000.0;
        host_metrics.sample_throttled();
    }
    println!(
        "Native preview adapter={}; 300 frames={:.3}s; average render+present={:.2}ms",
        renderer.adapter_name(),
        started.elapsed().as_secs_f64(),
        render_ms / 300.0
    );
    for i in 0..60 {
        let time = (i * 7 % 90) as f64 / 10.0;
        let start = std::time::Instant::now();
        source.seek(time).unwrap();
        let pts = source.select(time, || false).unwrap().unwrap();
        assert!((time - pts).abs() < 0.04);
        surface
            .resize(
                &gpu,
                Bounds {
                    width: 400.0 + (i % 3) as f64 * 100.0,
                    height: 300.0 + (i % 2) as f64 * 100.0,
                    dpr: [1.0, 1.25, 1.5, 2.0][i % 4],
                    ..b
                },
            )
            .unwrap();
        let frame = surface.surface.get_current_texture().unwrap();
        source
            .present(
                &mut renderer,
                &frame.texture.create_view(&Default::default()),
                (surface.config.width, surface.config.height),
                pts,
                None,
            )
            .unwrap();
        frame.present();
        if i == 59 {
            println!(
                "seek+render+resize latency={:.2}ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
    println!("{}", host_metrics.report("Preview play/seek/resize"));
    drop(source);
    let long = dir.path().join("one-hour.mp4");
    assert!(crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=navy:s=320x180:r=1:d=3605",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "1",
            "-y"
        ])
        .arg(&long)
        .status()
        .unwrap()
        .success());
    let mut source =
        crate::media_gpu::windows::preview::PreviewSource::new(&renderer, &long).unwrap();
    for time in [3600.0, 5.0, 2700.0, 10.0] {
        source.seek(time).unwrap();
        let pts = source.select(time, || false).unwrap().unwrap();
        assert!((pts - time).abs() < 0.01);
    }
    // Pause redraw reuses the held frame; no additional decoder consumption.
    assert_eq!(source.select(10.0, || false).unwrap(), Some(10.0));
    drop(source);
    drop(surface);
    drop(renderer);
    drop(host);
}

#[cfg(windows)]
#[test]
#[ignore = "Actual 720p/1080p/4K swapchain, software fallback and repeated lifecycle stress"]
fn native_preview_resolution_and_lifecycle_stress() {
    use windows::{
        core::w,
        Win32::{
            Foundation::*,
            System::Threading::{GetCurrentProcess, GetProcessHandleCount},
            UI::WindowsAndMessaging::*,
        },
    };
    struct Host(HWND);
    impl Drop for Host {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    let host = Host(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("JC AI preview stress"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            0,
            0,
            1000,
            650,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    let dir = tempfile::tempdir().unwrap();
    let (store, template, tr) = fixture(dir.path().join("projects"));
    let gpu =
        crate::render::device::create_with_power(None, wgpu::PowerPreference::LowPower).unwrap();
    let b = Bounds {
        x: 0.0,
        y: 0.0,
        width: 960.0,
        height: 540.0,
        dpr: 1.0,
        visible: true,
    };
    let mut metrics = crate::media_gpu::windows::metrics::HostMetrics::new();
    for (w, h, scale) in [
        (1280, 720, 1.0),
        (1920, 1080, 1.0),
        (3840, 2160, 1.0),
        (3840, 2160, 0.25),
    ] {
        let input = dir.path().join(format!("test-{w}.mp4"));
        if !input.is_file() {
            assert!(crate::runtime_paths::ffmpeg_command()
                .unwrap()
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("testsrc2=s={w}x{h}:r=30:d=3"),
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-g",
                    "30",
                    "-y"
                ])
                .arg(&input)
                .status()
                .unwrap()
                .success());
        }
        let mut p = template.clone();
        p.source.width = Some(w);
        p.source.height = Some(h);
        let scene = scene::build(&p, &tr).unwrap().scaled(scale);
        let paths = scene::logos(&store, &p.id, &scene).unwrap();
        let mut renderer = Renderer::with_device(scene, &paths, gpu.clone()).unwrap();
        let surface = surface::NativeSurface::new(
            host.0 .0 as isize,
            Box::new(|_, _, _, _| {}),
            Box::new(|_| {}),
            &gpu,
            b,
        )
        .unwrap();
        let mut source =
            crate::media_gpu::windows::preview::PreviewSource::new(&renderer, &input).unwrap();
        let start = std::time::Instant::now();
        let mut render_ms = 0.0;
        for i in 0..90 {
            surface.pump();
            let pts = source.select(i as f64 / 30.0, || false).unwrap().unwrap();
            let frame = surface.surface.get_current_texture().unwrap();
            let start = std::time::Instant::now();
            source
                .present(
                    &mut renderer,
                    &frame.texture.create_view(&Default::default()),
                    (surface.config.width, surface.config.height),
                    pts,
                    None,
                )
                .unwrap();
            frame.present();
            render_ms += start.elapsed().as_secs_f64() * 1000.0;
            metrics.sample_throttled();
        }
        println!(
            "Preview {w}x{h} scale={scale}: 90 frames={:.3}s, render+present average={:.2}ms",
            start.elapsed().as_secs_f64(),
            render_ms / 90.0
        );
        drop(source);
        // The staging decoder still invokes the same renderer/surface and PTS selection.
        let mut software =
            software::SoftwareSource::new(&input, (renderer.scene.width, renderer.scene.height))
                .unwrap();
        software.seek(1.0).unwrap();
        let mut pts = None;
        for _ in 0..30 {
            pts = software.select(1.0, || false).unwrap();
            if pts.is_some() {
                break;
            }
        }
        assert!((pts.unwrap() - 1.0).abs() < 0.04);
        software.upload(&renderer).unwrap();
        let frame = surface.surface.get_current_texture().unwrap();
        renderer
            .present_preview(
                None,
                &frame.texture.create_view(&Default::default()),
                (surface.config.width, surface.config.height),
                1.0,
                None,
            )
            .unwrap();
        frame.present();
    }
    let input = dir.path().join("test-1280.mp4");
    let handles = || {
        let mut count = 0;
        unsafe {
            GetProcessHandleCount(GetCurrentProcess(), &mut count).unwrap();
        }
        count
    };
    let baseline = handles();
    for _ in 0..20 {
        let scene = scene::build(&template, &tr).unwrap();
        let paths = scene::logos(&store, &template.id, &scene).unwrap();
        let mut renderer = Renderer::with_device(scene, &paths, gpu.clone()).unwrap();
        let surface = surface::NativeSurface::new(
            host.0 .0 as isize,
            Box::new(|_, _, _, _| {}),
            Box::new(|_| {}),
            &gpu,
            b,
        )
        .unwrap();
        let mut source =
            crate::media_gpu::windows::preview::PreviewSource::new(&renderer, &input).unwrap();
        for time in [0.0, 2.0, 0.5] {
            source.seek(time).unwrap();
            let pts = source.select(time, || false).unwrap().unwrap();
            let frame = surface.surface.get_current_texture().unwrap();
            source
                .present(
                    &mut renderer,
                    &frame.texture.create_view(&Default::default()),
                    (surface.config.width, surface.config.height),
                    pts,
                    None,
                )
                .unwrap();
            frame.present();
        }
        surface.pump();
        drop(source);
        drop(surface);
        drop(renderer);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let after = handles();
    assert!(
        after <= baseline + 32,
        "Native handles increased: {baseline}->{after}"
    );
    println!(
        "20 preview lifecycles: handles {baseline}->{after}; {}",
        metrics.report("Resolution/lifecycle")
    );
    let seconds = std::env::var("JC_PREVIEW_STRESS_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30)
        .clamp(1, 300);
    let scene = scene::build(&template, &tr).unwrap();
    let paths = scene::logos(&store, &template.id, &scene).unwrap();
    let mut renderer = Renderer::with_device(scene, &paths, gpu.clone()).unwrap();
    let surface = surface::NativeSurface::new(
        host.0 .0 as isize,
        Box::new(|_, _, _, _| {}),
        Box::new(|_| {}),
        &gpu,
        b,
    )
    .unwrap();
    let mut source =
        crate::media_gpu::windows::preview::PreviewSource::new(&renderer, &input).unwrap();
    source.select(0.0, || false).unwrap();
    let frame = surface.surface.get_current_texture().unwrap();
    source
        .present(
            &mut renderer,
            &frame.texture.create_view(&Default::default()),
            (surface.config.width, surface.config.height),
            0.0,
            None,
        )
        .unwrap();
    frame.present();
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let active_baseline = handles();
    let resources_before =
        crate::media_gpu::windows::metrics::ResourceSnapshot::read(&gpu.device).unwrap();
    let start = std::time::Instant::now();
    let mut previous = 0.0;
    let mut frames = 0;
    while start.elapsed().as_secs() < seconds {
        let t = start.elapsed().as_secs_f64() % 3.0;
        if t < previous {
            source.seek(0.0).unwrap();
        }
        previous = t;
        surface.pump();
        let pts = source.select(t, || false).unwrap().unwrap();
        let frame = surface.surface.get_current_texture().unwrap();
        source
            .present(
                &mut renderer,
                &frame.texture.create_view(&Default::default()),
                (surface.config.width, surface.config.height),
                pts,
                None,
            )
            .unwrap();
        frame.present();
        frames += 1;
        metrics.sample_throttled();
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let active_end = handles();
    assert!(
        active_end <= active_baseline + 16,
        "Active handles increased: {active_baseline}->{active_end}"
    );
    println!("{seconds}s continuous preview stress, {frames} presents; handles {active_baseline}->{active_end}; {}",metrics.report("Stress"));
    let resources_after =
        crate::media_gpu::windows::metrics::ResourceSnapshot::read(&gpu.device).unwrap();
    println!("Resource baseline={resources_before:?}; after={resources_after:?}");
    assert!(resources_after.handles <= resources_before.handles + 16);
    assert!(resources_after.threads <= resources_before.threads + 8);
    assert!(resources_after.working_set <= resources_before.working_set + 192 * 1024 * 1024);
    assert!(resources_after.gpu_local <= resources_before.gpu_local + 128 * 1024 * 1024);
    assert!(resources_after.gpu_shared <= resources_before.gpu_shared + 128 * 1024 * 1024);
}
