use super::*;
use crate::{
    hardware_acceleration::VideoEncoder,
    project_service::{Project, TranscriptSourceType},
    timeline::{self, ClipContent, ClipStyle, ClipTransform, MediaKind, Track, TrackKind},
    transcript_service::{self, Transcript, TranscriptSegment},
    worker_manager::WorkerCommandExt,
};
use std::{
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use uuid::Uuid;

pub(super) fn plan() -> Plan {
    let mut doc = timeline::TimelineDocument::default();
    let clip = Clip {
        id: Uuid::new_v4().to_string(),
        track_id: doc.tracks[0].id.clone(),
        start_us: 2_000_000,
        source_in_us: 1_000_000,
        source_out_us: 5_000_000,
        rate: Rational {
            numerator: 2,
            denominator: 1,
        },
        content: ClipContent::Text {
            text: "hello".into(),
        },
        transform: ClipTransform::default(),
        linked_clip_id: None,
        group_id: None,
        style: None,
        audio: None,
    };
    doc.clips.push(clip);
    Plan {
        document: doc,
        options: TimelineExportOptions {
            width: 320,
            height: 180,
            frame_rate: Rational {
                numerator: 25,
                denominator: 1,
            },
            start_us: None,
            end_us: None,
        },
        encoder: VideoEncoder::Cpu,
        start: 0,
        end: 4_000_000,
        assets: HashMap::new(),
        sources: HashMap::new(),
        audio: vec![],
        warnings: vec![],
    }
}
#[test]
fn ranges_use_integer_clip_mapping_and_keep_gaps() {
    let mut p = plan();
    assert_eq!(
        p.window(&p.document.clips[0]).unwrap(),
        Some((2_000_000, 4_000_000))
    );
    p.start = 3_000_000;
    p.end = 3_500_000;
    assert_eq!(
        p.window(&p.document.clips[0]).unwrap(),
        Some((3_000_000, 3_500_000))
    );
    assert_eq!(
        p.document.clips[0]
            .timeline_to_source_us(3_000_000)
            .unwrap(),
        Some(3_000_000)
    );
    assert_eq!(
        caption_states(&p, &p.document.clips[0]).unwrap(),
        vec![(3_000_000, 3_500_000, "hello".into())]
    );
}
#[test]
fn hidden_tracks_and_order_are_shared_with_preview() {
    let mut p = plan();
    let mut c = p.document.clips[0].clone();
    c.id = Uuid::new_v4().to_string();
    let mut t = p.document.tracks[0].clone();
    t.id = Uuid::new_v4().to_string();
    t.order = 5;
    t.name = "top".into();
    c.track_id = t.id.clone();
    p.document.tracks.push(t);
    p.document.clips.insert(0, c);
    assert_eq!(p.visual_clips()[0].track_id, p.document.tracks[0].id);
    p.document.tracks[0].enabled = false;
    assert_eq!(p.visual_clips().len(), 1);
}
#[test]
fn solo_and_implicit_voice_follow_preview_rules() {
    let mut p = plan();
    let c = &p.document.clips[0];
    let mut a = audio_settings(&p.document, c);
    assert!(audible(&p.document, c, &a));
    a.muted = true;
    assert!(!audible(&p.document, c, &a));
    a.muted = false;
    let mut t = p.document.tracks[0].clone();
    t.id = Uuid::new_v4().to_string();
    t.kind = TrackKind::Subtitle;
    t.solo = true;
    p.document.tracks.push(t);
    assert!(audible(&p.document, &p.document.clips[0], &a));
    p.document.tracks[1].kind = TrackKind::Audio;
    assert!(!audible(&p.document, &p.document.clips[0], &a));
    p.document.tracks[1].enabled = false;
    assert!(audible(&p.document, &p.document.clips[0], &a));
}

fn generate(path: &Path, filter: &str, audio: Option<&str>, fps: &str, codec: &str) {
    let mut cmd = crate::runtime_paths::ffmpeg_command().unwrap();
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i", filter]);
    if let Some(audio) = audio {
        cmd.args(["-f", "lavfi", "-i", audio, "-c:a", "aac", "-shortest"]);
    }
    let out = cmd
        .args(["-t", "4", "-r", fps, "-c:v", codec, "-pix_fmt", "yuv420p"])
        .arg(path)
        .output_contained(Duration::from_secs(30))
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn fixture() -> (tempfile::TempDir, ProjectStore, Project, String, String) {
    let (dir, store, mut p, a, b) = crate::video_content::tests::fixture();
    let root = store.data_directory(&p.id, "media/source").unwrap();
    generate(
        &root.join("a.mp4"),
        "color=red:s=320x180:r=24:d=4",
        Some("sine=frequency=440:duration=4"),
        "24",
        "libx264",
    );
    generate(
        &root.join("b.mp4"),
        "color=blue:s=160x120:r=30:d=4",
        Some("sine=frequency=880:duration=4"),
        "30",
        "libx265",
    );
    p.source.width = Some(320);
    p.source.height = Some(180);
    p.source.duration = Some(4.0);
    p.source.checksum_sha256 = None;
    p.source.audio_streams=serde_json::from_value(serde_json::json!([{"streamIndex":1,"codec":"aac","language":null,"title":null,"channels":1,"sampleRate":44100}])).unwrap();
    let source = p.source.clone();
    let doc = p.timeline.as_mut().unwrap();
    doc.settings.width = 320;
    doc.settings.height = 180;
    doc.settings.frame_rate = Rational {
        numerator: 25,
        denominator: 1,
    };
    for (i, asset) in doc.assets.iter_mut().enumerate() {
        asset.duration_us = Some(4_000_000);
        asset.metadata.duration = Some(4.0);
        asset.metadata.checksum_sha256 = None;
        asset.metadata.width = Some(if i == 0 { 320 } else { 160 });
        asset.metadata.height = Some(if i == 0 { 180 } else { 120 });
        asset.metadata.audio_streams = source.audio_streams.clone();
        asset.frame_rate = None;
    }
    let ca = &mut doc.clips[0];
    ca.start_us = 0;
    ca.source_in_us = 1_000_000;
    ca.source_out_us = 3_000_000;
    ca.rate = Rational {
        numerator: 2,
        denominator: 1,
    };
    let cb = &mut doc.clips[1];
    cb.start_us = 2_000_000;
    cb.source_in_us = 0;
    cb.source_out_us = 2_000_000;
    p = store.update_project(&p).unwrap();
    (dir, store, p, a, b)
}
fn frame(path: &Path, at: f64) -> Vec<u8> {
    let out = crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args(["-v", "error", "-ss", &at.to_string(), "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output_contained(Duration::from_secs(30))
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn pixel(rgb: &[u8], w: usize, x: usize, y: usize) -> [u8; 3] {
    rgb[(y * w + x) * 3..(y * w + x) * 3 + 3]
        .try_into()
        .unwrap()
}

fn pcm(path: &Path) -> Vec<f32> {
    let out = crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"])
        .output_contained(Duration::from_secs(30))
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn rms(samples: &[f32], from: f64, to: f64) -> f64 {
    let part = &samples[(from * 48000.0) as usize..(to * 48000.0) as usize];
    (part.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / part.len() as f64).sqrt()
}
fn tone(samples: &[f32], from: f64, to: f64, hz: f64) -> f64 {
    let part = &samples[(from * 48000.0) as usize..(to * 48000.0) as usize];
    let (re, im) = part
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(re, im), (i, v)| {
            let phase = i as f64 * hz * std::f64::consts::TAU / 48000.0;
            (re + *v as f64 * phase.cos(), im + *v as f64 * phase.sin())
        });
    2.0 * (re * re + im * im).sqrt() / part.len() as f64
}
fn options() -> ExportOptions {
    ExportOptions {
        encoder: VideoEncoder::Cpu,
        quality: "fast".into(),
        timeline: Some(TimelineExportOptions {
            width: 320,
            height: 180,
            frame_rate: Rational {
                numerator: 25,
                denominator: 1,
            },
            start_us: None,
            end_us: None,
        }),
        ..Default::default()
    }
}

#[test]
#[ignore = "Requires real FFmpeg/FFprobe; checks TTS snapshot and audio DSP."]
fn media_voice_speed_duck_fade_and_missing_assets() {
    use crate::transcript_service::{TranscriptTts, TranscriptTtsStatus};
    let (_dir, store, mut p, a, _b) = fixture();
    p.dubbing.enabled = true;
    p.dubbing.ducking_enabled = true;
    p.dubbing.ducking_amount = 0.9;
    let doc = p.timeline.as_mut().unwrap();
    let mut audio = audio_settings(doc, &doc.clips[0]);
    audio.volume = 1.0;
    audio.muted = false;
    audio.fade_in_us = 100_000;
    audio.fade_out_us = 100_000;
    audio.ducking_enabled = true;
    audio.ducking_amount = 0.9;
    audio.duck_attack_us = 30_000;
    audio.duck_release_us = 60_000;
    audio.voice_speed = 1.25;
    doc.clips[0].audio = Some(audio);
    p = store.update_project(&p).unwrap();
    let scoped = store.for_video(&p.id, Some(&a)).unwrap();
    let relative = scoped.content_path("audio/tts/voice.wav");
    let voice = store.project_directory(&p.id).unwrap().join(&relative);
    fs::create_dir_all(voice.parent().unwrap()).unwrap();
    let mut wave = hound::WavWriter::create(
        &voice,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..38_400 {
        wave.write_sample(if (4800..33_600).contains(&i) {
            0.2 * (i as f32 * 660.0 * std::f32::consts::TAU / 48000.0).sin()
        } else {
            0.0
        })
        .unwrap();
    }
    wave.finalize().unwrap();
    let mut segment =
        TranscriptSegment::new(1.2, 2.0, "Original".into(), TranscriptSourceType::Asr);
    segment.translated = "VOICE".into();
    segment.tts = Some(TranscriptTts {
        audio_path: Some(relative),
        duration: Some(0.8),
        status: Some(TranscriptTtsStatus::Ready),
        fingerprint: None,
        voice: None,
        speed: None,
        text_hash: None,
        generated_at: None,
        warning: None,
        provider: None,
        model_id: None,
        model_revision: None,
    });
    transcript_service::save_transcript(
        &scoped,
        &p.id,
        Transcript {
            schema_version: 1,
            project_id: p.id.clone(),
            video_content_id: Some(a.clone()),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: Some("en".into()),
            video_checksum: None,
            segments: vec![segment],
        },
    )
    .unwrap();
    let opt = options();
    let snapshot = capture(&store, &p.id, &opt).unwrap();
    let v = snapshot.plan.audio.iter().find(|r| r.voice).unwrap();
    assert_eq!((v.start, v.end), (100_000, 420_000));
    assert!((v.speed - 2.5).abs() < 1e-9);
    let frozen = fs::read(&v.path).unwrap();
    fs::write(&voice, b"replaced while exporting").unwrap();
    assert_eq!(fs::read(&v.path).unwrap(), frozen);
    let (output, _) = render(&snapshot, &opt, &AtomicU8::new(0), |_, _, _| {}).unwrap();
    let samples = pcm(Path::new(&output));
    let normal = tone(&samples, 0.6, 0.8, 440.0);
    let ducked = tone(&samples, 0.2, 0.3, 440.0);
    assert!(normal > 0.05, "normal {normal}");
    assert!(ducked < normal * 0.25, "ducked {ducked}, normal {normal}");
    assert!(tone(&samples, 0.2, 0.3, 660.0) > 0.06);
    assert!(rms(&samples, 0.01, 0.03) < rms(&samples, 0.07, 0.09) * 0.6);
    assert!(rms(&samples, 0.96, 0.98) < rms(&samples, 0.7, 0.8) * 0.6);
    assert!(rms(&samples, 1.3, 1.8) < 0.002);
    // Gaps export as silent canvas; muted or hidden missing sources aren't required.
    let mut gap = options();
    gap.timeline.as_mut().unwrap().start_us = Some(1_200_000);
    gap.timeline.as_mut().unwrap().end_us = Some(1_800_000);
    assert!(
        !build(&store, &p.id, &gap)
            .unwrap()
            .validation()
            .expected_audio
    );
    let source = store
        .project_directory(&p.id)
        .unwrap()
        .join("media/source/b.mp4");
    fs::remove_file(&source).unwrap();
    let error = build(&store, &p.id, &opt).err().unwrap();
    assert!(error.contains("Nguồn"));
    store.edit_project(&p.id,|p|{p.timeline.as_mut().unwrap().clips.retain(|c|!matches!(&c.content,ClipContent::Video{video_content_id,..} if video_content_id!=&a));Ok(())}).unwrap();
    assert!(build(&store, &p.id, &opt).is_ok());
}

#[test]
#[ignore = "Requires real FFmpeg/FFprobe; run separately for T09 media acceptance."]
fn media_export_snapshot_range_layers_captions_audio_and_cancellation() {
    let (_dir, store, mut p, a, _b) = fixture();
    // Image layer above A, with crop/position/scale; B keeps a different source codec/FPS/aspect.
    let green = store
        .data_directory(&p.id, "media/images")
        .unwrap()
        .join("ảnh xanh.png");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([0, 255, 0, 255]))
        .save(&green)
        .unwrap();
    let doc = p.timeline.as_mut().unwrap();
    let mut asset = doc.assets[0].clone();
    asset.id = Uuid::new_v4().to_string();
    asset.kind = MediaKind::Image;
    asset.relative_path = "media/images/ảnh xanh.png".into();
    asset.duration_us = None;
    asset.metadata.video_path = Some(asset.relative_path.clone());
    asset.metadata.width = Some(40);
    asset.metadata.height = Some(20);
    asset.metadata.duration = None;
    asset.metadata.audio_streams.clear();
    let image_id = asset.id.clone();
    doc.assets.push(asset);
    let top = Track {
        id: Uuid::new_v4().to_string(),
        kind: TrackKind::Visual,
        name: "image".into(),
        order: 1,
        locked: false,
        enabled: true,
        muted: false,
        solo: false,
    };
    let mut clip = doc.clips[0].clone();
    clip.id = Uuid::new_v4().to_string();
    clip.track_id = top.id.clone();
    clip.start_us = 0;
    clip.source_in_us = 0;
    clip.source_out_us = 800_000;
    clip.rate = Rational::default();
    clip.content = ClipContent::Image { asset_id: image_id };
    clip.transform.scale_x = 0.25;
    clip.transform.scale_y = 0.25;
    clip.transform.x = 0.75;
    clip.transform.y = 0.3;
    clip.transform.rotation = 20.0;
    clip.transform.opacity = 0.8;
    doc.tracks.push(top);
    doc.clips.push(clip);
    let subtitles = Track {
        id: Uuid::new_v4().to_string(),
        kind: TrackKind::Subtitle,
        name: "caption".into(),
        order: 2,
        locked: false,
        enabled: true,
        muted: false,
        solo: false,
    };
    let mut cap = doc.clips[0].clone();
    cap.id = Uuid::new_v4().to_string();
    cap.track_id = subtitles.id.clone();
    cap.content = ClipContent::Subtitles {
        video_content_id: a.clone(),
    };
    cap.transform.y = 0.8;
    cap.style = Some(ClipStyle {
        font_family: "Arial".into(),
        font_size: 18.0,
        font_weight: 700,
        color: "#FFFFFF".into(),
        background_color: "#000000".into(),
        background_opacity: 0.6,
        alignment: "center".into(),
    });
    doc.tracks.push(subtitles);
    doc.clips.push(cap);
    p = store.update_project(&p).unwrap();
    let scoped = store.for_video(&p.id, Some(&a)).unwrap();
    let mut segment =
        TranscriptSegment::new(1.0, 2.0, "Xin chào".into(), TranscriptSourceType::Asr);
    segment.translated = "CHÀO A".into();
    transcript_service::save_transcript(
        &scoped,
        &p.id,
        Transcript {
            schema_version: 1,
            project_id: p.id.clone(),
            video_content_id: Some(a),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: Some("vi".into()),
            video_checksum: None,
            segments: vec![segment],
        },
    )
    .unwrap();
    let opt = options();
    let snapshot = capture(&store, &p.id, &opt).unwrap();
    // Removing original paths and editing placement after capture must not affect the output.
    fs::remove_file(
        store
            .project_directory(&p.id)
            .unwrap()
            .join("media/source/a.mp4"),
    )
    .unwrap();
    store
        .edit_project(&p.id, |p| {
            p.timeline.as_mut().unwrap().clips[0].start_us = 5_000_000;
            Ok(())
        })
        .unwrap();
    let state = AtomicU8::new(crate::render_service::JOB_RUNNING);
    let (out, duration) = render(&snapshot, &opt, &state, |_, _, _| {}).unwrap();
    assert_eq!(duration, 4.0);
    let audio = pcm(Path::new(&out));
    assert!(tone(&audio, 0.2, 0.6, 440.0) > 0.05);
    assert!(rms(&audio, 1.2, 1.8) < 0.002);
    assert!(tone(&audio, 2.2, 2.6, 880.0) > 0.05);
    assert_eq!(
        state.load(Ordering::SeqCst),
        crate::render_service::JOB_COMMITTED
    );
    let red = frame(Path::new(&out), 0.2);
    assert!(pixel(&red, 320, 10, 10)[0] > 200);
    let green = pixel(&red, 320, 240, 54);
    assert!(green[1] > 150, "{green:?}");
    let gap = frame(Path::new(&out), 1.5);
    assert!(pixel(&gap, 320, 160, 90).iter().all(|v| *v < 10));
    let blue = frame(Path::new(&out), 2.5);
    assert!(pixel(&blue, 320, 160, 90)[2] > 200);
    assert!(pixel(&blue, 320, 10, 90).iter().all(|v| *v < 10));
    let caption = frame(Path::new(&out), 0.2);
    assert!(
        caption
            .chunks(3)
            .skip(130 * 320)
            .take(28 * 320)
            .filter(|p| p.iter().all(|v| *v > 180))
            .count()
            > 15
    );
    // A range retains absolute source/caption/audio timing, but rebases output timestamps.
    let mut selected = opt.clone();
    selected.timeline.as_mut().unwrap().start_us = Some(500_000);
    selected.timeline.as_mut().unwrap().end_us = Some(2_500_000);
    selected.video_codec = crate::hardware_acceleration::VideoCodec::Hevc;
    selected.timeline.as_mut().unwrap().width = 640;
    selected.timeline.as_mut().unwrap().height = 360;
    selected.timeline.as_mut().unwrap().frame_rate = Rational {
        numerator: 30000,
        denominator: 1001,
    };
    let mut range = snapshot;
    range.plan.start = 500_000;
    range.plan.end = 2_500_000;
    range.plan.options = selected.timeline.clone().unwrap();
    let (r, d) = render(&range, &selected, &AtomicU8::new(0), |_, _, _| {}).unwrap();
    assert_eq!(d, 2.0);
    assert!(pixel(&frame(Path::new(&r), 0.1), 640, 10, 10)[0] > 200);
    assert!(pixel(&frame(Path::new(&r), 0.8), 640, 320, 180)
        .iter()
        .all(|v| *v < 10));
    assert!(pixel(&frame(Path::new(&r), 1.8), 640, 320, 180)[2] > 200);
    // Cancel after FFmpeg has started and before publication; no corrupt final or partial remains.
    range.plan.start = 0;
    range.plan.end = 60_000_000;
    range.plan.document.clips[1].source_out_us = 4_000_000;
    let before = fs::read_dir(&range.output_directory).unwrap().count();
    let cancelled = AtomicU8::new(0);
    let result = render(&range, &opt, &cancelled, |p, _, _| {
        if p > 0.0 {
            cancelled.store(crate::render_service::JOB_CANCELLED, Ordering::SeqCst);
        }
    });
    assert!(result.is_err());
    assert_eq!(
        fs::read_dir(&range.output_directory).unwrap().count(),
        before
    );
    assert!(fs::read_dir(&range.output_directory).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("partial")));
}
