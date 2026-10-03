mod asr_service;
mod batch_service;
mod commands;
mod diagnostics_service;
pub mod hardware_acceleration;
mod dialogue_service;
mod job_service;
mod log_service;
mod overlays_service;
mod optional_runtime;
mod project_service;
mod render_service;
pub mod render;
pub mod render_spec;
mod resource_scheduler;
mod runtime_health;
mod runtime_paths;
mod settings_service;
mod storage_service;
mod transcript_service;
mod text_removal_service;
mod translation_service;
mod tts_service;
mod video_service;
mod video_tools_service;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(asr_service::AsrJobs::default())
        .manage(translation_service::TranslationJobs::default())
        .manage(tts_service::TtsJobs::default())
        .manage(render_service::RenderJobs::default())
        .manage(video_tools_service::VideoToolsJobs::default())
        .manage(job_service::JobStore::default())
        .setup(|app| {
            let resource_dir = app.path().resource_dir()?;
            let app_data = app.path().app_data_dir()?;
            runtime_paths::initialize(resource_dir, app_data.clone());
            std::fs::create_dir_all(app_data.join("projects"))?;
            if let Err(error) = settings_service::migrate_legacy_custom_prompt(app.handle()) {
                log::warn!("Could not migrate legacy project translation prompt: {error}");
            }
            let logs = log_service::logs_directory(&app_data);
            if let Err(error) = log_service::initialize(&logs) {
                eprintln!("Application file logger unavailable: {error}");
            }
            match text_removal_service::cleanup_legacy_lama_assets(app.handle()) {
                Ok(bytes) if bytes > 0 => log::info!("Removed retired LaMa model files ({bytes} bytes)"),
                Ok(_) => {}
                Err(error) => log::warn!("Could not remove retired LaMa model files: {error}"),
            }
            match storage_service::migrate_local_storage(app.handle()) {
                Ok(warnings) => for warning in warnings { log::warn!("Storage migration: {warning}"); },
                Err(error) => log::warn!("Storage migration could not finish: {error}"),
            }
            log::info!("Application startup; version={}", env!("CARGO_PKG_VERSION"));
            let path = app_data.join("jobs.json");
            app.state::<job_service::JobStore>()
                .initialize(path)
                .map_err(std::io::Error::other)?;
            std::panic::set_hook(Box::new(|info| {
                let location = info
                    .location()
                    .map(|location| format!("{}:{}", location.file(), location.line()))
                    .unwrap_or_else(|| "unknown location".to_string());
                log::error!("Application panic at {location}; panic payload omitted from logs");
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_optional_runtime_status,
            commands::open_ai_components_folder,
            commands::install_optional_runtime,
            commands::clear_ai_installation_cache,
            commands::get_runtime_health,
            commands::export_diagnostics,
            commands::list_projects,
            commands::create_project,
            commands::get_project,
            commands::rename_project,
            commands::delete_project,
            commands::import_video,
            commands::get_video_asset_path,
            commands::create_preview_proxy,
            commands::get_video_metadata,
            commands::get_subtitle_tracks,
            commands::extract_subtitle,
            commands::get_transcript,
            commands::save_transcript,
            commands::get_asr_environment,
            commands::get_ocr_environment,
            commands::get_text_removal_environment,
            commands::install_sttn_text_removal_model,
            commands::remove_sttn_text_removal_model,
            commands::start_text_removal_preview,
            commands::install_ocr_runtime,
            commands::install_ocr_model,
            commands::remove_ocr_model,
            commands::install_asr_model,
            commands::install_whisperx_alignment,
            commands::remove_asr_model,
            commands::get_batch_video_queue,
            commands::save_batch_video_queue,
            commands::clear_batch_video_queue,
            commands::generate_transcript,
            commands::cancel_transcription,
            commands::get_app_settings,
            commands::save_app_settings,
            commands::set_translation_credential,
            commands::delete_translation_credential,
            commands::test_translation_provider,
            commands::start_translation,
            commands::cancel_translation,
            commands::update_translation_languages,
            commands::update_subtitle_settings,
            commands::save_visual_settings,
            commands::import_logo,
            commands::replace_logo,
            commands::remove_logo,
            commands::get_logo_asset_path,
            commands::get_tts_environment,
            commands::get_dialogue_environment,
            commands::prepare_dialogue_preview_stem,
            commands::install_dialogue_model,
            commands::install_dialogue_model_from_file,
            commands::remove_dialogue_model,
            commands::install_tts_model,
            commands::remove_tts_model,
            commands::preview_tts_voice,
            commands::add_tts_voice_profile,
            commands::delete_tts_voice_profile,
            commands::start_tts_generation,
            commands::cancel_tts_generation,
            commands::get_tts_audio_path,
            commands::update_dubbing_settings,
            commands::update_project_preferences,
            commands::update_transcript_settings,
            commands::validate_export,
            commands::get_export_capabilities,
            commands::start_export,
            commands::cancel_export,
            commands::get_export_history,
            commands::get_export_directory,
            commands::open_export_folder,
            commands::inspect_tiktok_cookie_file,
            commands::has_tiktok_browser_session,
            commands::start_tiktok_browser_login,
            commands::get_video_tool_output_directory,
            commands::scan_merge_videos,
            commands::start_tiktok_drama_download,
            commands::start_video_merge,
            commands::list_jobs,
            commands::cancel_unified_job,
            commands::retry_unified_job,
            commands::get_storage_usage,
            commands::clear_temporary_cache,
            commands::open_cache_folder,
            commands::open_model_folder
        ])
        .run(tauri::generate_context!())
        .expect("failed to run JC AI");
}
