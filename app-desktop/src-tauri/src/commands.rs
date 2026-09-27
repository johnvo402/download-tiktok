use crate::project_service::{
    Project, ProjectListResponse, ProjectSource, ProjectStore, SubtitleTrackMetadata,
};
use crate::render_spec;
use crate::{
    asr_service, batch_service, dialogue_service, optional_runtime, overlays_service, project_service::SubtitleSettings, render_service,
    settings_service, transcript_service, translation_service, tts_service, video_service,
    video_tools_service,
};
use std::sync::Mutex;
use tauri::Manager;
use transcript_service::Transcript;

static PROJECT_JOB_START_LOCK: Mutex<()> = Mutex::new(());

#[tauri::command]
pub fn get_optional_runtime_status() -> optional_runtime::RuntimeStatus {
    optional_runtime::status()
}

#[tauri::command]
pub fn open_ai_components_folder(app: tauri::AppHandle) -> Result<String, String> {
    crate::storage_service::open_ai_components_folder(&app)
}

#[tauri::command]
pub async fn install_optional_runtime(app: tauri::AppHandle, feature: String, profile: String) -> Result<optional_runtime::RuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(move || optional_runtime::install(&app, &feature, &profile))
        .await.map_err(|error| format!("Không thể cài bộ xử lý bổ sung: {error}"))?
}

fn store(app: &tauri::AppHandle) -> Result<ProjectStore, String> {
    let mut root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu ứng dụng: {error}"))?;
    root.push("projects");
    Ok(ProjectStore::new(root))
}

#[tauri::command]
pub fn get_batch_video_queue(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<Option<batch_service::BatchVideoQueue>, String> {
    batch_service::load(&store(&app)?, &project_id)
}

#[tauri::command]
pub fn save_batch_video_queue(
    app: tauri::AppHandle,
    project_id: String,
    queue: batch_service::BatchVideoQueue,
) -> Result<batch_service::BatchVideoQueue, String> {
    batch_service::save(&store(&app)?, &project_id, queue)
}

#[tauri::command]
pub fn clear_batch_video_queue(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    batch_service::clear(&store(&app)?, &project_id)
}

#[tauri::command]
pub fn list_projects(app: tauri::AppHandle) -> Result<ProjectListResponse, String> {
    store(&app)?.list()
}

#[tauri::command]
pub fn get_runtime_health(app: tauri::AppHandle) -> crate::runtime_health::RuntimeHealth {
    crate::runtime_health::check(&app)
}

#[tauri::command]
pub async fn export_diagnostics(
    app: tauri::AppHandle,
    destination: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::diagnostics_service::export(&app, &destination)
    })
    .await
    .map_err(|error| format!("Không thể tạo diagnostics: {error}"))?
}

#[tauri::command]
pub fn list_jobs(
    jobs: tauri::State<'_, crate::job_service::JobStore>,
) -> Result<Vec<crate::job_service::JobRecord>, String> {
    jobs.list()
}

#[tauri::command]
pub async fn get_storage_usage(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, crate::job_service::JobStore>,
) -> Result<crate::storage_service::StorageUsage, String> {
    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || crate::storage_service::usage(&app, &jobs))
        .await
        .map_err(|error| format!("Không thể đọc dung lượng cache: {error}"))?
}

#[tauri::command]
pub async fn clear_temporary_cache(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, crate::job_service::JobStore>,
) -> Result<u64, String> {
    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::storage_service::clear_temporary(&app, &jobs)
    })
    .await
    .map_err(|error| format!("Không thể dọn cache tạm: {error}"))?
}

#[tauri::command]
pub fn open_cache_folder(app: tauri::AppHandle) -> Result<String, String> {
    crate::storage_service::open_cache_folder(&app)
}

#[tauri::command]
pub fn open_model_folder(
    app: tauri::AppHandle,
    kind: crate::storage_service::ModelFolderKind,
) -> Result<String, String> {
    crate::storage_service::open_model_folder(&app, kind)
}

#[tauri::command]
pub fn cancel_unified_job(
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    video_tools_jobs: tauri::State<'_, video_tools_service::VideoToolsJobs>,
    job_id: String,
) -> Result<bool, String> {
    let job = job_store.get(&job_id)?;
    if !crate::job_service::is_active(&job.status) {
        return Ok(false);
    }
    match job.job_type.as_str() {
        "transcribe" => asr_service::cancel_transcription(asr_jobs, job_id),
        "translate" => translation_service::cancel_translation(translation_jobs, job_id),
        "tts" => tts_service::cancel(&tts_jobs, &job_id),
        "export" => render_service::cancel_export(&render_jobs, &job_id),
        "video_merge" | "tiktok_download" | "tiktok_cookie_login" => video_tools_service::cancel(&video_tools_jobs, &job_id),
        _ => Err("Loại job này chưa hỗ trợ hủy từ hàng đợi.".into()),
    }
}

