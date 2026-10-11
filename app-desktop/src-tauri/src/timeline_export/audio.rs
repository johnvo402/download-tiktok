//! Streaming timeline mix. A late clip consumes two decoder blocks, never minutes of silence in RAM.
use super::{seconds, AudioRegion, Plan};
use crate::audio::{mixer::Decoder, BLOCK, RATE};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

struct Reader {
    decoder: Decoder,
    samples: VecDeque<f32>,
    upmix_gain: f32,
}
impl Reader {
    fn next(&mut self, cancel: &impl Fn() -> bool) -> Result<[f32; 2], String> {
        if self.samples.is_empty() {
            self.samples.extend(self.decoder.block(cancel)?);
        }
        Ok([
            self.samples.pop_front().unwrap_or(0.0) * self.upmix_gain,
            self.samples.pop_front().unwrap_or(0.0) * self.upmix_gain,
        ])
    }
}
struct Curve {
    points: Vec<(f64, f64)>,
    index: usize,
}
impl Curve {
    fn new(plan: &Plan, r: &AudioRegion) -> Self {
        if r.voice || !r.audio.ducking_enabled {
            return Self {
                points: vec![],
                index: 0,
            };
        }
        let attack = r.audio.duck_attack_us as f64;
        let release = r.audio.duck_release_us as f64;
        let mut intervals: Vec<_> = plan
            .audio
            .iter()
            .filter(|v| v.voice && v.speech.1 > v.speech.0)
            .map(|v| (v.speech.0 as f64, v.speech.1 as f64))
            .collect();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged = Vec::<(f64, f64)>::new();
        for (a, b) in intervals {
            if let Some(last) = merged.last_mut() {
                if a <= last.1 {
                    last.1 = last.1.max(b);
                    continue;
                }
            }
            merged.push((a, b));
        }
        let mut cuts = vec![];
        for (a, b) in &merged {
            cuts.extend([*a - attack, *a, *b, *b + release]);
        }
        // Crossings in a short gap are the valley of max(release(previous), attack(next)).
        if attack > 0.0 && release > 0.0 {
            for pair in merged.windows(2) {
                let cross = (attack * pair[0].1 + release * pair[1].0) / (attack + release);
                if cross > pair[1].0 - attack && cross < pair[0].1 + release {
                    cuts.push(cross);
                }
            }
        }
        // Evaluate jumps on the correct side of half-open speech intervals.
        for (a, b) in &merged {
            if attack == 0.0 {
                cuts.push(a - 0.001);
            }
            if release == 0.0 {
                cuts.push(b - 0.001);
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let points = cuts
            .into_iter()
            .map(|t| {
                let gain = merged
                    .iter()
                    .map(|(a, b)| {
                        if t < *a {
                            if attack > 0.0 {
                                1.0 - (*a - t) / attack
                            } else {
                                0.0
                            }
                        } else if t < *b {
                            1.0
                        } else if release > 0.0 {
                            1.0 - (t - *b) / release
                        } else {
                            0.0
                        }
                    })
                    .fold(0.0f64, f64::max)
                    .clamp(0.0, 1.0);
                (t, gain)
            })
            .collect();
        Self { points, index: 0 }
    }
    fn at(&mut self, t: f64) -> f64 {
        if self.points.is_empty() || t < self.points[0].0 {
            return 0.0;
        }
        while self.index + 1 < self.points.len() && t >= self.points[self.index + 1].0 {
            self.index += 1;
        }
        let a = self.points[self.index];
        let Some(b) = self.points.get(self.index + 1) else {
            return a.1;
        };
        a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0)
    }
}
pub fn render(
    plan: &Plan,
    root: &Path,
    cancel: impl Fn() -> bool,
    mut progress: impl FnMut(f64),
) -> Result<Option<PathBuf>, String> {
    let regions: Vec<_> = plan
        .audio
        .iter()
        .filter(|r| r.start < plan.end && r.end > plan.start)
        .collect();
    if regions.is_empty() {
        return Ok(None);
    }
    let frames = ((plan.end - plan.start) as u128 * RATE as u128 + 500_000) / 1_000_000;
    let bytes = frames * 8 + 256;
    if bytes > u32::MAX as u128 {
        return Err(
            "Audio tổng vượt giới hạn WAV. Hãy xuất timeline thành các vùng ngắn hơn.".into(),
        );
    }
    crate::disk_space::require(root, bytes as u64)?;
    let path = root.join("master-audio.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .map_err(|e| e.to_string())?;
    let frame_at = |us: u64| {
        ((us.saturating_sub(plan.start) as u128 * RATE as u128 + 500_000) / 1_000_000) as u64
    };
    let windows: Vec<_> = regions
        .iter()
        .map(|r| {
            (
                frame_at(r.start.max(plan.start)),
                frame_at(r.end.min(plan.end)),
            )
        })
        .collect();
    let mut readers: Vec<Option<Reader>> = (0..regions.len()).map(|_| None).collect();
    let mut curves: Vec<_> = regions.iter().map(|r| Curve::new(plan, r)).collect();
    let fades: Vec<_> = regions
        .iter()
        .map(|r| Ok((r.clip.duration_us()?, r.clip.end_us()?)))
        .collect::<Result<Vec<_>, String>>()?;
    let mut at = 0u64;
    while at < frames as u64 {
        if cancel() {
            return Err("Đã hủy trộn âm thanh.".into());
        }
        let end = (at + BLOCK as u64).min(frames as u64);
        let mut block = vec![[0f64; 2]; (end - at) as usize];
        for (i, r) in regions.iter().enumerate() {
            let (from, to) = windows[i];
            if from >= end || to <= at {
                continue;
            }
            if readers[i].is_none() {
                let start = r.start.max(plan.start);
                let source = r
                    .clip
                    .timeline_to_source_us(start)?
                    .ok_or("Khoảng audio không nằm trong clip.")?;
                let source = if r.voice {
                    seconds(source.saturating_sub(r.source_start)) * r.audio.voice_speed
                } else {
                    seconds(source)
                };
                let mono = if r.voice {
                    hound::WavReader::open(&r.path).is_ok_and(|w| w.spec().channels == 1)
                } else {
                    super::asset_id(&r.clip)
                        .and_then(|id| plan.document.assets.iter().find(|a| a.id == id))
                        .and_then(|a| a.metadata.audio_streams.first())
                        .is_some_and(|s| s.channels == Some(1))
                };
                readers[i] = Some(Reader {
                    decoder: Decoder::timed(
                        &r.path,
                        source,
                        2,
                        Some((seconds(r.end.min(plan.end) - start) * r.speed, r.speed)),
                    )?,
                    samples: VecDeque::new(),
                    // Web Audio duplicates mono into both speakers; swresample's center
                    // channel upmix attenuates by sqrt(2). Undo only that known conversion.
                    upmix_gain: if mono { std::f32::consts::SQRT_2 } else { 1.0 },
                });
            }
            for frame in at.max(from)..end.min(to) {
                let sample = readers[i].as_mut().unwrap().next(&cancel)?;
                let time = plan.start as f64 + frame as f64 * 1e6 / RATE as f64;
                let (duration, clip_end) = fades[i];
                let fade_in = if r.audio.fade_in_us > 0 {
                    (time - r.clip.start_us as f64) / r.audio.fade_in_us.min(duration) as f64
                } else {
                    1.0
                };
                let fade_out = if r.audio.fade_out_us > 0 {
                    (clip_end as f64 - time) / r.audio.fade_out_us.min(duration) as f64
                } else {
                    1.0
                };
                let gain = r.audio.volume
                    * fade_in.min(fade_out).clamp(0.0, 1.0)
                    * (1.0 - r.audio.ducking_amount * curves[i].at(time));
                for ch in 0..2 {
                    block[(frame - at) as usize][ch] += sample[ch] as f64 * gain;
                }
            }
            if to <= end {
                readers[i].take();
            }
        }
        // The preview sums gains directly. Only protect peaks outside the PCM full scale.
        for sample in block {
            for value in sample {
                writer
                    .write_sample(value.clamp(-1.0, 1.0) as f32)
                    .map_err(|e| e.to_string())?;
            }
        }
        at = end;
        if at % (BLOCK as u64 * 48) == 0 {
            progress(at as f64 / frames as f64);
        }
    }
    writer.finalize().map_err(|e| e.to_string())?;
    if cancel() {
        return Err("Đã hủy trộn âm thanh.".into());
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_speech_and_zero_attack_match_the_preview_envelope() {
        let mut plan = super::super::tests::plan();
        let clip = plan.document.clips[0].clone();
        let mut a = super::super::audio_settings(&plan.document, &clip);
        a.ducking_enabled = true;
        a.duck_attack_us = 100_000;
        a.duck_release_us = 200_000;
        let original = AudioRegion {
            clip: clip.clone(),
            path: PathBuf::new(),
            voice: false,
            start: 0,
            end: 4_000_000,
            source_start: 0,
            speed: 1.0,
            speech: (0, 0),
            audio: a.clone(),
        };
        let mut voice = AudioRegion {
            voice: true,
            speech: (1_000_000, 1_100_000),
            ..original.clone()
        };
        plan.audio.push(voice.clone());
        voice.speech = (1_250_000, 1_500_000);
        plan.audio.push(voice);
        let mut curve = Curve::new(&plan, &original);
        for (at, want) in [
            (850_000.0, 0.0),
            (950_000.0, 0.5),
            (1_050_000.0, 1.0),
            (1_150_000.0, 0.75),
            (1_200_000.0, 0.5),
            (1_300_000.0, 1.0),
            (1_600_000.0, 0.5),
            (1_800_000.0, 0.0),
        ] {
            assert!((curve.at(at) - want).abs() < 1e-8, "at {at}");
        }
        let mut original = original;
        original.audio.duck_attack_us = 0;
        original.audio.duck_release_us = 0;
        let mut curve = Curve::new(&plan, &original);
        for (at, want) in [
            (999_999.0, 0.0),
            (1_000_000.0, 1.0),
            (1_100_000.0, 0.0),
            (1_250_000.0, 1.0),
            (1_500_000.0, 0.0),
        ] {
            assert!((curve.at(at) - want).abs() < 1e-8, "zero attack at {at}");
        }
    }
}
