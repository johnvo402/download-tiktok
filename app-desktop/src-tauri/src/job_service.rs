use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};

const HISTORY_LIMIT: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    #[serde(rename = "type")]
    pub job_type: String,
    pub status: String,
    pub priority: String,
    pub resources: Vec<String>,
    pub progress: f64,
    pub message: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub attempt: u32,
    pub retry_of: Option<String>,
    pub error: Option<JobError>,
    /// Typed, non-secret request options used only to retry supported jobs.
    pub retry_payload: Option<Value>,
}

#[derive(Clone, Default)]
pub struct JobStore {
    inner: Arc<Mutex<JobStoreData>>,
}

#[derive(Default)]
struct JobStoreData {
    file: Option<PathBuf>,
    jobs: Vec<JobRecord>,
    last_event_persist: Option<Instant>,
    persistence_failed: bool,
    pending_warning: Option<String>,
}

impl JobStore {
    pub fn initialize(&self, file: PathBuf) -> Result<(), String> {
        let parent = file
            .parent()
            .ok_or_else(|| "Không xác định được thư mục lưu job history.".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Không thể tạo thư mục lưu job history: {error}"))?;

        let mut jobs = if file.exists() || crate::recovery_store::backup_path(&file,1).exists() {
            match crate::recovery_store::read(&file, None, |bytes| serde_json::from_slice::<Vec<JobRecord>>(bytes).map_err(|_| "Job history không hợp lệ.".to_string())) {
                Ok(jobs) => jobs,
                Err(error) => {
                    // History is not project data. Preserve unreadable data and
                    // continue only when an invalid JSON file can be backed up.
                    let bytes=fs::read(&file).map_err(|e|e.to_string())?;
                    if serde_json::from_slice::<Vec<JobRecord>>(&bytes).is_ok() { return Err(error); }
                    crate::recovery_store::preserve_corrupt(&file,&bytes)?;
                    log::warn!("Invalid job history preserved; starting empty history");
                    Vec::new()
                }
            }
        } else { Vec::new() };
        let now = timestamp();
        for job in &mut jobs {
            if matches!(job.status.as_str(), "running" | "queued" | "waiting" | "cancelling") {
                job.status = "interrupted".into();
                job.completed_at = Some(now.clone());
                job.message = "Ứng dụng đã đóng trước khi tác vụ hoàn tất. Có thể thử lại.".into();
                job.error = Some(JobError {
                    code: "APP_RESTARTED".into(),
                    message: job.message.clone(),
                    retryable: job.retry_payload.is_some(),
                });
            }
        }
        trim_history(&mut jobs);

        let mut state = self
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập job history.".to_string())?;
        state.file = Some(file);
        state.jobs = jobs;
        persist(&state)
    }

    pub fn register(&self, record: JobRecord) -> Result<(),String> {
        let mut state=self.inner.lock().map_err(|_|"Không truy cập được lịch sử tác vụ.")?;
        let previous=state.jobs.clone();
        state.jobs.retain(|job|job.id!=record.id);state.jobs.push(record);trim_history(&mut state.jobs);
        if let Err(error)=persist(&state) {state.jobs=previous;return Err(format!("Không lưu được lịch sử tác vụ; chưa bắt đầu xử lý. {error}"));}
        Ok(())
    }

    pub fn take_warning(&self) -> Option<String> {self.inner.lock().ok()?.pending_warning.take()}
    pub fn get(&self, id: &str) -> Result<JobRecord, String> {
        self.inner
            .lock()
            .map_err(|_| "Không thể truy cập job history.".to_string())?
            .jobs
            .iter()
            .find(|job| job.id == id)
            .cloned()
            .ok_or_else(|| "Không tìm thấy job trong history.".to_string())
    }

    pub fn list(&self) -> Result<Vec<JobRecord>, String> {
        let mut jobs = self
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập job history.".to_string())?
            .jobs
            .clone();
        jobs.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(jobs)
    }

    pub fn find_active_duplicate(
        &self,
        project_id: &str,
        job_type: &str,
        retry_payload: &Value,
    ) -> Result<Option<JobRecord>, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| "Không thể truy cập job history.".to_string())?
            .jobs
            .iter()
            .rev()
            .find(|job| {
                job.project_id.as_deref() == Some(project_id)
                    && job.job_type == job_type
                    && is_active(&job.status)
                    && job.retry_payload.as_ref() == Some(retry_payload)
            })
            .cloned())
    }

    pub fn sync_event(&self, event_type: &str, event: &Value) -> Option<JobRecord> {
        let id = event.get("jobId")?.as_str()?;
        let event_status = event.get("status")?.as_str()?;
        let status = normalized_status(event_type, event_status);
        let now = timestamp();
        let mut state = self.inner.lock().ok()?;
        let record = state.jobs.iter_mut().find(|job| job.id == id)?;
        // Delayed events from a stopped attempt must not resurrect a job.
        if is_terminal(&record.status) { return None; }
        if record.status == "cancelling" && !is_terminal(status) { return None; }
        record.status = status.into();
        record.message = event
            .get("message")
            .and_then(Value::as_str)
            .map(short_message)
            .unwrap_or_else(|| record.message.clone());
        record.progress = progress_for(event_type, event).clamp(0.0, 1.0);
        if record.status == "running" && record.started_at.is_none() {
            record.started_at = Some(now.clone());
        }
        if is_terminal(&record.status) {
            record.completed_at = Some(now);
        }
        if record.status == "failed" || record.status == "partial" {
            let code = event
                .get("errorCode")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| crate::app_error::AppError::classify(&record.message,crate::app_error::ErrorContext::default()).code);
            record.error = Some(JobError {
                code,
                message: record.message.clone(),
                retryable: record.retry_payload.is_some(),
            });
        } else if record.status == "completed" {
            record.progress = 1.0;
            record.error = None;
        } else if record.status == "cancelled" {
            record.error = Some(JobError {
                code: "CANCELLED".into(),
                message: record.message.clone(),
                retryable: record.retry_payload.is_some(),
            });
        }
        let result = record.clone();
        trim_history(&mut state.jobs);
        let should_persist = is_terminal(&result.status)
            || state.last_event_persist.map_or(true, |last| {
                last.elapsed() >= std::time::Duration::from_millis(500)
            });
        if should_persist {
            persist_event(&mut state);
            state.last_event_persist = Some(Instant::now());
        }
        Some(result)
    }

    pub fn link_retry(&self, new_id: &str, old_id: &str) {
        if let Ok(mut state) = self.inner.lock() {
            let previous_attempt = state
                .jobs
                .iter()
                .find(|job| job.id == old_id)
                .map(|job| job.attempt + 1)
                .unwrap_or(2);
            if let Some(job) = state.jobs.iter_mut().find(|job| job.id == new_id) {
                job.retry_of = Some(old_id.to_string());
                job.attempt = previous_attempt;
                persist_event(&mut state);
            }
        }
    }

    pub fn cancel_project_records(&self, project_id: &str) {
        if let Ok(mut state) = self.inner.lock() {
            let now = timestamp();
            for job in state.jobs.iter_mut().filter(|job| {
                job.project_id.as_deref() == Some(project_id)
                    && matches!(job.status.as_str(), "queued" | "waiting" | "running")
            }) {
                job.status = "cancelled".into();
                job.completed_at = Some(now.clone());
                job.message = "Job đã dừng vì dự án đang bị xóa.".into();
                job.error = Some(JobError {
                    code: "PROJECT_DELETED".into(),
                    message: job.message.clone(),
                    retryable: false,
                });
            }
            persist_event(&mut state);
        }
    }
}

