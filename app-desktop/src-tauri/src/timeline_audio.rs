//! Resolve existing per-source TTS assets; timeline edits never regenerate or rewrite audio.
use crate::{
    audio,
    project_service::ProjectStore,
    transcript_service::{self, Transcript, TranscriptTtsStatus},
};
use serde::Serialize;
use tauri::Manager;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMedia {
    pub segment_id: String,
    pub path: String,
    pub start_us: u64,
    pub end_us: u64,
    pub speech_start_us: u64,
    pub speech_end_us: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceWarning {
    pub segment_id: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMedia {
    pub video_content_id: String,
    pub content_revision: u64,
    pub transcript: Option<Transcript>,
    pub voices: Vec<VoiceMedia>,
    pub warnings: Vec<VoiceWarning>,
}
pub fn resolve(
    store: &ProjectStore,
    project_id: &str,
    video_content_id: &str,
) -> Result<SourceMedia, String> {
    let scoped = store.for_video(project_id, Some(video_content_id))?;
    let project = scoped.get(project_id)?;
    let revision = project
        .timeline
        .as_ref()
        .and_then(|d| d.video_contents.iter().find(|v| v.id == video_content_id))
        .ok_or("Nội dung video không tồn tại.")?
        .revision;
    let transcript = transcript_service::get_transcript(&scoped, project_id)?;
    let mut result = SourceMedia {
        video_content_id: video_content_id.into(),
        content_revision: revision,
        transcript: transcript.clone(),
        voices: Vec::new(),
        warnings: Vec::new(),
    };
    if let Some(transcript) = transcript {
        let mut one = transcript.clone();
        one.segments.clear();
        for segment in &transcript.segments {
            if segment.translated.trim().is_empty() {
                continue;
            }
            if !segment.tts.as_ref().is_some_and(|t| {
                matches!(
                    t.status,
                    Some(TranscriptTtsStatus::Ready | TranscriptTtsStatus::DurationOverflow)
                )
            }) {
                result.warnings.push(VoiceWarning {
                    segment_id: segment.id.clone(),
                    message: "Chưa có giọng đọc mới nhất cho đoạn này.".into(),
                });
                continue;
            }
            if let Some(path) = segment.tts.as_ref().and_then(|t| t.audio_path.as_deref()) {
                if let Err(message) = scoped.validate_audio_owner(project_id, path) {
                    result.warnings.push(VoiceWarning {
                        segment_id: segment.id.clone(),
                        message,
                    });
                    continue;
                }
            }
            one.segments = vec![segment.clone()];
            match audio::timeline::voices(
                &scoped,
                project_id,
                &one,
                false,
                project.source.duration.unwrap_or(0.0),
            ) {
                Ok(voices) => {
                    for voice in voices {
                        result.voices.push(VoiceMedia {
                            segment_id: voice.id,
                            path: voice.path.to_string_lossy().into_owned(),
                            start_us: crate::timeline::seconds_to_time_us(voice.start)?,
                            end_us: crate::timeline::seconds_to_time_us(voice.end)?,
                            speech_start_us: crate::timeline::seconds_to_time_us(
                                voice.speech_start,
                            )?,
                            speech_end_us: crate::timeline::seconds_to_time_us(voice.speech_end)?,
                        });
                    }
                }
                Err(message) => result.warnings.push(VoiceWarning {
                    segment_id: segment.id.clone(),
                    message,
                }),
            }
        }
    }
    Ok(result)
}
#[tauri::command]
pub async fn get_timeline_source_media(
    app: tauri::AppHandle,
    project_id: String,
    video_content_id: String,
) -> Result<SourceMedia, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = resolve(
            &crate::commands::store(&app)?,
            &project_id,
            &video_content_id,
        )?;
        for voice in &result.voices {
            app.asset_protocol_scope()
                .allow_file(&voice.path)
                .map_err(|e| format!("Không mở được giọng đọc: {e}"))?;
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::TranscriptSourceType;
    use crate::transcript_service::{TranscriptSegment, TranscriptTts};
    use std::fs;
    fn save_voice(
        store: &ProjectStore,
        project: &str,
        seconds: f64,
    ) -> (String, std::path::PathBuf) {
        let view = store.get(project).unwrap();
        let relative = store.content_path("audio/tts/voice.wav");
        let path = store.project_directory(project).unwrap().join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..(seconds * 48000.0) as usize {
            writer
                .write_sample(if i < 4800 {
                    0.0
                } else {
                    0.05 * (i as f32 * 0.1).sin()
                })
                .unwrap();
        }
        writer.finalize().unwrap();
        let mut segment =
            TranscriptSegment::new(2.0, 3.0, "Original".into(), TranscriptSourceType::Asr);
        segment.translated = "Translated".into();
        segment.tts = Some(TranscriptTts {
            audio_path: Some(relative),
            duration: Some(seconds),
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
        let id = segment.id.clone();
        transcript_service::save_transcript(
            store,
            project,
            Transcript {
                schema_version: 1,
                project_id: project.into(),
                video_content_id: store.content_id().map(str::to_owned),
                source_type: TranscriptSourceType::Asr,
                source_track: None,
                language: Some("en".into()),
                video_checksum: view.source.checksum_sha256,
                segments: vec![segment],
            },
        )
        .unwrap();
        (id, path)
    }
    #[test]
    fn source_manifests_preserve_real_wavs_and_timestamps_after_reorder_trim_split_and_reload() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let first = store.for_video(&project.id, Some(&a)).unwrap();
        let second = store.for_video(&project.id, Some(&b)).unwrap();
        let (a_segment, a_path) = save_voice(&first, &project.id, 2.0);
        let (b_segment, b_path) = save_voice(&second, &project.id, 1.0);
        let a_bytes = fs::read(&a_path).unwrap();
        let b_bytes = fs::read(&b_path).unwrap();
        let before = resolve(&store, &project.id, &a).unwrap();
        assert_eq!(before.voices.len(), 1);
        assert!(before.warnings.is_empty());
        assert_eq!(before.voices[0].segment_id, a_segment);
        assert_eq!(
            (before.voices[0].start_us, before.voices[0].end_us),
            (2_000_000, 4_000_000)
        );
        assert!(before.voices[0].speech_start_us >= 2_090_000);
        let project = store.get(&project.id).unwrap();
        let expected =
            crate::timeline_edit::EditLayout::from_document(project.timeline.as_ref().unwrap());
        let mut next = expected.clone();
        next.clips[1].start_us = 0;
        next.clips[0].start_us = 10_000_000;
        next.clips[0].source_in_us = 3_000_000;
        let mut right = next.clips[0].clone();
        right.id = uuid::Uuid::new_v4().to_string();
        right.start_us = 12_000_000;
        right.source_in_us = 5_000_000;
        next.clips[0].source_out_us = 5_000_000;
        next.clips.push(right);
        crate::timeline_edit::apply_layout(&store, &project.id, expected, next).unwrap();
        let after = resolve(&store, &project.id, &a).unwrap();
        let other = resolve(&store, &project.id, &b).unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        assert_eq!(other.voices[0].segment_id, b_segment);
        assert_ne!(other.voices[0].path, a_path.to_string_lossy());
        assert_eq!(fs::read(a_path).unwrap(), a_bytes);
        assert_eq!(fs::read(b_path).unwrap(), b_bytes);
        assert!(resolve(&store, &project.id, "../missing").is_err());
    }
    #[test]
    fn foreign_voice_file_never_enters_another_source_manifest() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let first = store.for_video(&project.id, Some(&a)).unwrap();
        let second = store.for_video(&project.id, Some(&b)).unwrap();
        save_voice(&first, &project.id, 2.0);
        let (_, b_path) = save_voice(&second, &project.id, 1.0);
        let mut transcript = transcript_service::get_transcript(&first, &project.id)
            .unwrap()
            .unwrap();
        transcript.segments[0].tts.as_mut().unwrap().audio_path =
            Some(second.content_path("audio/tts/voice.wav"));
        transcript_service::save_transcript(&first, &project.id, transcript).unwrap();
        let manifest = resolve(&store, &project.id, &a).unwrap();
        assert!(manifest.voices.is_empty());
        assert!(manifest.warnings[0].message.contains("không thuộc"));
        assert_eq!(
            resolve(&store, &project.id, &b).unwrap().voices[0].path,
            b_path.to_string_lossy()
        );
    }
    #[test]
    fn one_corrupt_or_outdated_voice_warns_without_suppressing_other_segments() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let scoped = store.for_video(&project.id, Some(&a)).unwrap();
        let (_, path) = save_voice(&scoped, &project.id, 2.0);
        let mut data = transcript_service::get_transcript(&scoped, &project.id)
            .unwrap()
            .unwrap();
        let mut stale = data.segments[0].clone();
        stale.id = uuid::Uuid::new_v4().to_string();
        stale.start = 5.0;
        stale.end = 6.0;
        stale.tts.as_mut().unwrap().status = Some(TranscriptTtsStatus::Outdated);
        data.segments.push(stale.clone());
        transcript_service::save_transcript(&scoped, &project.id, data).unwrap();
        let ready = resolve(&store, &project.id, &a).unwrap();
        assert_eq!(ready.voices.len(), 1);
        assert!(ready.warnings.iter().any(|w| w.segment_id == stale.id));
        fs::write(path.with_extension("audio.json"), b"invalid report").unwrap();
        let corrupt = resolve(&store, &project.id, &a).unwrap();
        assert!(corrupt.voices.is_empty());
        assert_eq!(corrupt.warnings.len(), 2);
        assert!(resolve(&store, &project.id, &b)
            .unwrap()
            .transcript
            .is_none());
        fs::remove_file(path).unwrap();
        assert!(resolve(&store, &project.id, &a).unwrap().voices.is_empty());
    }
}