#[tauri::command]
pub async fn retry_unified_job(
    app: tauri::AppHandle,
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    job_id: String,
) -> Result<String, String> {
    let previous = job_store.get(&job_id)?;
    if !matches!(
        previous.status.as_str(),
        "failed" | "partial" | "cancelled" | "interrupted"
    ) {
        return Err("Chỉ có thể thử lại job đã dừng hoặc thất bại.".into());
    }
    let project_id = previous
        .project_id
        .clone()
        .ok_or_else(|| "Job này không thuộc project nào và chưa thể thử lại.".to_string())?;
    let payload = previous
        .retry_payload
        .clone()
        .ok_or_else(|| "Job này không có dữ liệu cần thiết để thử lại.".to_string())?;
    let asr_jobs = asr_jobs.inner().clone();
    let translation_jobs = translation_jobs.inner().clone();
    let tts_jobs = tts_jobs.inner().clone();
    let render_jobs = render_jobs.inner().clone();
    let jobs = job_store.inner().clone();
    let root = projects_root(&app)?;
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        let id = match previous.job_type.as_str() {
            "transcribe" => {
                if translation_service::has_active_project_job(&translation_jobs, &project_id)
                    || render_service::has_active_project_job(&render_jobs, &project_id)
                    || tts_service::has_active_project_job(&tts_jobs, &project_id)
                {
                    return Err("Dự án đang có job ghi hoặc đọc dữ liệu. Hãy đợi job đó kết thúc rồi thử lại.".into());
                }
                let options = serde_json::from_value(payload)
                    .map_err(|error| format!("Cấu hình transcription cũ không hợp lệ: {error}"))?;
                asr_service::generate_transcript(app.clone(), &asr_jobs, project_id.clone(), options)?
            }
            "translate" => {
                if asr_service::has_active_project_job(&asr_jobs, &project_id)
                    || render_service::has_active_project_job(&render_jobs, &project_id)
                    || tts_service::has_active_project_job(&tts_jobs, &project_id)
                {
                    return Err("Dự án đang có job ghi hoặc đọc dữ liệu. Hãy đợi job đó kết thúc rồi thử lại.".into());
                }
                let request = serde_json::from_value(payload)
                    .map_err(|error| format!("Cấu hình translation cũ không hợp lệ: {error}"))?;
                translation_service::start_translation_job(app.clone(), &translation_jobs, project_id.clone(), request)?
            }
            "tts" => {
                if asr_service::has_active_project_job(&asr_jobs, &project_id)
                    || translation_service::has_active_project_job(&translation_jobs, &project_id)
                    || render_service::has_active_project_job(&render_jobs, &project_id)
                {
                    return Err("Dự án đang có job ghi hoặc đọc dữ liệu. Hãy đợi job đó kết thúc rồi thử lại.".into());
                }
                let request = serde_json::from_value(payload)
                    .map_err(|error| format!("Cấu hình TTS cũ không hợp lệ: {error}"))?;
                tts_service::start_generation(app.clone(), &tts_jobs, project_id.clone(), request)?
            }
            "export" => {
                if asr_service::has_active_project_job(&asr_jobs, &project_id)
                    || translation_service::has_active_project_job(&translation_jobs, &project_id)
                    || tts_service::has_active_project_job(&tts_jobs, &project_id)
                {
                    return Err("Dự án đang có job ghi dữ liệu. Hãy đợi job đó kết thúc rồi thử lại export.".into());
                }
                let options = serde_json::from_value(payload)
                    .map_err(|error| format!("Cấu hình export cũ không hợp lệ: {error}"))?;
                render_service::start_export(app.clone(), &render_jobs, ProjectStore::new(root), project_id.clone(), options)?
            }
            _ => return Err("Loại job này chưa hỗ trợ retry.".into()),
        };
        jobs.link_retry(&id, &job_id);
        Ok(id)
    })
    .await
    .map_err(|error| format!("Không thể thử lại job: {error}"))??;
    Ok(result)
}

#[tauri::command]
pub fn create_project(
    app: tauri::AppHandle,
    name: String,
    storage_directory: Option<String>,
) -> Result<Project, String> {
    let projects = store(&app)?;
    let created = match storage_directory.as_deref() {
        Some(directory) => projects.create_at(&name, Some(std::path::Path::new(directory)))?,
        None => projects.create(&name)?,
    };
    if let Err(error) = settings_service::migrate_legacy_custom_prompt(&app) {
        log::warn!("Could not migrate legacy project translation prompt: {error}");
    }
    projects.get(&created.id)
}

#[tauri::command]
pub fn get_project(app: tauri::AppHandle, id: String) -> Result<Project, String> {
    store(&app)?.get(&id)
}

#[tauri::command]
pub fn rename_project(app: tauri::AppHandle, id: String, name: String) -> Result<Project, String> {
    store(&app)?.rename(&id, &name)
}