pub fn register_job(
    app: &AppHandle,
    id: String,
    project_id: String,
    project_name: String,
    job_type: &str,
    resources: &[&str],
    retry_payload: Option<Value>,
) -> Result<(),String> {
    let record = JobRecord {
        id,
        project_id: Some(project_id),
        project_name: Some(project_name),
        job_type: job_type.into(),
        status: "queued".into(),
        priority: "normal".into(),
        resources: resources
            .iter()
            .map(|resource| (*resource).into())
            .collect(),
        progress: 0.0,
        message: "Đã thêm vào hàng đợi.".into(),
        created_at: timestamp(),
        started_at: None,
        completed_at: None,
        attempt: 1,
        retry_of: None,
        error: None,
        retry_payload,
    };
    if let Some(store) = app.try_state::<JobStore>() {
        store.register(record.clone())?;
        let _ = app.emit("job-record-updated", record);
    }
    Ok(())
}

pub fn sync_app_event(app: &AppHandle, event_type: &str, event: &impl Serialize) {
    let Some(store) = app.try_state::<JobStore>() else {
        return;
    };
    let Ok(value) = serde_json::to_value(event) else {
        return;
    };
    if let Some(record) = store.sync_event(event_type, &value) {
        let _ = app.emit("job-record-updated", record);
    }
    if let Some(message)=store.take_warning() {let _=app.emit("job-history-warning",message);}
}

