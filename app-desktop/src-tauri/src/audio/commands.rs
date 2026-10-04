use super::{
    mixer::Mix,
    transport::{AudioManager, Status},
};
use tauri::Manager;

#[tauri::command]
pub async fn audio_preview_open(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_id: String,
    dubbing: Option<crate::project_service::DubbingSettings>,
) -> Result<Status, String> {
    if window.label() != "main" {
        return Err("Cửa sổ không hỗ trợ audio preview.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let store = crate::commands::store(&app)?;
        let mut p = store.get(&project_id)?;
        if let Some(d) = dubbing {
            validate(&d)?;
            p.dubbing = d;
        }
        let duration = p
            .source
            .duration
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("Video chưa có thời lượng.")?;
        let source = crate::video_service::resolve_video(&store, &project_id)?;
        let transcript = crate::transcript_service::get_transcript(&store, &project_id)?;
        let voices = transcript
            .as_ref()
            .map(|t| super::timeline::voices(&store, &project_id, t, false, duration))
            .transpose()?
            .unwrap_or_default();
        let stem = if p.dubbing.dialogue_reduction_enabled && !p.source.audio_streams.is_empty() {
            Some(crate::dialogue_service::prepare_preview_stem(
                &app,
                &store,
                &project_id,
            )?)
        } else {
            None
        };
        let audio = app.state::<AudioManager>();
        if store.get(&project_id)?.source.checksum_sha256 != p.source.checksum_sha256 {
            return Err("Video đã thay đổi khi chuẩn bị âm thanh.".into());
        }
        audio.open(
            project_id.clone(),
            if p.source.audio_streams.is_empty() {
                None
            } else {
                Some(source)
            },
            stem,
            voices,
            Mix::project(&p.dubbing),
            duration,
        )
    })
    .await
    .map_err(|_| "Không mở được audio preview.")?
}
#[tauri::command]
pub fn audio_preview_status(app: tauri::AppHandle, project_id: String) -> Result<Status, String> {
    app.state::<AudioManager>().status(&project_id)
}
#[tauri::command]
pub async fn audio_preview_close(
    app: tauri::AppHandle,
    project_id: String,
    session_id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AudioManager>()
            .close_session(Some(&project_id), Some(&session_id))
    })
    .await
    .map_err(|_| "Không đóng được audio preview.".to_string())
}
#[tauri::command]
pub fn audio_preview_control(
    app: tauri::AppHandle,
    project_id: String,
    session_id: String,
    action: String,
    time: Option<f64>,
    volume: Option<f32>,
    dubbing: Option<crate::project_service::DubbingSettings>,
) -> Result<(), String> {
    if let Some(d) = &dubbing {
        validate(d)?;
    }
    app.state::<AudioManager>().control_session(
        &project_id,
        &session_id,
        &action,
        time,
        volume,
        dubbing.as_ref().map(Mix::project),
    )
}
fn validate(d: &crate::project_service::DubbingSettings) -> Result<(), String> {
    d.audio_processing.validate()?;
    for v in [
        d.original_volume,
        d.dubbed_volume,
        d.ducking_amount,
        d.dialogue_reduction_amount,
    ] {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err("Âm lượng không hợp lệ.".into());
        }
    }
    Ok(())
}