#[tauri::command]
pub async fn delete_project(
    app: tauri::AppHandle,
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    video_tools_jobs: tauri::State<'_, video_tools_service::VideoToolsJobs>,
    id: String,
) -> Result<(), String> {
    let root = projects_root(&app)?;
    let job_store = job_store.inner().clone();
    let asr_jobs = asr_jobs.inner().clone();
    let translation_jobs = translation_jobs.inner().clone();
    let tts_jobs = tts_jobs.inner().clone();
    let render_jobs = render_jobs.inner().clone();
    let video_tools_jobs = video_tools_jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let _start_guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        asr_service::set_project_deleting(&asr_jobs, &id)?;
        if let Err(error) = translation_service::set_project_deleting(&translation_jobs, &id) {
            asr_service::clear_project_deleting(&asr_jobs, &id);
            return Err(error);
        }
        if let Err(error) = tts_service::set_project_deleting(&tts_jobs, &id) {
            asr_service::clear_project_deleting(&asr_jobs, &id);
            translation_service::clear_project_deleting(&translation_jobs, &id);
            return Err(error);
        }
        if let Err(error) = render_service::set_project_deleting(&render_jobs, &id) {
            asr_service::clear_project_deleting(&asr_jobs, &id);
            translation_service::clear_project_deleting(&translation_jobs, &id);
            tts_service::clear_project_deleting(&tts_jobs, &id);
            return Err(error);
        }
        if let Err(error) = video_tools_jobs.set_project_deleting(&id) {
            asr_service::clear_project_deleting(&asr_jobs, &id);
            translation_service::clear_project_deleting(&translation_jobs, &id);
            tts_service::clear_project_deleting(&tts_jobs, &id);
            render_service::clear_project_deleting(&render_jobs, &id);
            return Err(error);
        }
        let result = (|| {
            asr_service::cancel_project_jobs_and_wait(&asr_jobs, &id)?;
            translation_service::cancel_project_jobs_and_wait_ref(&translation_jobs, &id)?;
            tts_service::cancel_project_jobs_and_wait(&tts_jobs, &id)?;
            render_service::cancel_project_jobs_and_wait(&render_jobs, &id)?;
            video_tools_jobs.cancel_project_jobs_and_wait(&id)?;
            ProjectStore::new(root).delete(&id)
        })();
        if result.is_ok() {
            job_store.cancel_project_records(&id);
        }
        asr_service::clear_project_deleting(&asr_jobs, &id);
        translation_service::clear_project_deleting(&translation_jobs, &id);
        tts_service::clear_project_deleting(&tts_jobs, &id);
        render_service::clear_project_deleting(&render_jobs, &id);
        video_tools_jobs.clear_project_deleting(&id);
        result
    })
    .await
    .map_err(|error| format!("Không thể xóa dự án: {error}"))?
}

#[tauri::command]
pub async fn validate_export(
    app: tauri::AppHandle,
    project_id: String,
    options: render_spec::ExportOptions,
) -> Result<render_service::ExportValidation, String> {
    let root = projects_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        render_service::validate_export(&ProjectStore::new(root), &project_id, &options)
    })
    .await
    .map_err(|error| format!("Không thể kiểm tra video export: {error}"))?
}

#[tauri::command]
pub async fn start_export(
    app: tauri::AppHandle,
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    jobs: tauri::State<'_, render_service::RenderJobs>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    project_id: String,
    options: render_spec::ExportOptions,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let job_store = job_store.inner().clone();
    let asr_jobs = asr_jobs.inner().clone();
    let translation_jobs = translation_jobs.inner().clone();
    let tts_jobs = tts_jobs.inner().clone();
    let root = projects_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        let payload = serde_json::to_value(&options).map_err(|error| error.to_string())?;
        if let Some(existing) = job_store.find_active_duplicate(&project_id, "export", &payload)? {
            return Ok(existing.id);
        }
        if asr_service::has_active_project_job(&asr_jobs, &project_id)
            || translation_service::has_active_project_job(&translation_jobs, &project_id)
            || tts_service::has_active_project_job(&tts_jobs, &project_id)
        {
            return Err(
                "Hãy đợi Speech Recognition, dịch hoặc TTS hoàn tất trước khi export.".into(),
            );
        }
        render_service::start_export(app, &jobs, ProjectStore::new(root), project_id, options)
    })
    .await
    .map_err(|error| format!("Không thể khởi chạy export: {error}"))?
}

#[tauri::command]
pub fn cancel_export(
    jobs: tauri::State<'_, render_service::RenderJobs>,
    job_id: String,
) -> Result<bool, String> {
    render_service::cancel_export(&jobs, &job_id)
}

#[tauri::command]
pub fn get_export_history(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<Vec<render_service::ExportHistoryEntry>, String> {
    render_service::history(&store(&app)?, &project_id)
}

#[tauri::command]
pub fn get_export_directory(app: tauri::AppHandle, project_id: String) -> Result<String, String> {
    Ok(
        render_service::export_directory(&store(&app)?, &project_id)?
            .to_string_lossy()
            .into_owned(),
    )
}

#[tauri::command]
pub async fn inspect_tiktok_cookie_file(
    cookie_file: String,
) -> Result<video_tools_service::TikTokCookieInfo, String> {
    tauri::async_runtime::spawn_blocking(move || video_tools_service::inspect_tiktok_cookie_file(&cookie_file))
        .await
        .map_err(|error| format!("Không thể kiểm tra file cookies TikTok: {error}"))?
}

#[tauri::command]
pub fn has_tiktok_browser_session(app: tauri::AppHandle) -> Result<bool, String> {
    video_tools_service::has_tiktok_browser_session(&app)
}

#[tauri::command]
pub async fn start_tiktok_browser_login(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, video_tools_service::VideoToolsJobs>,
    project_id: String,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let root = projects_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK.lock().map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        video_tools_service::start_tiktok_browser_login(app, &jobs, ProjectStore::new(root), project_id)
    })
    .await
    .map_err(|error| format!("Không thể kết nối TikTok bằng trình duyệt: {error}"))?
}

