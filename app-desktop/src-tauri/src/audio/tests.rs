use super::*;
use mixer::{Mix, Mixer};
use std::{path::Path, time::Instant};
use timeline::Voice;
fn settings() -> ProcessingSettings {
    ProcessingSettings {
        normalize: false,
        ..Default::default()
    }
}
fn mix() -> Mix {
    Mix {
        original: true,
        dubbing: true,
        original_gain: 1.0,
        dub_gain: 1.0,
        reduction: 0.0,
        duck: 0.0,
        settings: settings(),
    }
}
fn wav(path: &Path, seconds: f64, channels: u16, rate: u32, f: impl Fn(usize) -> f32) {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..(seconds * rate as f64).round() as usize {
        for _ in 0..channels {
            w.write_sample(f(i)).unwrap();
        }
    }
    w.finalize().unwrap();
}
fn voice(path: &Path, start: f64) -> Voice {
    let duration = signal::inspect(path).unwrap().duration;
    Voice {
        id: "one".into(),
        path: path.into(),
        start,
        end: start + duration,
        speech_start: start,
        speech_end: start + duration,
    }
}
fn collect(mut mixer: Mixer) -> Vec<f32> {
    let mut output = Vec::new();
    loop {
        let block = mixer.next_block(&|| false).unwrap();
        if block.is_empty() {
            break;
        }
        output.extend(block);
    }
    output
}
#[test]
fn short_speech_is_never_stretched_and_overflow_keeps_content() {
    assert_eq!(processing::tempo(0.7, 1.0, 1.0, true, 1.2), (1.0, false));
    let (tempo, overflow) = processing::tempo(1.08, 1.0, 1.0, true, 1.2);
    assert!((tempo - 1.08).abs() < 0.0001);
    assert!(!overflow);
    assert_eq!(processing::tempo(3.0, 1.0, 1.0, true, 1.2), (1.2, true));
    assert_eq!(processing::tempo(3.0, 1.0, 1.0, false, 1.2), (1.0, true));
    assert_eq!(processing::tempo(2.0, 1.0, 2.0, false, 1.2), (2.0, false));
}
#[test]
fn dialogue_reduction_and_volume_have_linear_meaning() {
    let mut m = mix();
    for (amount, expected) in [(0.0, 0.5), (0.5, 0.35), (1.0, 0.2)] {
        m.reduction = amount;
        assert!((mixer::mix_sample(0.5, 0.3, 0.0, &m, 0.0) - expected).abs() < 1e-6);
    }
    m.reduction = 0.0;
    m.original_gain = 0.5;
    assert_eq!(mixer::mix_sample(0.5, 0.0, 0.0, &m, 0.0), 0.25);
    m.original = false;
    m.dub_gain = 0.5;
    assert_eq!(mixer::mix_sample(0.5, 0.3, 0.4, &m, 1.0), 0.2);
}
#[test]
fn ducking_attack_release_and_overlap_are_smooth() {
    let intervals = vec![(1.0, 2.0), (1.5, 2.5)];
    assert_eq!(mixer::duck_envelope(0.9, &intervals, 0.06, 0.18), 0.0);
    assert!((mixer::duck_envelope(0.97, &intervals, 0.06, 0.18) - 0.5).abs() < 1e-8);
    assert_eq!(mixer::duck_envelope(1.8, &intervals, 0.06, 0.18), 1.0);
    assert!((mixer::duck_envelope(2.59, &intervals, 0.06, 0.18) - 0.5).abs() < 1e-8);
    let mut previous = 0.0_f64;
    for n in 0..3000 {
        let current = mixer::duck_envelope(n as f64 / 1000.0, &intervals, 0.06, 0.18);
        assert!((current - previous).abs() < 0.018);
        previous = current;
    }
    let mut m = mix();
    m.duck = 0.75;
    assert_eq!(mixer::mix_sample(0.8, 0.0, 0.0, &m, 1.0), 0.2);
}
#[test]
fn limiter_preserves_quiet_gain_and_bounds_loud_mix() {
    assert_eq!(mixer::limit(0.8), 0.8);
    assert!(mixer::limit(50.0) <= 0.98);
    assert!(mixer::limit(-50.0) >= -0.98);
}
#[test]
fn corrupt_nonfinite_and_empty_wavs_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voice.wav");
    std::fs::write(&path, b"RIFFbrokenWAVE").unwrap();
    assert!(signal::inspect(&path).is_err());
    wav(&path, 0.01, 1, RATE, |_| f32::NAN);
    assert!(signal::inspect(&path).is_err());
    wav(&path, 0.0, 1, RATE, |_| 0.0);
    assert!(signal::inspect(&path).is_err());
    wav(&path, 0.1, 1, RATE, |_| 0.1);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(64).unwrap();
    assert!(signal::inspect(&path).is_err());
}
#[test]
fn silent_source_tts_disabled_and_two_hour_seek_are_bounded() {
    let mut m = mix();
    m.dubbing = false;
    let mut mixer = Mixer::new(None, None, vec![], m, 7199.9, 7200.0).unwrap();
    let mut frames = 0;
    loop {
        let block = mixer.next_block(&|| false).unwrap();
        if block.is_empty() {
            break;
        }
        assert!(block.len() <= BLOCK * 2);
        assert!(block.iter().all(|v| *v == 0.0));
        frames += block.len() / 2;
    }
    assert_eq!(frames, 4800);
}
#[test]
fn preview_stream_and_offline_pcm_are_identical_and_seek_discards_old_audio() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voice.wav");
    wav(&path, 0.7, 1, RATE, |i| if i < 240 { 0.4 } else { 0.1 });
    let voices = vec![voice(&path, 0.2), voice(&path, 1.2)];
    let pcm = collect(Mixer::new(None, None, voices.clone(), mix(), 0.0, 2.0).unwrap());
    let out = temp.path().join("mixed.wav");
    mixer::render(
        None,
        None,
        voices.clone(),
        mix(),
        2.0,
        &out,
        || false,
        |_| {},
    )
    .unwrap();
    let mut reader = signal::open(&out).unwrap();
    let mut actual = Vec::new();
    while let Some(v) = signal::sample(&mut reader).unwrap() {
        actual.push(v);
    }
    assert_eq!(actual, pcm);
    let seeked = collect(Mixer::new(None, None, voices, mix(), 1.0, 2.0).unwrap());
    assert_eq!(seeked, &pcm[RATE as usize * 2..]);
    // First consonant attack, including its first sample, survives mixing.
    assert_eq!(pcm[(0.2 * RATE as f64) as usize * 2], 0.4);
    assert_eq!(signal::inspect(&out).unwrap().duration, 2.0);
}
#[test]
fn cancellation_preserves_output_and_stem_must_match_timeline() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("mixed.wav");
    assert!(mixer::render(None, None, vec![], mix(), 7200.0, &out, || true, |_| {}).is_err());
    assert!(!out.exists());
    let stem = temp.path().join("speech.wav");
    wav(&stem, 0.1, 2, RATE, |_| 0.1);
    assert!(signal::validate_stem(&stem, 3.0).is_err());
    assert!(signal::validate_stem(&stem, 0.1).is_ok());
}
#[test]
fn model_and_project_settings_are_backward_compatible() {
    let old: crate::project_service::DubbingSettings =
        serde_json::from_str(r#"{"enabled":true,"originalVolume":0.5}"#).unwrap();
    assert_eq!(old.audio_processing, ProcessingSettings::default());
    let json = serde_json::to_string(&old).unwrap();
    assert_eq!(
        serde_json::from_str::<crate::project_service::DubbingSettings>(&json).unwrap(),
        old
    );
}

#[test]
fn enabling_voice_mid_playback_does_not_resume_old_samples() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voice.wav");
    wav(&path, 1.0, 1, RATE, |i| i as f32 / RATE as f32 * 0.5);
    let voices = vec![voice(&path, 0.0)];
    let reference = collect(Mixer::new(None, None, voices.clone(), mix(), 0.0, 1.0).unwrap());
    let mut m = Mixer::new(None, None, voices, mix(), 0.0, 1.0).unwrap();
    m.next_block(&|| false).unwrap();
    m.mix.dubbing = false;
    m.next_block(&|| false).unwrap();
    m.mix.dubbing = true;
    let resumed = m.next_block(&|| false).unwrap();
    assert_eq!(resumed, &reference[BLOCK * 4..BLOCK * 6]);
}

