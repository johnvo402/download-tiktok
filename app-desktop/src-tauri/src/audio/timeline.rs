use crate::{
    project_service::ProjectStore,
    transcript_service::{Transcript, TranscriptTtsStatus},
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Voice {
    pub id: String,
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub speech_start: f64,
    pub speech_end: f64,
}
pub fn voices(
    store: &ProjectStore,
    project: &str,
    transcript: &Transcript,
    strict: bool,
    duration: f64,
) -> Result<Vec<Voice>, String> {
    let mut result = Vec::new();
    for segment in &transcript.segments {
        if segment.translated.trim().is_empty() {
            continue;
        }
        let tts = segment.tts.as_ref();
        if !tts.is_some_and(|t| {
            matches!(
                t.status,
                Some(TranscriptTtsStatus::Ready | TranscriptTtsStatus::DurationOverflow)
            )
        }) {
            if strict {
                return Err(format!("Đoạn {} chưa có giọng đọc mới nhất.", segment.id));
            }
            continue;
        }
        let path = crate::video_service::resolve_project_file(
            store,
            project,
            tts.and_then(|t| t.audio_path.as_deref())
                .ok_or("Thiếu tệp giọng đọc.")?,
            "audio/tts",
        )?;
        let info = super::signal::inspect(&path)?;
        let report = super::processing::report(&path);
        if path.with_extension("audio.json").exists() && report.is_none() {
            return Err(format!(
                "Thông tin audio đoạn {} bị hỏng. Hãy tạo lại giọng đọc.",
                segment.id
            ));
        }
        if report
            .as_ref()
            .is_some_and(|r| (r.processed.duration - info.duration).abs() > 0.002)
        {
            return Err(format!(
                "Audio đoạn {} không khớp thông tin đã lưu. Hãy tạo lại giọng đọc.",
                segment.id
            ));
        }
        let lead = report.as_ref().map_or(0.0, |r| r.lead_seconds);
        let start = (segment.start - lead).max(0.0);
        let end = start + info.duration;
        let speech_end = start + info.speech_end.unwrap_or(info.duration);
        if strict && speech_end > duration + 0.025 {
            return Err(format!("Giọng đọc đoạn {} vượt cuối video {:.2}s. Hãy rút gọn hoặc điều chỉnh tốc độ trước khi xuất.", segment.id, speech_end-duration));
        }
        let path = super::processing::ingest_legacy(&path, &info)?;
        result.push(Voice {
            id: segment.id.clone(),
            path,
            start,
            end,
            speech_start: start + info.speech_start.unwrap_or(0.0),
            speech_end,
        });
    }
    result.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(result)
}
pub fn collisions(voices: &[Voice]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (i, a) in voices.iter().enumerate() {
        for b in &voices[i + 1..] {
            if b.start >= a.speech_end {
                break;
            }
            if b.speech_start >= a.speech_end {
                continue;
            }
            if a.speech_start < b.speech_end {
                result.push((a.id.clone(), b.id.clone()));
            }
        }
    }
    result
}
pub fn intervals(voices: &[Voice]) -> Vec<(f64, f64)> {
    let mut input: Vec<_> = voices
        .iter()
        .map(|v| (v.speech_start, v.speech_end))
        .collect();
    input.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut result: Vec<(f64, f64)> = Vec::new();
    for (start, end) in input {
        if let Some(last) = result.last_mut() {
            if start <= last.1 + 0.04 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        result.push((start, end));
    }
    result
}