/// Model/runtime installs have a journal even though pip/extraction cannot be
/// safely cancelled at every step. No credentials or request payloads are saved.
pub fn maintenance<T>(app: &AppHandle, job_type: &str, action: impl FnOnce() -> Result<T,String>) -> Result<T,String> {
    let id=uuid::Uuid::new_v4().to_string();
    let record=JobRecord {id:id.clone(),project_id:None,project_name:None,job_type:job_type.into(),status:"running".into(),priority:"normal".into(),resources:vec!["network".into(),"disk".into()],progress:0.0,
        message:"Đang cài thành phần. Không thể dừng ở bước cài đặt này.".into(),created_at:timestamp(),started_at:Some(timestamp()),completed_at:None,attempt:1,retry_of:None,error:None,retry_payload:None};
    if let Some(store)=app.try_state::<JobStore>() {store.register(record.clone())?;let _=app.emit("job-record-updated",record);}
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)).unwrap_or_else(|_|Err("Bộ cài đã dừng bất ngờ. Bản cài đang dùng được giữ nguyên; hãy thử lại trong Cài đặt.".into()));
    let message=result.as_ref().err().cloned().unwrap_or_else(||"Đã cài thành phần được chọn.".into());
    sync_app_event(app,"maintenance",&json_value(&id,if result.is_ok(){"completed"}else{"failed"},&message));
    result
}
fn json_value(id: &str,status: &str,message: &str) -> Value {serde_json::json!({"jobId":id,"status":status,"message":message})}

pub fn is_active(status: &str) -> bool {
    matches!(status, "queued" | "waiting" | "running" | "cancelling")
}

pub fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn normalized_status(event_type: &str, status: &str) -> &'static str {
    match (event_type, status) {
        ("transcription", "progress") | (_, "running") => "running",
        (_, "queued") => "queued",
        (_, "waiting") => "waiting",
        (_, "cancelling") => "cancelling",
        (_, "completed") => "completed",
        (_, "partial") => "partial",
        (_, "failed") => "failed",
        (_, "cancelled" | "canceled") => "cancelled",
        _ => "running",
    }
}

fn progress_for(event_type: &str, event: &Value) -> f64 {
    match event_type {
        "translation" => {
            let total = event
                .get("totalSegments")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let done = event
                .get("translatedSegments")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if total > 0.0 {
                done / total
            } else {
                0.0
            }
        }
        "transcription" => event.get("progress").and_then(Value::as_f64).unwrap_or(0.0),
        "tts" | "studio" => event.get("progress").and_then(Value::as_f64).unwrap_or(0.0),
        "export" | "video_tools" => event.get("progress").and_then(Value::as_f64).unwrap_or(0.0),
        _ => 0.0,
    }
}

fn is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "partial" | "failed" | "cancelled" | "interrupted"
    )
}

fn short_message(message: &str) -> String {
    let flattened = message.split_whitespace().collect::<Vec<_>>().join(" ");
    flattened.chars().take(500).collect()
}

fn trim_history(jobs: &mut Vec<JobRecord>) {
    jobs.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let terminal_count = jobs.iter().filter(|job| is_terminal(&job.status)).count();
    let mut remove_count = terminal_count.saturating_sub(HISTORY_LIMIT);
    if remove_count == 0 {
        return;
    }
    jobs.retain(|job| {
        if remove_count > 0 && is_terminal(&job.status) {
            remove_count -= 1;
            false
        } else {
            true
        }
    });
}

fn persist_event(state:&mut JobStoreData) {
    match persist(state) {
        Ok(())=>state.persistence_failed=false,
        Err(error)=>{
            log::error!("Job history save failed: {error}");
            if !state.persistence_failed {state.pending_warning=Some("Không lưu được lịch sử tác vụ. Dữ liệu đã xử lý được giữ lại, nhưng tiến trình gần nhất có thể không khôi phục sau khi đóng. Hãy kiểm tra dung lượng và quyền ghi.".into());}
            state.persistence_failed=true;
        }
    }
}