#[test]
#[ignore = "Requires FFmpeg; final AAC encoding and cached legacy ingest"]
fn final_aac_preserves_attack_and_legacy_assets_are_ingested_once() {
    let temp = tempfile::tempdir().unwrap();
    let raw = temp.path().join("legacy.wav");
    wav(&raw, 0.7, 1, 24000, |i| {
        if i < 480 {
            (i as f32 * 0.19).sin() * 0.4
        } else {
            (i as f32 * 0.07).sin() * 0.1
        }
    });
    let original = std::fs::read(&raw).unwrap();
    let info = signal::inspect(&raw).unwrap();
    let normalized = processing::ingest_legacy(&raw, &info).unwrap();
    let modified = std::fs::metadata(&normalized).unwrap().modified().unwrap();
    assert_eq!(normalized, processing::ingest_legacy(&raw, &info).unwrap());
    assert_eq!(
        modified,
        std::fs::metadata(&normalized).unwrap().modified().unwrap()
    );
    assert_eq!(original, std::fs::read(&raw).unwrap());
    let processed = temp.path().join("processed.wav");
    let report =
        processing::process(&raw, &processed, 1.0, 0.5, 1.0, true, &settings(), || false).unwrap();
    let mut voice = voice(&processed, 0.5 - report.lead_seconds);
    voice.speech_start = voice.start + report.processed.speech_start.unwrap();
    voice.speech_end = voice.start + report.processed.speech_end.unwrap();
    let master = temp.path().join("master.wav");
    mixer::render(
        None,
        None,
        vec![voice],
        mix(),
        10.0,
        &master,
        || false,
        |_| {},
    )
    .unwrap();
    let encoded = temp.path().join("output.m4a");
    let decoded = temp.path().join("decoded.wav");
    assert!(crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args(["-v", "error", "-y", "-i"])
        .arg(&master)
        .args(["-c:a", "aac", "-b:a", "192k"])
        .arg(&encoded)
        .status()
        .unwrap()
        .success());
    assert!(crate::runtime_paths::ffmpeg_command()
        .unwrap()
        .args(["-v", "error", "-y", "-i"])
        .arg(&encoded)
        .args(["-c:a", "pcm_f32le"])
        .arg(&decoded)
        .status()
        .unwrap()
        .success());
    let signal = signal::inspect(&decoded).unwrap();
    assert!((signal.duration - 10.0).abs() < 0.03);
    assert!(
        (signal.speech_start.unwrap() - 0.5).abs() < 0.03,
        "AAC must retain onset timing"
    );
    let mut reader = signal::open(&decoded).unwrap();
    reader.seek(24000).unwrap();
    let mut attack = 0.0_f32;
    for _ in 0..1920 {
        attack = attack.max(signal::sample(&mut reader).unwrap().unwrap().abs());
    }
    assert!(
        attack > 0.3,
        "AAC must preserve initial consonant-like transient"
    );
}

