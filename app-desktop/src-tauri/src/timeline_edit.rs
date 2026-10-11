//! Compare-and-swap editor layout; source content and track controls stay authoritative.
use crate::project_service::{Project, ProjectStore};
use crate::timeline::{
    self, Clip, LegacyBinding, TimelineDocument, TimelineSettings, Track, TrackKind,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackLayout {
    pub id: String,
    pub kind: TrackKind,
    pub name: String,
    pub order: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditLayout {
    pub settings: TimelineSettings,
    pub clips: Vec<Clip>,
    pub tracks: Vec<TrackLayout>,
    pub legacy_binding: Option<LegacyBinding>,
}
impl EditLayout {
    pub fn from_document(doc: &TimelineDocument) -> Self {
        Self {
            settings: doc.settings.clone(),
            clips: doc.clips.clone(),
            tracks: doc
                .tracks
                .iter()
                .map(|t| TrackLayout {
                    id: t.id.clone(),
                    kind: t.kind.clone(),
                    name: t.name.clone(),
                    order: t.order,
                })
                .collect(),
            legacy_binding: doc.legacy_binding.clone(),
        }
    }
}
pub fn apply_layout(
    store: &ProjectStore,
    project_id: &str,
    expected: EditLayout,
    next: EditLayout,
) -> Result<Project, String> {
    if next.clips.len() > 100_000 || next.tracks.len() > 10_000 {
        return Err("Timeline có quá nhiều clip/track.".into());
    }
    store.edit_project(project_id, |project| {
        let doc = project.timeline.as_mut().ok_or("Thiếu timeline.")?;
        if EditLayout::from_document(doc) != expected {
            return Err("Timeline đã thay đổi. Tải lại trước khi thực hiện thao tác dựng.".into());
        }
        for track in &doc.tracks {
            let layout = next.tracks.iter().find(|t| t.id == track.id);
            if layout.is_some_and(|t| t.kind != track.kind) {
                return Err("Không thể đổi loại track đang có.".into());
            }
            if track.locked {
                let old = expected.tracks.iter().find(|t| t.id == track.id);
                if layout != old
                    || doc
                        .clips
                        .iter()
                        .filter(|c| c.track_id == track.id)
                        .collect::<Vec<_>>()
                        != next
                            .clips
                            .iter()
                            .filter(|c| c.track_id == track.id)
                            .collect::<Vec<_>>()
                {
                    return Err("Track đang khóa. Mở khóa trước khi sửa hoặc undo/redo.".into());
                }
            }
        }
        let tracks = next
            .tracks
            .iter()
            .map(|layout| {
                let mut track = doc
                    .tracks
                    .iter()
                    .find(|t| t.id == layout.id)
                    .cloned()
                    .unwrap_or(Track {
                        id: layout.id.clone(),
                        kind: layout.kind.clone(),
                        name: layout.name.clone(),
                        order: layout.order,
                        locked: false,
                        enabled: true,
                        muted: false,
                        solo: false,
                    });
                track.name = layout.name.clone();
                track.order = layout.order;
                track
            })
            .collect();
        let previous_binding = doc.legacy_binding.clone();
        doc.settings = next.settings.clone();
        doc.clips = next.clips.clone();
        doc.tracks = tracks;
        doc.legacy_binding = next.legacy_binding.clone();
        if previous_binding.is_none()
            && project.source.video_path.is_none()
            && expected.settings == TimelineSettings::default()
        {
            if let Some(binding) = &doc.legacy_binding {
                if let Some(asset) = doc.assets.iter().find(|a| a.id == binding.asset_id) {
                    if let Some(width) = asset.metadata.width {
                        doc.settings.width = width;
                    }
                    if let Some(height) = asset.metadata.height {
                        doc.settings.height = height;
                    }
                    if let Some(rate) = asset.frame_rate {
                        doc.settings.frame_rate = rate;
                    }
                }
            }
        }
        timeline::validate(doc)?;
        // Undoing the first insertion can unbind the compatibility source; owned content remains.
        if previous_binding.is_some() && doc.legacy_binding.is_none() {
            project.source = Default::default();
        }
        Ok(())
    })
}
#[tauri::command]
pub async fn apply_timeline_edit(
    app: tauri::AppHandle,
    project_id: String,
    expected: EditLayout,
    next: EditLayout,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        apply_layout(&crate::commands::store(&app)?, &project_id, expected, next)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_and_instance_transforms_roundtrip_without_rewriting_source_regions_or_ai() {
        let (_dir, store, project, _, b) = crate::video_content::tests::fixture();
        let before = EditLayout::from_document(project.timeline.as_ref().unwrap());
        let mut next = before.clone();
        next.settings.width = 1080;
        next.settings.height = 1920;
        next.clips[0].transform.x = 0.8;
        next.clips[0].transform.scale_x = 0.25;
        next.clips[0].transform.scale_y = 0.25;
        next.clips[0].transform.crop = timeline::Crop {
            x: 0.2,
            y: 0.1,
            width: 0.6,
            height: 0.8,
        };
        let saved = apply_layout(&store, &project.id, before.clone(), next.clone()).unwrap();
        assert_eq!(
            store.get(&project.id).unwrap().timeline.unwrap().settings,
            next.settings
        );
        let original_content = project.timeline.as_ref().unwrap().video_contents.clone();
        assert_eq!(
            saved.timeline.as_ref().unwrap().video_contents,
            original_content
        );
        assert_eq!(saved.source, project.source);
        assert_eq!(next.clips[0].source_in_us, before.clips[0].source_in_us);
        let scoped = store.for_video(&project.id, Some(&b)).unwrap();
        let mut view = scoped.get(&project.id).unwrap();
        view.translation.target_language = "ja".into();
        scoped.update_project(&view).unwrap();
        let restored = apply_layout(&store, &project.id, next.clone(), before.clone()).unwrap();
        assert_eq!(
            restored.timeline.as_ref().unwrap().settings,
            before.settings
        );
        assert_eq!(
            restored.timeline.as_ref().unwrap().video_contents[1]
                .settings
                .translation
                .target_language,
            "ja"
        );
        assert_eq!(
            apply_layout(&store, &project.id, before, next.clone())
                .unwrap()
                .timeline
                .unwrap()
                .clips,
            next.clips
        );
    }
    #[test]
    fn typography_is_optional_for_old_clips_and_invalid_visual_edits_are_atomic() {
        let (_dir, store, project, _, _) = crate::video_content::tests::fixture();
        let before = EditLayout::from_document(project.timeline.as_ref().unwrap());
        let old = serde_json::to_value(&before.clips[0]).unwrap();
        assert!(old.get("style").is_none());
        assert!(serde_json::from_value::<Clip>(old).unwrap().style.is_none());
        let mut text = before.clips[0].clone();
        text.id = uuid::Uuid::new_v4().to_string();
        text.start_us = 20_000_000;
        text.content = timeline::ClipContent::Text {
            text: "Tiêu đề".into(),
        };
        text.style = Some(timeline::ClipStyle {
            font_family: "Arial".into(),
            font_size: 48.0,
            font_weight: 700,
            color: "#FFFFFF".into(),
            background_color: "#000000".into(),
            background_opacity: 0.5,
            alignment: "center".into(),
        });
        let mut next = before.clone();
        next.clips.push(text);
        let saved = apply_layout(&store, &project.id, before, next.clone()).unwrap();
        assert_eq!(
            store
                .get(&project.id)
                .unwrap()
                .timeline
                .unwrap()
                .clips
                .last()
                .unwrap()
                .style,
            next.clips.last().unwrap().style
        );
        let mut invalid = next.clone();
        invalid
            .clips
            .last_mut()
            .unwrap()
            .style
            .as_mut()
            .unwrap()
            .font_size = 0.0;
        assert!(apply_layout(&store, &project.id, next.clone(), invalid).is_err());
        let mut invalid = next.clone();
        invalid.settings.width = 20000;
        assert!(apply_layout(&store, &project.id, next.clone(), invalid).is_err());
        let mut invalid = next.clone();
        invalid.clips[0].transform.crop.width = 1.1;
        assert!(apply_layout(&store, &project.id, next, invalid).is_err());
        assert_eq!(store.get(&project.id).unwrap().updated_at, saved.updated_at);
    }
    #[test]
    fn first_insertion_undo_redo_keeps_inventory_and_independent_source_content() {
        let (_dir, store, mut project, _, _) = crate::video_content::tests::fixture();
        let mut inserted = EditLayout::from_document(project.timeline.as_ref().unwrap());
        project.timeline.as_mut().unwrap().clips.clear();
        project.timeline.as_mut().unwrap().legacy_binding = None;
        project.source = Default::default();
        project.timeline.as_mut().unwrap().settings.width = 1080;
        project.timeline.as_mut().unwrap().settings.height = 1920;
        inserted.settings = project.timeline.as_ref().unwrap().settings.clone();
        let empty_project = store.update_project(&project).unwrap();
        let empty = EditLayout::from_document(empty_project.timeline.as_ref().unwrap());
        let added = apply_layout(&store, &project.id, empty.clone(), inserted.clone()).unwrap();
        assert!(added.source.video_path.is_some());
        assert_eq!(added.timeline.as_ref().unwrap().settings.width, 1080);
        assert_eq!(added.timeline.as_ref().unwrap().settings.height, 1920);
        let content = added.timeline.as_ref().unwrap().video_contents.clone();
        let undone = apply_layout(&store, &project.id, inserted.clone(), empty.clone()).unwrap();
        assert!(undone.source.video_path.is_none());
        assert_eq!(undone.timeline.as_ref().unwrap().assets.len(), 2);
        assert_eq!(undone.timeline.as_ref().unwrap().video_contents, content);
        let redone = apply_layout(&store, &project.id, empty, inserted).unwrap();
        assert_eq!(redone.timeline.as_ref().unwrap().video_contents, content);
        assert_eq!(redone.timeline.as_ref().unwrap().clips.len(), 2);
    }
    #[test]
    fn delete_last_bound_clip_and_undo_do_not_recreate_it_or_revert_ai_content() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let before = EditLayout::from_document(project.timeline.as_ref().unwrap());
        let mut deleted = before.clone();
        deleted.clips.retain(|c| c.id != before.clips[0].id);
        deleted.legacy_binding.as_mut().unwrap().clip_id = None;
        let changed = apply_layout(&store, &project.id, before.clone(), deleted.clone()).unwrap();
        assert_eq!(changed.timeline.as_ref().unwrap().clips.len(), 1);
        let scoped = store.for_video(&project.id, Some(&b)).unwrap();
        let mut view = scoped.get(&project.id).unwrap();
        view.translation.target_language = "ja".into();
        scoped.update_project(&view).unwrap();
        let contents = store
            .get(&project.id)
            .unwrap()
            .timeline
            .unwrap()
            .video_contents;
        let restored = apply_layout(&store, &project.id, deleted, before).unwrap();
        assert_eq!(restored.timeline.as_ref().unwrap().video_contents, contents);
        assert_eq!(restored.timeline.as_ref().unwrap().clips.len(), 2);
        assert!(store.for_video(&project.id, Some(&a)).is_ok());
    }
    #[test]
    fn stale_overlap_bounds_and_locked_edits_are_atomic_and_controls_survive_undo() {
        let (_dir, store, project, _, _) = crate::video_content::tests::fixture();
        let before = EditLayout::from_document(project.timeline.as_ref().unwrap());
        let mut after = before.clone();
        after.clips[1].start_us = 20_000_000;
        let saved = apply_layout(&store, &project.id, before.clone(), after.clone()).unwrap();
        assert!(apply_layout(&store, &project.id, before.clone(), after.clone()).is_err());
        let mut overlap = after.clone();
        overlap.clips[1].start_us = 1;
        assert!(apply_layout(&store, &project.id, after.clone(), overlap).is_err());
        let mut overflow = after.clone();
        overflow.clips[0].source_out_us += 1;
        assert!(apply_layout(&store, &project.id, after.clone(), overflow).is_err());
        assert_eq!(store.get(&project.id).unwrap().updated_at, saved.updated_at);
        let track = &before.tracks[0].id;
        crate::timeline_service::change_track(
            &store,
            &project.id,
            track,
            crate::timeline_service::TrackControl::Locked,
            true,
        )
        .unwrap();
        assert!(apply_layout(&store, &project.id, after.clone(), before.clone()).is_err());
        crate::timeline_service::change_track(
            &store,
            &project.id,
            track,
            crate::timeline_service::TrackControl::Locked,
            false,
        )
        .unwrap();
        crate::timeline_service::change_track(
            &store,
            &project.id,
            track,
            crate::timeline_service::TrackControl::Muted,
            true,
        )
        .unwrap();
        assert!(
            apply_layout(&store, &project.id, after, before)
                .unwrap()
                .timeline
                .unwrap()
                .tracks[0]
                .muted
        );
    }
    #[test]
    fn audio_defaults_roundtrip_detached_source_and_invalid_mix_edits_are_atomic() {
        let (_dir, store, project, a, b) = crate::video_content::tests::fixture();
        let before = EditLayout::from_document(project.timeline.as_ref().unwrap());
        let old = serde_json::to_value(&before.clips[0]).unwrap();
        assert!(old.get("audio").is_none());
        assert!(serde_json::from_value::<Clip>(old).unwrap().audio.is_none());
        let mut next = before.clone();
        let audio = timeline::ClipAudio {
            volume: 1.5,
            muted: false,
            fade_in_us: 1_000_000,
            fade_out_us: 2_000_000,
            voice_enabled: false,
            voice_speed: 1.25,
            ducking_enabled: true,
            ducking_amount: 0.75,
            duck_attack_us: 60_000,
            duck_release_us: 180_000,
        };
        next.clips[0].audio = Some(audio.clone());
        let mut extracted = next.clips[0].clone();
        extracted.id = uuid::Uuid::new_v4().to_string();
        let asset_id = project.timeline.as_ref().unwrap().video_contents[0]
            .asset_id
            .clone();
        extracted.content = timeline::ClipContent::SourceAudio {
            asset_id,
            video_content_id: a,
        };
        extracted.linked_clip_id = Some(next.clips[0].id.clone());
        let mut track = next.tracks[0].clone();
        track.id = uuid::Uuid::new_v4().to_string();
        track.kind = TrackKind::Audio;
        track.order = 1;
        extracted.track_id = track.id.clone();
        next.tracks.push(track);
        next.clips.push(extracted);
        let saved = apply_layout(&store, &project.id, before.clone(), next.clone()).unwrap();
        assert_eq!(saved.timeline.as_ref().unwrap().clips[0].audio, Some(audio));
        let mut invalid = next.clone();
        invalid.clips[0].audio.as_mut().unwrap().volume = 2.01;
        assert!(apply_layout(&store, &project.id, next.clone(), invalid).is_err());
        let mut invalid = next.clone();
        invalid.clips.last_mut().unwrap().content = timeline::ClipContent::SourceAudio {
            asset_id: project.timeline.as_ref().unwrap().video_contents[0]
                .asset_id
                .clone(),
            video_content_id: b,
        };
        assert!(apply_layout(&store, &project.id, next.clone(), invalid).is_err());
        assert_eq!(store.get(&project.id).unwrap().updated_at, saved.updated_at);
        let undone = apply_layout(&store, &project.id, next, before).unwrap();
        assert!(undone.timeline.unwrap().clips[0].audio.is_none());
    }
}