fn persist(state: &JobStoreData) -> Result<(), String> {
    let Some(path) = state.file.as_deref() else {
        return Ok(());
    };
    let parent = path
        .parent()
        .ok_or_else(|| "Không xác định được thư mục lưu job history.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Không thể tạo thư mục lưu job history: {error}"))?;
    let contents = serde_json::to_vec_pretty(&state.jobs)
        .map_err(|error| format!("Không thể mã hóa job history: {error}"))?;
    crate::recovery_store::write(path, &contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn failed_initial_journal_write_does_not_register_a_job() {
        let temp=tempdir().unwrap();let store=JobStore::default();store.inner.lock().unwrap().file=Some(temp.path().join("blocked.json"));
        fs::create_dir(temp.path().join("blocked.json")).unwrap();assert!(store.register(record("job","queued")).is_err());assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn stopped_attempt_cannot_be_resurrected_by_late_progress() {
        let store=JobStore::default();store.register(record("old","cancelled")).unwrap();
        assert!(store.sync_event("tts",&json!({"jobId":"old","status":"running","progress":0.9})).is_none());
        assert_eq!(store.get("old").unwrap().status,"cancelled");
        store.register(record("current","running")).unwrap();
        store.sync_event("cancel",&json!({"jobId":"current","status":"cancelling"})).unwrap();
        assert!(store.sync_event("tts",&json!({"jobId":"current","status":"running"})).is_none());
        assert_eq!(store.sync_event("tts",&json!({"jobId":"current","status":"cancelled"})).unwrap().status,"cancelled");
    }

    #[test]
    fn cancelling_jobs_are_interrupted_after_restart() {
        let dir=tempdir().unwrap();let path=dir.path().join("jobs.json");let first=JobStore::default();first.initialize(path.clone()).unwrap();first.register(record("job","cancelling")).unwrap();
        let reopened=JobStore::default();reopened.initialize(path).unwrap();assert_eq!(reopened.get("job").unwrap().status,"interrupted");
    }

    fn record(id: &str, status: &str) -> JobRecord {
        JobRecord {
            id: id.into(),
            project_id: Some("project-1".into()),
            project_name: Some("Project".into()),
            job_type: "translate".into(),
            status: status.into(),
            priority: "normal".into(),
            resources: vec!["network".into()],
            progress: 0.0,
            message: String::new(),
            created_at: timestamp(),
            started_at: None,
            completed_at: None,
            attempt: 1,
            retry_of: None,
            error: None,
            retry_payload: Some(json!({ "scope": "missing" })),
        }
    }

    #[test]
    fn restart_marks_nonterminal_jobs_interrupted_and_preserves_retry_payload() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("jobs.json");
        let initial = JobStore::default();
        initial.inner.lock().unwrap().file = Some(path.clone());
        initial.register(record("job-a", "running")).unwrap();

        let recovered = JobStore::default();
        recovered.initialize(path).unwrap();
        let job = recovered.get("job-a").unwrap();
        assert_eq!(job.status, "interrupted");
        assert!(job.error.unwrap().retryable);
    }

    #[test]
    fn event_updates_progress_and_terminal_error() {
        let store = JobStore::default();
        store.register(record("job-a", "queued")).unwrap();
        let updated = store
            .sync_event(
                "translation",
                &json!({ "jobId":"job-a", "status":"failed", "message":"Network timeout", "translatedSegments":4, "totalSegments":10, "errorCode":"NETWORK_TIMEOUT" }),
            )
            .unwrap();
        assert_eq!(updated.status, "failed");
        assert!((updated.progress - 0.4).abs() < 0.001);
        assert_eq!(updated.error.unwrap().code, "NETWORK_TIMEOUT");
    }

    #[test]
    fn identical_active_requests_attach_to_the_existing_job() {
        let store = JobStore::default();
        let payload = json!({ "scope":"missing", "segmentIds":[] });
        let mut job = record("job-a", "running");
        job.retry_payload = Some(payload.clone());
        store.register(job).unwrap();
        assert_eq!(
            store
                .find_active_duplicate("project-1", "translate", &payload)
                .unwrap()
                .unwrap()
                .id,
            "job-a"
        );
        assert!(store
            .find_active_duplicate("project-2", "translate", &payload)
            .unwrap()
            .is_none());
        assert!(store
            .find_active_duplicate("project-1", "tts", &payload)
            .unwrap()
            .is_none());
    }

    #[test]
    fn history_keeps_active_jobs_and_caps_terminal_records() {
        let mut jobs = (0..(HISTORY_LIMIT + 3))
            .map(|index| record(&format!("done-{index}"), "completed"))
            .collect::<Vec<_>>();
        jobs.push(record("active", "running"));
        trim_history(&mut jobs);
        assert_eq!(jobs.len(), HISTORY_LIMIT + 1);
        assert!(jobs.iter().any(|job| job.id == "active"));
    }
}
