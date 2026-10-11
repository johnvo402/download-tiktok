//! A processing view of one video. The persisted project and its legacy binding
//! never change when a user selects a different clip.
use crate::project_service::{Project, ProjectStore};
use crate::timeline::{MediaKind, VideoSettings};
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobTarget {
    pub video_content_id: Option<String>,
    pub content_revision: Option<u64>,
}

impl ProjectStore {
    pub fn for_video(&self, project_id: &str, content_id: Option<&str>) -> Result<Self, String> {
        let mut scoped = self.clone();
        scoped.video_content_id = content_id.map(str::to_owned).or_else(|| {
            self.get(project_id).ok().and_then(|project| {
                project
                    .timeline?
                    .legacy_binding
                    .map(|binding| binding.video_content_id)
            })
        });
        scoped.get(project_id)?; // Validate identity before IO or registering a job.
        Ok(scoped)
    }

    pub fn content_id(&self) -> Option<&str> {
        self.video_content_id.as_deref()
    }

    pub fn transcript_lock_key(&self, project_id: &str) -> String {
        let identity = self.content_id().map(str::to_owned).or_else(|| {
            self.get(project_id).ok().and_then(|project| {
                project
                    .timeline?
                    .legacy_binding
                    .map(|binding| binding.video_content_id)
            })
        });
        format!("{project_id}:{}", identity.as_deref().unwrap_or("legacy"))
    }

    pub fn content_path(&self, relative: &str) -> String {
        let Some(id) = self.content_id() else {
            return relative.to_owned();
        };
        for prefix in ["subtitles", "audio/tts", "audio/extracted", "cache"] {
            if relative == prefix || relative.starts_with(&format!("{prefix}/")) {
                return format!("{prefix}/contents/{id}{}", &relative[prefix.len()..]);
            }
        }
        relative.to_owned()
    }

    pub fn job_target(&self, project: &Project) -> JobTarget {
        JobTarget {
            video_content_id: self.video_content_id.clone(),
            content_revision: self.content_id().and_then(|id| {
                project
                    .timeline
                    .as_ref()?
                    .video_contents
                    .iter()
                    .find(|content| content.id == id)
                    .map(|content| content.revision)
            }),
        }
    }

    pub fn validate_audio_owner(&self, project_id: &str, relative: &str) -> Result<(), String> {
        let Some(id) = self.content_id() else {
            return Ok(());
        };
        let expected = format!("audio/tts/contents/{id}/");
        let bound = self
            .get(project_id)?
            .timeline
            .and_then(|d| d.legacy_binding)
            .is_some_and(|binding| binding.video_content_id == id);
        if relative.starts_with(&expected)
            || (bound
                && relative.starts_with("audio/tts/")
                && !relative.starts_with("audio/tts/contents/"))
        {
            Ok(())
        } else {
            Err("Audio TTS không thuộc nội dung video được chọn.".into())
        }
    }

    pub(crate) fn project_view(&self, mut project: Project) -> Result<Project, String> {
        let Some(id) = self.content_id() else {
            return Ok(project);
        };
        let document = project
            .timeline
            .as_ref()
            .ok_or("Dự án chưa có dữ liệu timeline.")?;
        let content = document
            .video_contents
            .iter()
            .find(|content| content.id == id)
            .ok_or("Nội dung video không còn tồn tại trong dự án.")?;
        let asset = document
            .assets
            .iter()
            .find(|asset| asset.id == content.asset_id && asset.kind == MediaKind::Video)
            .ok_or("Không tìm thấy nguồn video của nội dung.")?;
        let settings = content.settings.clone();
        project.source = asset.metadata.clone();
        apply_settings(&mut project, &settings);
        Ok(project)
    }