#[tauri::command]
pub fn get_video_tool_output_directory(
    app: tauri::AppHandle,
    project_id: String,
    purpose: String,
) -> Result<String, String> {
    let project_store = store(&app)?;
    match purpose.as_str() {
        "tiktok" => video_tools_service::default_drama_directory(&project_store, &project_id),
        "merge" => video_tools_service::default_merge_directory(&project_store, &project_id),
        _ => Err("Loại thư mục Kéo video không hợp lệ.".into()),
    }
}

#[tauri::command]
pub async fn scan_merge_videos(
    source_directory: String,
) -> Result<video_tools_service::MergeScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || video_tools_service::scan_merge_videos(&source_directory))
        .await
        .map_err(|error| format!("Không thể quét thư mục video: {error}"))?
}

#[tauri::command]
pub async fn start_tiktok_drama_download(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, video_tools_service::VideoToolsJobs>,
    project_id: String,
    url: String,
    output_directory: Option<String>,
    cookie_file: Option<String>,
    use_browser_cookies: bool,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let root = projects_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK.lock().map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        video_tools_service::start_tiktok_download(app, &jobs, ProjectStore::new(root), project_id, url, output_directory, cookie_file, use_browser_cookies)
    })
    .await
    .map_err(|error| format!("Không thể bắt đầu tải TikTok Drama: {error}"))?
}

#[tauri::command]
pub async fn start_video_merge(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, video_tools_service::VideoToolsJobs>,
    project_id: String,
    source_directory: String,
    file_paths: Vec<String>,
    output_directory: Option<String>,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let root = projects_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK.lock().map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        video_tools_service::start_video_merge(app, &jobs, ProjectStore::new(root), project_id, source_directory, file_paths, output_directory)
    })
    .await
    .map_err(|error| format!("Không thể bắt đầu gộp video: {error}"))?
}

#[tauri::command]
pub fn open_export_folder(
    app: tauri::AppHandle,
    project_id: String,
    requested_directory: Option<String>,
) -> Result<String, String> {
    render_service::open_export_folder(&store(&app)?, &project_id, requested_directory.as_deref())
}

#[tauri::command]
pub fn get_app_settings(
    app: tauri::AppHandle,
) -> Result<settings_service::AppSettingsResponse, String> {
    settings_service::get_settings(&app)
}

#[tauri::command]
pub fn save_app_settings(
    app: tauri::AppHandle,
    settings: settings_service::AppSettings,
) -> Result<settings_service::AppSettingsResponse, String> {
    settings_service::save_settings(&app, settings)
}

#[tauri::command]
pub fn set_translation_credential(
    provider: settings_service::TranslationProviderId,
    secret: String,
) -> Result<(), String> {
    settings_service::set_credential(provider, &secret)
}

#[tauri::command]
pub fn delete_translation_credential(
    provider: settings_service::TranslationProviderId,
) -> Result<(), String> {
    settings_service::delete_credential(provider)
}

#[tauri::command]
pub async fn test_translation_provider(
    app: tauri::AppHandle,
    provider: settings_service::TranslationProviderId,
) -> Result<translation_service::TranslationTestResult, String> {
    translation_service::test_provider(app, provider).await
}

#[tauri::command]
pub async fn start_translation(
    app: tauri::AppHandle,
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    jobs: tauri::State<'_, translation_service::TranslationJobs>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    project_id: String,
    request: translation_service::TranslationJobRequest,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let job_store = job_store.inner().clone();
    let asr_jobs = asr_jobs.inner().clone();
    let tts_jobs = tts_jobs.inner().clone();
    let render_jobs = render_jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        let payload = serde_json::to_value(&request).map_err(|error| error.to_string())?;
        if let Some(existing) = job_store.find_active_duplicate(&project_id, "translate", &payload)? {
            return Ok(existing.id);
        }
        if asr_service::has_active_project_job(&asr_jobs, &project_id) {
            return Err("Speech Recognition đang thay thế transcript của dự án. Hãy đợi tác vụ kết thúc trước khi dịch.".to_string());
        }
        if tts_service::has_active_project_job(&tts_jobs, &project_id) {
            return Err("TTS đang chạy trong dự án. Hãy đợi hoặc hủy TTS trước khi dịch.".to_string());
        }
        if render_service::has_active_project_job(&render_jobs, &project_id) { return Err("Video đang được export. Hãy đợi hoặc hủy export trước khi dịch.".into()); }
        translation_service::start_translation_job(app, &jobs, project_id, request)
    })
    .await
    .map_err(|error| format!("Không thể khởi chạy translation job: {error}"))?
}

#[tauri::command]
pub fn cancel_translation(
    jobs: tauri::State<'_, translation_service::TranslationJobs>,
    job_id: String,
) -> Result<bool, String> {
    translation_service::cancel_translation(jobs, job_id)
}

