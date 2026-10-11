mod file_integrity;
mod disk_space;
mod startup_recovery;
mod app_session;
mod app_error;
mod recovery_store;
mod worker_manager;
mod runtime_registry;
mod asr_service;
mod audio;
mod batch_service;
mod commands;
mod preview;
mod diagnostics_service;
pub mod hardware_acceleration;
mod dialogue_service;
mod job_service;
mod log_service;
mod overlays_service;
mod optional_runtime;
mod project_service;
pub mod timeline;
mod timeline_service;
mod timeline_edit;
mod timeline_audio;
mod timeline_export;
pub mod video_content;
mod render_service;
pub mod render;
pub mod media_gpu;
pub mod render_spec;
mod resource_scheduler;
mod runtime_health;
mod runtime_paths;
mod app_storage;
mod settings_service;
mod storage_service;
mod studio_service;
mod transcript_service;
mod text_removal_service;
mod text_removal_cache;
mod translation_service;
mod tts_service;
mod video_service;
mod media_library;
mod video_tools_service;

use tauri::Manager;

pub fn run() {
    let application=tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(preview::PreviewManager::default())
        .manage(audio::transport::AudioManager::default())
        .manage(asr_service::AsrJobs::default())
        .manage(translation_service::TranslationJobs::default())
        .manage(tts_service::TtsJobs::default())
        .manage(render_service::RenderJobs::default())
        .manage(video_tools_service::VideoToolsJobs::default())
        .manage(job_service::JobStore::default())
        .setup(|app| {
            let resource_dir = app.path().resource_dir()?;
            let app_data = app_storage::resolve(app.handle())?;
            app_storage::prepare(&app_data)?;
            let session = match app_session::AppSession::begin(&app_data) {
                Ok(session) => session,
                Err(error) if error == "JC AI đã mở." => { app.handle().exit(0); return Ok(()); }
                Err(error) => return Err(std::io::Error::other(error).into()),
            };
            app.manage(session);
            runtime_paths::initialize(resource_dir, app_data.clone());
            // Third-party tools and WebView subprocesses inherit this location.
            // Set it once at startup, before opening the application's window.
            for key in ["TEMP", "TMP", "TMPDIR"] {
                std::env::set_var(key, app_data.join("cache/tmp"));
            }
            let logs = log_service::logs_directory(&app_data);
            if let Err(error) = log_service::initialize(&logs) { eprintln!("Application file logger unavailable: {error}"); }
            if let Err(error)=optional_runtime::recover_installations() {
                log::warn!("Optional runtime recovery deferred: {error}");
            }
            std::fs::create_dir_all(app_data.join("projects"))?;
            if let Err(error) = settings_service::migrate_legacy_custom_prompt(app.handle()) {
                log::warn!("Could not migrate legacy project translation prompt: {error}");
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
            let recovered_jobs=app.state::<job_service::JobStore>().list().unwrap_or_default();
            let projects=project_service::ProjectStore::new(app_data.join("projects"));
            std::thread::spawn(move || startup_recovery::cleanup(&projects,&recovered_jobs));
            let crash_directory=app_data.join("crashes");
            std::panic::set_hook(Box::new(move |info| {
                let location = info
                    .location()
                    .map(|location| format!("{}:{}", location.file(), location.line()))
                    .unwrap_or_else(|| "unknown location".to_string());
                log::error!("Application panic at {location}; panic payload omitted from logs");
                // Local only: omit panic payload, transcripts, credentials and
                // user paths. Diagnostics bundles never copy raw crash files.
                if std::fs::create_dir_all(&crash_directory).is_ok() {
                    let path=crash_directory.join(format!("crash-{}.json",uuid::Uuid::new_v4()));
                    let report=serde_json::json!({"code":"APP_PANIC","sessionId":app_session::id(),"appVersion":env!("CARGO_PKG_VERSION"),"commit":option_env!("GIT_COMMIT"),"location":location,"date":chrono::Utc::now().to_rfc3339(),"backtrace":std::backtrace::Backtrace::force_capture().to_string()});
                    if let Ok(bytes)=serde_json::to_vec_pretty(&report) {
                        use std::io::Write;
                        if let Ok(mut file)=std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {let _=file.write_all(&bytes).and_then(|_|file.sync_all());}
                    }
                }
            }));
            // Build manually after storage selection; automatic Tauri windows
            // otherwise create a WebView2 profile in Local AppData first.
            for config in &app.config().app.windows {
                tauri::WebviewWindowBuilder::from_config(app.handle(), config)?
                    .data_directory(app_data.join("webview").join(&config.label))
                    .build()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            media_library::import_media_assets,
            media_library::inspect_media_assets,
            media_library::relink_media_asset,
            media_library::append_media_clip,
            media_library::get_media_cache,
            media_library::get_media_asset_path,
            timeline_service::set_timeline_track_control,
            timeline_edit::apply_timeline_edit,
            timeline_audio::get_timeline_source_media,
            studio_service::load_studio_draft,
            studio_service::has_studio_draft,
            studio_service::save_studio_draft,
            studio_service::studio_ai,
            studio_service::studio_import_image,
            studio_service::studio_asset,
            studio_service::studio_voice,
            studio_service::studio_export,
            studio_service::cancel_studio_job,
            commands::get_recovery_notices,
            audio::commands::audio_preview_open,
            audio::commands::audio_preview_close,
            audio::commands::audio_preview_status,
            audio::commands::audio_preview_control,
            preview::commands::preview_open,
            preview::commands::preview_audio_path,
            preview::commands::preview_close,
            preview::commands::preview_control,
            preview::commands::preview_resize,
            preview::commands::preview_update_scene,
            preview::commands::preview_status,
            commands::get_optional_runtime_status,
            commands::get_optional_runtime_plan,
            commands::check_optional_runtime_feature,
            commands::uninstall_optional_runtime,
            commands::open_ai_components_folder,
            commands::install_optional_runtime,
            commands::clear_ai_installation_cache,
            commands::get_runtime_health,
            commands::get_session_status,
            commands::report_frontend_error,
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
            commands::get_text_removal_cache_status,
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
        .build(tauri::generate_context!());
    let application=match application {
        Ok(application)=>application,
        Err(error)=>{
            log::error!("Application startup failed: {}",log_service::redact(&error.to_string()));
            #[cfg(windows)] unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW,MB_OK,MB_ICONERROR};
                let message:Vec<u16>="JC AI không mở được dữ liệu ứng dụng. Hãy kiểm tra quyền ghi và dung lượng ổ đĩa; dữ liệu hiện có được giữ nguyên.".encode_utf16().chain(Some(0)).collect();
                let title:Vec<u16>="JC AI".encode_utf16().chain(Some(0)).collect();
                MessageBoxW(None,windows::core::PCWSTR(message.as_ptr()),windows::core::PCWSTR(title.as_ptr()),MB_OK|MB_ICONERROR);
            }
            return;
        }
    };
    application.run(|app,event| { if matches!(event,tauri::RunEvent::Exit) { app.state::<preview::PreviewManager>().close(None,None); app.state::<audio::transport::AudioManager>().close(None); worker_manager::shutdown(); if let Some(session)=app.try_state::<app_session::AppSession>() { if let Err(error)=session.finish() { log::warn!("Session shutdown marker could not be saved: {error}"); } } } });
}
