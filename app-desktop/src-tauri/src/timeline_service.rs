//! Small track transactions operate on the latest document, never a stale editor snapshot.
use crate::project_service::{Project, ProjectStore};
use crate::timeline::TrackKind;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackControl {
    Locked,
    Enabled,
    Muted,
    Solo,
}

pub fn change_track(
    store: &ProjectStore,
    project_id: &str,
    track_id: &str,
    control: TrackControl,
    value: bool,
) -> Result<Project, String> {
    store.edit_project(project_id, |project| {
        let doc = project.timeline.as_mut().ok_or("Dự án chưa có timeline.")?;
        let track = doc
            .tracks
            .iter_mut()
            .find(|track| track.id == track_id)
            .ok_or("Track không còn tồn tại.")?;
        match control {
            TrackControl::Locked => track.locked = value,
            TrackControl::Enabled => track.enabled = value,
            TrackControl::Muted | TrackControl::Solo if track.kind == TrackKind::Subtitle => {
                return Err("Track phụ đề không có âm thanh.".into())
            }
            TrackControl::Muted => track.muted = value,
            TrackControl::Solo => track.solo = value,
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn set_timeline_track_control(
    app: tauri::AppHandle,
    project_id: String,
    track_id: String,
    control: TrackControl,
    value: bool,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        change_track(
            &crate::commands::store(&app)?,
            &project_id,
            &track_id,
            control,
            value,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn track_changes_preserve_latest_video_settings_and_content_revision() {
        let (_dir, store, project, _, b) = crate::video_content::tests::fixture();
        let scoped = store.for_video(&project.id, Some(&b)).unwrap();
        let mut view = scoped.get(&project.id).unwrap();
        view.translation.target_language = "ja".into();
        scoped.update_project(&view).unwrap();
        let before = store.get(&project.id).unwrap();
        let track = &before.timeline.as_ref().unwrap().tracks[0].id;
        for control in [
            TrackControl::Locked,
            TrackControl::Enabled,
            TrackControl::Muted,
            TrackControl::Solo,
        ] {
            change_track(&store, &project.id, track, control, true).unwrap();
        }
        let reloaded = store.get(&project.id).unwrap();
        let doc = reloaded.timeline.as_ref().unwrap();
        assert_eq!(
            doc.video_contents,
            before.timeline.as_ref().unwrap().video_contents
        );
        assert_eq!(doc.clips, before.timeline.as_ref().unwrap().clips);
        assert!(
            doc.tracks[0].locked
                && doc.tracks[0].enabled
                && doc.tracks[0].muted
                && doc.tracks[0].solo
        );
        assert!(change_track(&store, &project.id, "missing", TrackControl::Locked, true).is_err());
        change_track(&store, &project.id, track, TrackControl::Locked, false).unwrap();
        assert!(!store.get(&project.id).unwrap().timeline.unwrap().tracks[0].locked);
    }
    #[test]
    fn rejected_subtitle_audio_control_does_not_write() {
        let (_dir, store, mut project, _, _) = crate::video_content::tests::fixture();
        let track_id = uuid::Uuid::new_v4().to_string();
        let mut track = project.timeline.as_ref().unwrap().tracks[0].clone();
        track.id = track_id.clone();
        track.kind = TrackKind::Subtitle;
        track.order = 1;
        project.timeline.as_mut().unwrap().tracks.push(track);
        let saved = store.update_project(&project).unwrap();
        assert!(change_track(&store, &project.id, &track_id, TrackControl::Solo, true).is_err());
        assert_eq!(store.get(&project.id).unwrap().updated_at, saved.updated_at);
    }
}