#[tauri::command]
pub fn update_translation_languages(
    app: tauri::AppHandle,
    project_id: String,
    source_language: String,
    target_language: String,
) -> Result<Project, String> {
    validate_language(&source_language, true)?;
    validate_language(&target_language, false)?;
    let projects = store(&app)?;
    let mut project = projects.get(&project_id)?;
    project.translation.source_language = source_language;
    project.translation.target_language = target_language;
    projects.update_project(&project)
}

#[tauri::command]
pub fn update_subtitle_settings(
    app: tauri::AppHandle,
    project_id: String,
    subtitle: SubtitleSettings,
) -> Result<Project, String> {
    overlays_service::update_subtitle_settings(&store(&app)?, &project_id, subtitle)
}

#[tauri::command]
pub fn save_visual_settings(
    app: tauri::AppHandle,
    project_id: String,
    settings: overlays_service::VisualSettings,
) -> Result<Project, String> {
    overlays_service::save_visual_settings(&store(&app)?, &project_id, settings)
}

#[tauri::command]
pub async fn import_logo(
    app: tauri::AppHandle,
    project_id: String,
    source_path: String,
) -> Result<overlays_service::LogoMutationResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        overlays_service::import_logo(
            &ProjectStore::new(projects_root(&app)?),
            &project_id,
            &source_path,
        )
    })
    .await
    .map_err(|error| format!("Không thể nhập logo: {error}"))?
}

#[tauri::command]
pub async fn replace_logo(
    app: tauri::AppHandle,
    project_id: String,
    logo_id: String,
    source_path: String,
) -> Result<overlays_service::LogoMutationResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        overlays_service::replace_logo(
            &ProjectStore::new(projects_root(&app)?),
            &project_id,
            &logo_id,
            &source_path,
        )
    })
    .await
    .map_err(|error| format!("Không thể thay ảnh logo: {error}"))?
}

#[tauri::command]
pub async fn remove_logo(
    app: tauri::AppHandle,
    project_id: String,
    logo_id: String,
) -> Result<overlays_service::LogoMutationResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        overlays_service::remove_logo(
            &ProjectStore::new(projects_root(&app)?),
            &project_id,
            &logo_id,
        )
    })
    .await
    .map_err(|error| format!("Không thể xóa logo: {error}"))?
}

#[tauri::command]
pub fn get_logo_asset_path(
    app: tauri::AppHandle,
    project_id: String,
    logo_id: String,
) -> Result<String, String> {
    overlays_service::logo_asset_path(&store(&app)?, &project_id, &logo_id)
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn get_tts_environment(
    app: tauri::AppHandle,
) -> Result<tts_service::TtsEnvironment, String> {
    tauri::async_runtime::spawn_blocking(move || tts_service::get_environment(app))
        .await
        .map_err(|error| format!("Không thể kiểm tra VieNeu-TTS: {error}"))?
}

#[tauri::command]
pub async fn get_dialogue_environment(app: tauri::AppHandle) -> Result<dialogue_service::DialogueEnvironment, String> {
    tauri::async_runtime::spawn_blocking(move || dialogue_service::get_environment(&app))
        .await.map_err(|error| format!("Không thể kiểm tra model tách thoại: {error}"))?
}

#[tauri::command]
pub async fn install_dialogue_model(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || dialogue_service::install_model(&app))
        .await.map_err(|error| format!("Không thể tải model tách thoại: {error}"))?
}

#[tauri::command]
pub async fn install_dialogue_model_from_file(app: tauri::AppHandle, source_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || dialogue_service::install_model_from_file(&app, std::path::Path::new(&source_path)))
        .await.map_err(|error| format!("Không thể nhập model tách thoại: {error}"))?
}

#[tauri::command]
pub async fn remove_dialogue_model(app: tauri::AppHandle, jobs: tauri::State<'_, render_service::RenderJobs>) -> Result<(), String> {
    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || dialogue_service::remove_model(&app, &jobs))
        .await.map_err(|error| format!("Không thể gỡ model tách thoại: {error}"))?
}

#[tauri::command]
pub async fn install_tts_model(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || tts_service::install_model(app))
        .await
        .map_err(|error| format!("Không thể tải model VieNeu: {error}"))?
}

#[tauri::command]
pub async fn remove_tts_model(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, tts_service::TtsJobs>,
) -> Result<(), String> {
    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        tts_service::remove_model(app, &jobs)
    })
    .await
    .map_err(|error| format!("Không thể gỡ model VieNeu-TTS: {error}"))?
}

#[tauri::command]
pub async fn preview_tts_voice(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, tts_service::TtsJobs>,
    voice: String,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        tts_service::preview_voice(app, &jobs, voice)
    })
    .await
    .map_err(|error| format!("Không thể nghe thử giọng VieNeu: {error}"))?
}

#[tauri::command]
pub async fn add_tts_voice_profile(
    app: tauri::AppHandle,
    name: String,
    source_path: String,
    consent: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        tts_service::add_voice_profile(&app, &name, &source_path, consent)
    })
    .await
    .map_err(|error| format!("Không thể thêm voice profile: {error}"))?
}