#[test]
#[ignore = "Opt-in real Vietnamese VieNeu WAVs; JC_AUDIO_TTS_QA_DIR must point to smoke outputs"]
fn real_vietnamese_raw_fit_and_mixed_waveforms() {
    let folder = std::path::PathBuf::from(
        std::env::var("JC_AUDIO_TTS_QA_DIR").expect("Run scripts/audio-engine-tts-smoke.py first"),
    );
    for n in 0..4 {
        let raw = folder.join(format!("raw-{n}.wav"));
        let info = signal::inspect(&raw).unwrap();
        for (label, slot) in [
            ("natural", info.duration + 0.4),
            ("fit", info.duration / 1.12),
        ] {
            let out = folder.join(format!("{label}-{n}.wav"));
            for p in [&out, &out.with_extension("audio.json")] {
                let _ = std::fs::remove_file(p);
            }
            let r = processing::process(&raw, &out, slot, 1.0, 1.0, true, &settings(), || false)
                .unwrap();
            assert!(r.processed.peak > info.peak * 0.65);
            assert!(
                (r.processed.speech_start.unwrap() * r.tempo - info.speech_start.unwrap() - 0.1)
                    .abs()
                    < 0.045
            );
            assert!(r.processed.speech_end.unwrap() * r.tempo > info.speech_end.unwrap() - 0.08);
            if label == "natural" {
                assert_eq!(r.tempo, 1.0);
            }
            let mixed = folder.join(format!("mixed-{label}-{n}.wav"));
            let _ = std::fs::remove_file(&mixed);
            mixer::render(
                None,
                None,
                vec![voice(&out, 1.0 - r.lead_seconds)],
                mix(),
                info.duration + 2.0,
                &mixed,
                || false,
                |_| {},
            )
            .unwrap();
            println!(
                "VieNeu phrase {n} {label}: raw={:.3}s playback={:.3}s rate={:.3} onset={:?}",
                info.duration, r.processed.duration, r.tempo, r.processed.speech_start
            );
        }
    }
}
#[test]
#[ignore = "Requires FFmpeg; no AI model or network"]
fn ffmpeg_padding_atempo_attack_and_raw_cache_regression() {
    let temp = tempfile::tempdir().unwrap();
    let raw = temp.path().join("raw.wav");
    wav(&raw, 0.7, 1, RATE, |i| {
        if i < 960 {
            0.4
        } else {
            (i as f32 * 0.08).sin() * 0.1
        }
    });
    let bytes = std::fs::read(&raw).unwrap();
    let output = temp.path().join("processed.wav");
    let r = processing::process(&raw, &output, 1.0, 1.0, 1.0, true, &settings(), || false).unwrap();
    assert_eq!(r.tempo, 1.0);
    assert!(!r.overflow);
    assert!((r.processed.duration - 0.86).abs() < 0.001);
    let mut reader = signal::open(&output).unwrap();
    reader.seek(4800).unwrap();
    assert!((signal::sample(&mut reader).unwrap().unwrap() - 0.4).abs() < 1e-6);
    let fitted = temp.path().join("fitted.wav");
    let fit =
        processing::process(&raw, &fitted, 0.62, 1.0, 1.0, true, &settings(), || false).unwrap();
    assert!(fit.tempo > 1.0 && fit.tempo <= 1.2);
    assert!(fit.processed.speech_start.unwrap() < 0.13);
    assert!(
        fit.processed.peak > 0.35,
        "WSOLA must retain the attack pulse"
    );
    assert_eq!(bytes, std::fs::read(&raw).unwrap());
    assert_eq!(processing::report(&fitted).unwrap().tempo, fit.tempo);
    assert!(processing::process(
        &raw,
        &temp.path().join("cancelled.wav"),
        1.0,
        1.0,
        1.0,
        true,
        &settings(),
        || true
    )
    .is_err());
}
#[test]
#[ignore = "Requires FFmpeg; stereo/original/dialogue decode integration"]
fn ffmpeg_original_dialogue_seek_failure_and_pcm_parity() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("source.wav");
    let stem = temp.path().join("stem.wav");
    wav(&original, 2.0, 2, RATE, |i| {
        if i < RATE as usize {
            0.6
        } else {
            0.4
        }
    });
    wav(&stem, 2.0, 2, RATE, |_| 0.2);
    let mut m = mix();
    m.reduction = 0.5;
    m.original_gain = 0.5;
    m.dubbing = false;
    let full =
        collect(Mixer::new(Some(&original), Some(&stem), vec![], m.clone(), 0.0, 2.0).unwrap());
    assert!((full[0] - 0.25).abs() < 1e-6);
    let seeked =
        collect(Mixer::new(Some(&original), Some(&stem), vec![], m.clone(), 1.0, 2.0).unwrap());
    assert_eq!(seeked, &full[RATE as usize * 2..]);
    let output = temp.path().join("master.wav");
    mixer::render(
        Some(&original),
        Some(&stem),
        vec![],
        m,
        2.0,
        &output,
        || false,
        |_| {},
    )
    .unwrap();
    let mut reader = signal::open(&output).unwrap();
    let mut actual = Vec::new();
    while let Some(v) = signal::sample(&mut reader).unwrap() {
        actual.push(v);
    }
    assert_eq!(full, actual);
    let bad = temp.path().join("bad.wav");
    std::fs::write(&bad, b"bad").unwrap();
    let mut bad_mixer = Mixer::new(Some(&bad), None, vec![], mix(), 0.0, 1.0).unwrap();
    assert!(bad_mixer.next_block(&|| false).is_err());
}
#[test]
#[ignore = "Optional CPU/disk benchmark; 100 voices and a complete two-hour timeline"]
fn benchmark_100_voices_and_two_hour_stream() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voice.wav");
    wav(&path, 0.7, 1, RATE, |i| (i as f32 * 0.06).sin() * 0.1);
    let voices: Vec<_> = (0..100).map(|i| voice(&path, i as f64)).collect();
    let started = Instant::now();
    let mut m = Mixer::new(None, None, voices, mix(), 0.0, 100.0).unwrap();
    let mut frames = 0;
    let mut peak_block = 0;
    loop {
        let b = m.next_block(&|| false).unwrap();
        if b.is_empty() {
            break;
        }
        frames += b.len() / 2;
        peak_block = peak_block.max(b.len() * 4);
    }
    println!(
        "100 voices: frames={frames} elapsed={:.3}s max PCM block={peak_block} bytes",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let mut m = Mixer::new(None, None, vec![], mix(), 0.0, 7200.0).unwrap();
    let mut frames = 0_u64;
    loop {
        let b = m.next_block(&|| false).unwrap();
        if b.is_empty() {
            break;
        }
        frames += b.len() as u64 / 2;
    }
    assert_eq!(frames, 7200 * RATE as u64);
    println!(
        "2h frames={frames} elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "Opt-in 100 real CPU VieNeu voices; JC_AUDIO_BENCH_DIR points to --count 100 smoke outputs"]
fn benchmark_100_real_cached_voices_processing_and_mix() {
    let folder = std::path::PathBuf::from(
        std::env::var("JC_AUDIO_BENCH_DIR").expect("Run the 100-voice offline smoke first"),
    );
    let temp = tempfile::tempdir().unwrap();
    let began = Instant::now();
    let mut voices = Vec::new();
    for n in 0..100 {
        let output = temp.path().join(format!("processed-{n}.wav"));
        let report = processing::process(
            &folder.join(format!("raw-{n}.wav")),
            &output,
            4.0,
            0.5 + n as f64 * 4.0,
            1.0,
            true,
            &settings(),
            || false,
        )
        .unwrap();
        voices.push(voice(&output, 0.5 + n as f64 * 4.0 - report.lead_seconds));
    }
    let processing_seconds = began.elapsed().as_secs_f64();
    let began = Instant::now();
    let mut m = Mixer::new(None, None, voices, mix(), 0.0, 400.0).unwrap();
    let mut frames = 0_u64;
    let mut max_block_bytes = 0;
    loop {
        let block = m.next_block(&|| false).unwrap();
        if block.is_empty() {
            break;
        }
        frames += block.len() as u64 / 2;
        max_block_bytes = max_block_bytes.max(block.len() * 4);
    }
    let report = serde_json::json!({"voices":100,"processingSeconds":processing_seconds,"mixSeconds":began.elapsed().as_secs_f64(),
        "timelineSeconds":400,"frames":frames,"maxPcmBlockBytes":max_block_bytes,"profile":"debug"});
    std::fs::write(
        folder.join("audio-benchmark.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
    assert_eq!(frames, 400 * RATE as u64);
    assert!(max_block_bytes <= BLOCK * 8);
}
