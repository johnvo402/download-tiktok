//! Compare-and-swap editor layout; source content and track controls stay authoritative.
use crate::project_service::{Project, ProjectStore};
use crate::timeline::{self, Clip, LegacyBinding, TimelineDocument, Track, TrackKind};
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
    pub clips: Vec<Clip>,
    pub tracks: Vec<TrackLayout>,
    pub legacy_binding: Option<LegacyBinding>,
}
impl EditLayout {
    pub fn from_document(doc: &TimelineDocument) -> Self {
        Self {
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
        doc.clips = next.clips.clone();
        doc.tracks = tracks;
        doc.legacy_binding = next.legacy_binding.clone();
        if previous_binding.is_none() && project.source.video_path.is_none() {
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
    fn first_insertion_undo_redo_keeps_inventory_and_independent_source_content() {
        let (_dir, store, mut project, _, _) = crate::video_content::tests::fixture();
        let inserted = EditLayout::from_document(project.timeline.as_ref().unwrap());
        project.timeline.as_mut().unwrap().clips.clear();
        project.timeline.as_mut().unwrap().legacy_binding = None;
        project.source = Default::default();
        let empty_project = store.update_project(&project).unwrap();
        let empty = EditLayout::from_document(empty_project.timeline.as_ref().unwrap());
        let added = apply_layout(&store, &project.id, empty.clone(), inserted.clone()).unwrap();
        assert!(added.source.video_path.is_some());
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
}