#[tauri::command]
pub async fn delete_tts_voice_profile(
    app: tauri::AppHandle,
    profile_id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        tts_service::delete_voice_profile(&app, &profile_id)
    })
    .await
    .map_err(|error| format!("Không thể xóa voice profile: {error}"))?
}

#[tauri::command]
pub async fn start_tts_generation(
    app: tauri::AppHandle,
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    jobs: tauri::State<'_, tts_service::TtsJobs>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    project_id: String,
    request: tts_service::TtsJobRequest,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let job_store = job_store.inner().clone();
    let asr_jobs = asr_jobs.inner().clone();
    let translation_jobs = translation_jobs.inner().clone();
    let render_jobs = render_jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        let payload = serde_json::to_value(&request).map_err(|error| error.to_string())?;
        if let Some(existing) = job_store.find_active_duplicate(&project_id, "tts", &payload)? {
            return Ok(existing.id);
        }
        if asr_service::has_active_project_job(&asr_jobs, &project_id) {
            return Err("Speech Recognition đang chạy trong dự án.".to_string());
        }
        if translation_service::has_active_project_job(&translation_jobs, &project_id) {
            return Err(
                "Đang dịch transcript. Hãy đợi translation job hoàn tất trước khi tạo TTS."
                    .to_string(),
            );
        }
        if render_service::has_active_project_job(&render_jobs, &project_id) {
            return Err(
                "Video đang được export. Hãy đợi hoặc hủy export trước khi tạo TTS.".into(),
            );
        }
        if tts_service::has_active_project_job(&jobs, &project_id) {
            return Err("Đã có TTS job đang chạy trong dự án.".to_string());
        }
        tts_service::start_generation(app, &jobs, project_id, request)
    })
    .await
    .map_err(|error| format!("Không thể khởi chạy TTS: {error}"))?
}

#[tauri::command]
pub fn cancel_tts_generation(
    jobs: tauri::State<'_, tts_service::TtsJobs>,
    job_id: String,
) -> Result<bool, String> {
    tts_service::cancel(&jobs, &job_id)
}

#[tauri::command]
pub fn get_tts_audio_path(
    app: tauri::AppHandle,
    project_id: String,
    segment_id: String,
) -> Result<String, String> {
    let path = tts_service::audio_path(&store(&app)?, &project_id, &segment_id)?;
    allow_verified_media_asset(&app, std::path::Path::new(&path))?;
    Ok(path)
}

#[tauri::command]
pub fn update_dubbing_settings(
    app: tauri::AppHandle,
    project_id: String,
    dubbing: crate::project_service::DubbingSettings,
) -> Result<Project, String> {
    if !dubbing.original_volume.is_finite()
        || !(0.0..=1.0).contains(&dubbing.original_volume)
        || !dubbing.dubbed_volume.is_finite()
        || !(0.0..=1.0).contains(&dubbing.dubbed_volume)
        || !dubbing.ducking_amount.is_finite()
        || !(0.0..=1.0).contains(&dubbing.ducking_amount)
        || !dubbing.dialogue_reduction_amount.is_finite()
        || !(0.0..=1.0).contains(&dubbing.dialogue_reduction_amount)
        || !matches!(dubbing.provider.as_str(), "vieneu")
        || dubbing.model_id != "v3-turbo"
        || dubbing
            .fit_mode
            .as_deref()
            .is_some_and(|mode| !matches!(mode, "natural" | "fit" | "custom"))
        || dubbing
            .speed
            .is_some_and(|speed| !speed.is_finite() || !(0.85..=2.0).contains(&speed))
        || dubbing
            .voice
            .as_ref()
            .is_some_and(|voice| voice.trim().is_empty() || voice.chars().count() > 100)
        || dubbing
            .voice_profile_id
            .as_ref()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err("Cấu hình lồng tiếng hoặc âm lượng không hợp lệ.".to_string());
    }
    let projects = store(&app)?;
    let mut project = projects.get(&project_id)?;
    project.dubbing = dubbing;
    projects.update_project(&project)
}

#[tauri::command]
pub fn update_project_preferences(
    app: tauri::AppHandle,
    project_id: String,
    custom_prompt: String,
    voice: Option<String>,
    voice_profile_id: Option<String>,
    transcript_settings: Option<crate::project_service::TranscriptSettings>,
) -> Result<Project, String> {
    crate::project_service::validate_translation_custom_prompt(&custom_prompt)?;
    let selected_voice = if let Some(profile_id) = voice_profile_id.as_deref() {
        uuid::Uuid::parse_str(profile_id)
            .map_err(|_| "Voice profile của dự án không hợp lệ.".to_string())?;
        let settings = settings_service::read_settings(&settings_service::settings_path(&app)?)?;
        Some(settings
            .tts
            .voice_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .ok_or_else(|| "Voice profile không còn tồn tại trong cài đặt ứng dụng.".to_string())?)
    } else {
        if let Some(voice) = voice.as_deref() {
            let voice = voice.trim();
            if voice.is_empty() || voice.chars().count() > 100 || voice.chars().any(char::is_control) {
                return Err("Giọng TTS của dự án không hợp lệ.".to_string());
            }
            Some(voice.to_string())
        } else { None }
    };

    let projects = store(&app)?;
    let mut project = projects.get(&project_id)?;
    if let Some(settings) = transcript_settings {
        validate_transcript_settings(&settings)?;
        project.transcript_settings = settings;
    }
    project.translation.custom_prompt = custom_prompt;
    if let Some(selected_voice) = selected_voice { project.dubbing.voice = Some(selected_voice); }
    if voice.is_some() || voice_profile_id.is_some() { project.dubbing.voice_profile_id = voice_profile_id; }
    projects.update_project(&project)
}

