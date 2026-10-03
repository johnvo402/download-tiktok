use crate::{
    overlays_service::VisualSettings,
    project_service::{BlurRegionRole, Project, ProjectStore, TranscriptSourceType},
    render::graph::RenderScene,
    render_spec::{build_render_specification, ExportOptions},
    transcript_service::Transcript,
};
use std::path::PathBuf;
pub fn apply_translations(
    transcript: &mut Transcript,
    patches: Vec<super::manager::TranslationPatch>,
    duration: f64,
) -> Result<(), String> {
    for p in patches {
        if p.deleted {
            transcript.segments.retain(|s| s.id != p.id);
            continue;
        }
        if let Some(seg) = transcript.segments.iter_mut().find(|seg| seg.id == p.id) {
            seg.start = p.start;
            seg.end = p.end;
            seg.translated = p.translated;
        } else {
            transcript
                .segments
                .push(crate::transcript_service::TranscriptSegment {
                    id: p.id,
                    start: p.start,
                    end: p.end,
                    source: String::new(),
                    translated: p.translated,
                    source_revision: 0,
                    translated_revision: None,
                    translation_meta: None,
                    speaker_id: None,
                    source_type: transcript.source_type.clone(),
                    translation_status:
                        crate::transcript_service::TranscriptTranslationStatus::Pending,
                    tts: None,
                });
        }
    }
    crate::transcript_service::validate_transcript(transcript, Some(duration))
}
pub fn empty_transcript(project: &Project) -> Transcript {
    Transcript {
        schema_version: 1,
        project_id: project.id.clone(),
        source_type: TranscriptSourceType::Asr,
        source_track: None,
        language: None,
        video_checksum: project.source.checksum_sha256.clone(),
        segments: vec![],
    }
}
pub fn build(project: &Project, transcript: &Transcript) -> Result<RenderScene, String> {
    // Editing an unfinished scene is valid. Export still applies its strict missing-region validation.
    let has_region = project
        .blur_regions
        .iter()
        .any(|r| r.role == BlurRegionRole::Subtitle && r.enabled)
        || (project.subtitle.region.is_some()
            && !project
                .blur_regions
                .iter()
                .any(|r| r.role == BlurRegionRole::Subtitle));
    let options = ExportOptions {
        burn_subtitles: project.subtitle.enabled
            && has_region
            && transcript
                .segments
                .iter()
                .any(|s| !s.translated.trim().is_empty()),
        ..Default::default()
    };
    Ok(RenderScene::from_spec(&build_render_specification(
        project,
        transcript,
        project.source.duration.ok_or("Thiếu thời lượng video.")?,
        &options,
    )?))
}
pub fn apply(
    store: &ProjectStore,
    project: &mut Project,
    visual: VisualSettings,
) -> Result<(), String> {
    crate::overlays_service::validate_subtitle_settings(&visual.subtitle)?;
    crate::overlays_service::validate_blur_regions(&visual.blur_regions)?;
    crate::overlays_service::validate_text_overlays(&visual.text_overlays)?;
    crate::overlays_service::validate_logos(store, &project.id, &visual.logos)?;
    project.subtitle = visual.subtitle;
    project.blur_regions = visual.blur_regions;
    project.logos = visual.logos;
    project.text_overlays = visual.text_overlays;
    Ok(())
}
pub fn logos(store: &ProjectStore, id: &str, scene: &RenderScene) -> Result<Vec<PathBuf>, String> {
    scene
        .logos
        .iter()
        .map(|l| {
            crate::video_service::resolve_project_file(store, id, &l.asset_path, "media/overlays")
        })
        .collect()
}