    /// Merge only this video's settings into the latest project. Timeline moves,
    /// imports and changes to other videos are deliberately retained.
    pub(crate) fn merge_video_update(
        &self,
        incoming: &Project,
        existing: &Project,
    ) -> Result<Project, String> {
        let id = self.content_id().ok_or("Chưa chọn nội dung video.")?;
        let baseline = incoming
            .timeline
            .as_ref()
            .and_then(|doc| doc.video_contents.iter().find(|c| c.id == id))
            .ok_or("Thiếu bản gốc của nội dung video.")?;
        let current = existing
            .timeline
            .as_ref()
            .and_then(|doc| doc.video_contents.iter().find(|c| c.id == id))
            .ok_or("Nội dung video không còn tồn tại.")?;
        if baseline != current || incoming.source != self.project_view(existing.clone())?.source {
            return Err("Nội dung video đã thay đổi. Hãy tải lại trước khi lưu.".into());
        }
        let mut merged = existing.clone();
        merged
            .timeline
            .as_mut()
            .unwrap()
            .video_contents
            .iter_mut()
            .find(|c| c.id == id)
            .unwrap()
            .settings = VideoSettings::from_project(incoming);
        Ok(merged)
    }
}

fn apply_settings(project: &mut Project, settings: &VideoSettings) {
    project.transcript = settings.transcript.clone();
    project.translation = settings.translation.clone();
    project.subtitle = settings.subtitle.clone();
    project.dubbing = settings.dubbing.clone();
    project.transcript_settings = settings.transcript_settings.clone();
    project.blur_regions = settings.blur_regions.clone();
    project.logos = settings.logos.clone();
    project.text_overlays = settings.text_overlays.clone();
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::project_service::TranscriptSourceType;
    use crate::timeline::ClipContent;
    use crate::transcript_service::{
        self, Transcript, TranscriptSegment, TranscriptTts, TranscriptTtsStatus,
    };
    use tempfile::TempDir;
    use uuid::Uuid;

    pub(crate) fn fixture() -> (TempDir, ProjectStore, Project, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        let mut project = store.create("Two videos").unwrap();
        project.source.video_path = Some("media/source/a.mp4".into());
        project.source.original_name = Some("A.mp4".into());
        project.source.duration = Some(10.0);
        project.source.checksum_sha256 = Some("a".repeat(64));
        let mut project = store.update_project(&project).unwrap();
        let doc = project.timeline.as_mut().unwrap();
        let a = doc.video_contents[0].id.clone();
        let mut asset = doc.assets[0].clone();
        asset.id = Uuid::new_v4().to_string();
        asset.relative_path = "media/source/b.mp4".into();
        asset.metadata.video_path = Some(asset.relative_path.clone());
        asset.metadata.original_name = Some("B.mp4".into());
        asset.metadata.checksum_sha256 = Some("b".repeat(64));
        let mut content = doc.video_contents[0].clone();
        content.id = Uuid::new_v4().to_string();
        content.asset_id = asset.id.clone();
        let b = content.id.clone();
        let mut clip = doc.clips[0].clone();
        clip.id = Uuid::new_v4().to_string();
        clip.start_us = 10_000_000;
        clip.content = ClipContent::Video {
            asset_id: asset.id.clone(),
            video_content_id: b.clone(),
        };
        doc.assets.push(asset);
        doc.video_contents.push(content);
        doc.clips.push(clip);
        let project = store.update_project(&project).unwrap();
        (dir, store, project, a, b)
    }
    fn transcript(store: &ProjectStore, project: &Project, text: &str) -> Transcript {
        Transcript {
            schema_version: 1,
            project_id: project.id.clone(),
            video_content_id: store.content_id().map(str::to_owned),
            source_type: TranscriptSourceType::Asr,
            source_track: None,
            language: Some("en".into()),
            video_checksum: project.source.checksum_sha256.clone(),
            segments: vec![TranscriptSegment::new(
                0.0,
                1.0,
                text.into(),
                TranscriptSourceType::Asr,
            )],
        }
    }

    #[test]
    fn transcripts_languages_voices_and_cache_paths_are_independent_after_reload() {
        let (_dir, store, project, a, b) = fixture();
        let first = store.for_video(&project.id, Some(&a)).unwrap();
        let second = store.for_video(&project.id, Some(&b)).unwrap();
        for (scoped, language, voice, text) in [
            (&first, "vi", "Ngọc Huyền", "Hello A"),
            (&second, "en", "B voice", "Hello B"),
        ] {
            let mut view = scoped.get(&project.id).unwrap();
            view.translation.target_language = language.into();
            view.dubbing.voice = Some(voice.into());
            let view = scoped.update_project(&view).unwrap();
            transcript_service::save_transcript(
                scoped,
                &project.id,
                transcript(scoped, &view, text),
            )
            .unwrap();
        }
        let root = store.get(&project.id).unwrap();
        assert_eq!(root.source.original_name.as_deref(), Some("A.mp4"));
        assert_eq!(root.translation.target_language, "vi");
        assert_eq!(
            root.timeline
                .as_ref()
                .unwrap()
                .legacy_binding
                .as_ref()
                .unwrap()
                .video_content_id,
            a
        );
        assert_eq!(
            second.get(&project.id).unwrap().translation.target_language,
            "en"
        );
        assert_eq!(
            second.get(&project.id).unwrap().dubbing.voice.as_deref(),
            Some("B voice")
        );
        for (id, text) in [(&a, "Hello A"), (&b, "Hello B")] {
            let reloaded = ProjectStore::new(_dir.path().join("projects"))
                .for_video(&project.id, Some(id))
                .unwrap();
            assert_eq!(
                transcript_service::get_transcript(&reloaded, &project.id)
                    .unwrap()
                    .unwrap()
                    .segments[0]
                    .source,
                text
            );
        }
        assert_ne!(
            first.content_path("audio/tts/raw.wav"),
            second.content_path("audio/tts/raw.wav")
        );
        assert_ne!(
            first.data_directory(&project.id, "cache/ocr").unwrap(),
            second.data_directory(&project.id, "cache/ocr").unwrap()
        );
        assert!(second
            .validate_audio_owner(&project.id, &first.content_path("audio/tts/raw.wav"))
            .is_err());
        assert!(first
            .validate_audio_owner(&project.id, "audio/tts/legacy.wav")
            .is_ok());
    }

    #[test]
    fn background_result_survives_reorder_trim_and_other_video_edits() {
        let (_dir, store, project, a, b) = fixture();
        let second = store.for_video(&project.id, Some(&b)).unwrap();
        let mut pending = second.get(&project.id).unwrap();
        let target = second.job_target(&pending);
        let first = store.for_video(&project.id, Some(&a)).unwrap();
        let mut edited = first.get(&project.id).unwrap();
        edited.translation.custom_prompt = "A only".into();
        first.update_project(&edited).unwrap();
        store
            .edit_project(&project.id, |project| {
                let clips = &mut project.timeline.as_mut().unwrap().clips;
                clips[0].start_us = 10_000_000;
                clips[1].start_us = 0;
                clips[1].source_in_us = 1_000_000;
                clips[1].source_out_us = 8_000_000;
                Ok(())
            })
            .unwrap();
        pending.translation.custom_prompt = "B only".into();
        second.update_project(&pending).unwrap();
        let root = store.get(&project.id).unwrap();
        assert_eq!(root.translation.custom_prompt, "A only");
        assert_eq!(root.timeline.as_ref().unwrap().clips[1].start_us, 0);
        assert_eq!(
            root.timeline.as_ref().unwrap().clips[1].source_in_us,
            1_000_000
        );
        // A content edit or placement cannot advance B's content revision.
        let fresh = second.get(&project.id).unwrap();
        assert_eq!(
            second.job_target(&fresh).content_revision,
            target.content_revision.map(|r| r + 1)
        );
    }

    #[test]
    fn recognition_discards_stale_revision_and_foreign_identity_even_with_same_checksum() {
        let (_dir, store, project, a, b) = fixture();
        let second = store.for_video(&project.id, Some(&b)).unwrap();
        let view = second.get(&project.id).unwrap();
        let target = second.job_target(&view);
        let original = transcript(&second, &view, "B draft");
        transcript_service::save_transcript(&second, &project.id, original.clone()).unwrap();
        assert!(transcript_service::save_generated_transcript(
            &second,
            &project.id,
            original.clone(),
            &target
        )
        .is_err());
        let mut stale = view;
        stale.translation.custom_prompt = "old snapshot".into();
        assert!(second.update_project(&stale).is_err());
        let mut wrong = original.clone();
        wrong.video_content_id = Some(a);
        assert!(transcript_service::save_transcript(&second, &project.id, wrong).is_err());
        let mut missing_identity = original;
        missing_identity.video_content_id = None;
        assert!(
            transcript_service::save_transcript(&second, &project.id, missing_identity).is_err()
        );
        assert_eq!(
            transcript_service::get_transcript(&second, &project.id)
                .unwrap()
                .unwrap()
                .segments[0]
                .source,
            "B draft"
        );
        assert!(store.for_video(&project.id, Some("../foreign")).is_err());
    }

    #[test]
    fn editing_one_translation_only_invalidates_its_audio() {
        let (_dir, store, project, a, b) = fixture();
        let first = store.for_video(&project.id, Some(&a)).unwrap();
        let second = store.for_video(&project.id, Some(&b)).unwrap();
        for scoped in [&first, &second] {
            let view = scoped.get(&project.id).unwrap();
            let mut value = transcript(scoped, &view, "source");
            value.segments.push(TranscriptSegment::new(
                2.0,
                3.0,
                "next".into(),
                TranscriptSourceType::Asr,
            ));
            for segment in &mut value.segments {
                segment.translated = "old".into();
                segment.tts = Some(TranscriptTts {
                    audio_path: Some(scoped.content_path("audio/tts/test.wav")),
                    duration: Some(1.0),
                    fingerprint: None,
                    status: Some(TranscriptTtsStatus::Ready),
                    voice: None,
                    speed: None,
                    text_hash: None,
                    generated_at: None,
                    warning: None,
                    provider: None,
                    model_id: None,
                    model_revision: None,
                });
            }
            transcript_service::save_transcript(scoped, &project.id, value).unwrap();
        }
        let mut edited = transcript_service::get_transcript(&first, &project.id)
            .unwrap()
            .unwrap();
        edited.segments[0].translated = "new".into();
        edited.segments[0].translation_status =
            crate::transcript_service::TranscriptTranslationStatus::Edited;
        transcript_service::save_transcript(&first, &project.id, edited).unwrap();
        let saved = transcript_service::get_transcript(&first, &project.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.segments[0].tts.as_ref().unwrap().status,
            Some(TranscriptTtsStatus::Outdated)
        );
        assert_eq!(
            saved.segments[1].tts.as_ref().unwrap().status,
            Some(TranscriptTtsStatus::Ready)
        );
        assert_eq!(
            transcript_service::get_transcript(&second, &project.id)
                .unwrap()
                .unwrap()
                .segments[0]
                .tts
                .as_ref()
                .unwrap()
                .status,
            Some(TranscriptTtsStatus::Ready)
        );
    }

    #[test]
    fn legacy_sidecar_remains_readable_and_first_scoped_save_keeps_original_bytes() {
        let (_dir, store, project, a, _) = fixture();
        let legacy = transcript(&store, &project, "historical A");
        transcript_service::save_transcript(&store, &project.id, legacy).unwrap();
        let old_path = store
            .project_directory(&project.id)
            .unwrap()
            .join("subtitles/transcript.json");
        let bytes = std::fs::read(&old_path).unwrap();
        let first = store.for_video(&project.id, Some(&a)).unwrap();
        let loaded = transcript_service::get_transcript(&first, &project.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.video_content_id.as_deref(), Some(a.as_str()));
        transcript_service::save_transcript(&first, &project.id, loaded).unwrap();
        assert_eq!(std::fs::read(old_path).unwrap(), bytes);
        assert_eq!(
            transcript_service::get_transcript(&store, &project.id)
                .unwrap()
                .unwrap()
                .segments[0]
                .source,
            "historical A"
        );
    }
}