#[tauri::command]
pub fn update_transcript_settings(
    app: tauri::AppHandle,
    project_id: String,
    settings: crate::project_service::TranscriptSettings,
) -> Result<Project, String> {
    validate_transcript_settings(&settings)?;
    let projects = store(&app)?;
    let mut project = projects.get(&project_id)?;
    project.transcript_settings = settings;
    projects.update_project(&project)
}

fn validate_transcript_settings(settings: &crate::project_service::TranscriptSettings) -> Result<(), String> {
    if ![1.0, 2.0, 4.0].contains(&settings.ocr_fps) {
        return Err("Tốc độ lấy mẫu OCR chỉ hỗ trợ 1, 2 hoặc 4 khung hình/giây.".into());
    }
    if let Some(region) = settings.ocr_region.as_ref() {
        let values = [region.x, region.y, region.width, region.height];
        if values.iter().any(|value| !value.is_finite())
            || region.x < 0.0 || region.y < 0.0
            || region.width <= 0.0 || region.height <= 0.0
            || region.x + region.width > 1.000001
            || region.y + region.height > 1.000001
        {
            return Err("Vùng OCR không hợp lệ. Hãy chọn lại vùng bên trong khung video.".into());
        }
    }
    Ok(())
}

fn validate_language(value: &str, allow_auto: bool) -> Result<(), String> {
    if allow_auto && value == "auto" {
        return Ok(());
    }
    let parts: Vec<&str> = value.split('-').collect();
    if parts.is_empty()
        || !(2..=3).contains(&parts[0].len())
        || !parts[0].bytes().all(|b| b.is_ascii_alphabetic())
        || parts.iter().skip(1).any(|part| {
            part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric())
        })
    {
        return Err("Mã ngôn ngữ không hợp lệ.".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn import_video(
    app: tauri::AppHandle,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    asr_jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    project_id: String,
    source_path: String,
    replace: bool,
) -> Result<video_service::VideoImportResult, String> {
    let root = projects_root(&app)?;
    let render_jobs = render_jobs.inner().clone();
    let asr_jobs = asr_jobs.inner().clone();
    let translation_jobs = translation_jobs.inner().clone();
    let tts_jobs = tts_jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        if render_service::has_active_project_job(&render_jobs, &project_id) {
            return Err(
                "Video đang được export. Hãy đợi hoặc hủy export trước khi thay nguồn.".into(),
            );
        }
        if asr_service::has_active_project_job(&asr_jobs, &project_id)
            || translation_service::has_active_project_job(&translation_jobs, &project_id)
            || tts_service::has_active_project_job(&tts_jobs, &project_id)
        {
            return Err(
                "Dự án đang có tác vụ xử lý. Hãy đợi hoặc hủy tác vụ trước khi thay video nguồn."
                    .into(),
            );
        }
        video_service::import_video(&ProjectStore::new(root), &project_id, &source_path, replace)
    })
    .await
    .map_err(|error| format!("Không thể hoàn tất nhập video: {error}"))?
}

#[tauri::command]
pub fn get_video_asset_path(app: tauri::AppHandle, project_id: String) -> Result<String, String> {
    let path = video_service::resolve_video(&store(&app)?, &project_id)?;
    allow_verified_media_asset(&app, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

fn allow_verified_media_asset(app: &tauri::AppHandle, path: &std::path::Path) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh đường dẫn media: {error}"))?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("Không thể xác minh tệp media: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Tệp media không hợp lệ hoặc là liên kết ngoài.".into());
    }
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("Không thể cấp quyền đọc preview media: {error}"))
}

#[tauri::command]
pub async fn create_preview_proxy(
    app: tauri::AppHandle,
    project_id: String,
    force_rebuild: bool,
) -> Result<String, String> {
    let root = projects_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = video_service::create_preview_proxy(
            &ProjectStore::new(root),
            &project_id,
            force_rebuild,
        )?;
        allow_verified_media_asset(&app, &path)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| format!("Không thể tạo preview video: {error}"))?
}

#[tauri::command]
pub fn get_video_metadata(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<ProjectSource, String> {
    let project_store = store(&app)?;
    video_service::resolve_video(&project_store, &project_id)?;
    let project = project_store.get(&project_id)?;
    Ok(project.source)
}

#[tauri::command]
pub fn get_subtitle_tracks(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<Vec<SubtitleTrackMetadata>, String> {
    let project_store = store(&app)?;
    video_service::resolve_video(&project_store, &project_id)?;
    Ok(project_store.get(&project_id)?.source.subtitle_tracks)
}

#[tauri::command]
pub async fn extract_subtitle(
    app: tauri::AppHandle,
    project_id: String,
    track_index: u32,
) -> Result<transcript_service::Transcript, String> {
    let root = projects_root(&app)?;
    let (transcript, _) = tauri::async_runtime::spawn_blocking(move || {
        transcript_service::extract_subtitle(&ProjectStore::new(root), &project_id, track_index)
    })
    .await
    .map_err(|error| format!("Không thể trích xuất subtitle: {error}"))??;
    Ok(transcript)
}

#[tauri::command]
pub fn get_transcript(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<Option<Transcript>, String> {
    crate::tts_service::get_transcript_with_audio_validation(&store(&app)?, &project_id)
}

#[tauri::command]
pub fn save_transcript(
    app: tauri::AppHandle,
    project_id: String,
    transcript: Transcript,
) -> Result<Project, String> {
    transcript_service::save_transcript(&store(&app)?, &project_id, transcript)
}

#[tauri::command]
pub async fn get_asr_environment(
    app: tauri::AppHandle,
    model: String,
    alignment_language: Option<String>,
) -> Result<asr_service::AsrEnvironment, String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::get_asr_environment(app, model, alignment_language))
        .await
        .map_err(|error| format!("Không thể kiểm tra môi trường Speech Recognition: {error}"))?
}

#[tauri::command]
pub async fn get_ocr_environment(app: tauri::AppHandle, model: String) -> Result<asr_service::OcrEnvironment, String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::get_ocr_environment(app, model))
        .await
        .map_err(|error| format!("Không thể kiểm tra môi trường OCR: {error}"))?
}

#[tauri::command]
pub async fn install_ocr_runtime(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::install_ocr_runtime(app))
        .await
        .map_err(|error| format!("Không thể cài runtime OCR: {error}"))?
}

#[tauri::command]
pub async fn install_ocr_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::install_ocr_model(app, model))
        .await
        .map_err(|error| format!("Không thể cài model OCR: {error}"))?
}

#[tauri::command]
pub async fn remove_ocr_model(app: tauri::AppHandle, model: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::remove_ocr_model(app, model))
        .await
        .map_err(|error| format!("Không thể gỡ model OCR: {error}"))?
}

#[tauri::command]
pub async fn install_whisperx_alignment(
    app: tauri::AppHandle,
    language: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::install_whisperx_alignment(app, language))
        .await
        .map_err(|error| format!("Không thể khởi tạo cài đặt aligner WhisperX: {error}"))?
}

#[tauri::command]
pub async fn install_asr_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || asr_service::install_model(app, model))
        .await
        .map_err(|error| format!("Không thể khởi tạo cài đặt model Speech Recognition: {error}"))?
}

#[tauri::command]
pub async fn remove_asr_model(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, asr_service::AsrJobs>,
    model: String,
) -> Result<(), String> {
    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PROJECT_JOB_START_LOCK
            .lock()
            .map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        asr_service::remove_model(app, &jobs, model)
    })
    .await
    .map_err(|error| format!("Không thể gỡ model faster-whisper: {error}"))?
}

#[tauri::command]
pub async fn generate_transcript(
    app: tauri::AppHandle,
    job_store: tauri::State<'_, crate::job_service::JobStore>,
    jobs: tauri::State<'_, asr_service::AsrJobs>,
    translation_jobs: tauri::State<'_, translation_service::TranslationJobs>,
    tts_jobs: tauri::State<'_, tts_service::TtsJobs>,
    render_jobs: tauri::State<'_, render_service::RenderJobs>,
    project_id: String,
    options: asr_service::AsrOptions,
) -> Result<String, String> {
    let jobs = jobs.inner().clone();
    let job_store = job_store.inner().clone();
    let translation_jobs = translation_jobs.inner().clone();
    let tts_jobs = tts_jobs.inner().clone();
    let render_jobs = render_jobs.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _guard = PROJECT_JOB_START_LOCK.lock().map_err(|_| "Không thể khóa trạng thái project jobs.".to_string())?;
        let payload = serde_json::to_value(&options).map_err(|error| error.to_string())?;
        if let Some(existing) = job_store.find_active_duplicate(&project_id, "transcribe", &payload)? {
            return Ok(existing.id);
        }
        if translation_service::has_active_project_job(&translation_jobs, &project_id) {
            return Err("Translation đang chạy trong dự án. Hãy đợi tác vụ kết thúc trước khi tạo lại transcript.".to_string());
        }
        if tts_service::has_active_project_job(&tts_jobs, &project_id) {
            return Err("TTS đang chạy trong dự án. Hãy đợi hoặc hủy TTS trước khi tạo lại transcript.".to_string());
        }
        if render_service::has_active_project_job(&render_jobs, &project_id) { return Err("Video đang được export. Hãy đợi hoặc hủy export trước khi tạo lại transcript.".into()); }
        asr_service::generate_transcript(app, &jobs, project_id, options)
    })
    .await
    .map_err(|error| format!("Không thể khởi tạo Speech Recognition: {error}"))?
}

#[tauri::command]
pub fn cancel_transcription(
    jobs: tauri::State<'_, asr_service::AsrJobs>,
    job_id: String,
) -> Result<bool, String> {
    asr_service::cancel_transcription(jobs, job_id)
}

pub fn projects_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let mut root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu ứng dụng: {error}"))?;
    root.push("projects");
    Ok(root)
}
