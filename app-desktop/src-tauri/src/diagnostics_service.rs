use crate::runtime_health::{self, RuntimeHealth};
use crate::settings_service;
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SanitizedDiagnostics<'a> {
    app: &'a RuntimeHealth,
    settings: serde_json::Value,
    logs_directory: String,
    includes_user_media: bool,
    includes_transcripts: bool,
    includes_credentials: bool,
}

pub fn export(app: &AppHandle, destination: &str) -> Result<String, String> {
    let destination = PathBuf::from(destination);
    let metadata = fs::symlink_metadata(&destination)
        .map_err(|error| format!("Không thể mở thư mục đã chọn: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Hãy chọn một thư mục thông thường để lưu diagnostics.".into());
    }
    let destination = destination
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục đã chọn: {error}"))?;
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let bundle = destination.join(format!("JC-AI-Diagnostics-{timestamp}"));
    fs::create_dir(&bundle)
        .map_err(|error| format!("Không thể tạo diagnostics trong thư mục đã chọn: {error}"))?;

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu ứng dụng: {error}"))?;
    let settings =
        settings_service::read_settings(&settings_service::settings_path(app)?).unwrap_or_default();
    let health = runtime_health::check(app);
    let safe = SanitizedDiagnostics {
        app: &health,
        settings: json!({
            "translationProvider": settings.translation.provider,
            "translationBatchSize": settings.translation.batch_size,
            "asrModel": settings.asr.model,
            "asrDevice": settings.asr.device,
            "asrEngine": settings.asr.engine,
            "ttsProvider": settings.tts.provider,
            "ttsModel": settings.tts.model_id,
            "ttsDevice": settings.tts.device,
        }),
        logs_directory: "appDataDir/logs".to_string(),
        includes_user_media: false,
        includes_transcripts: false,
        includes_credentials: false,
    };
    let mut diagnostics = serde_json::to_vec_pretty(&safe)
        .map_err(|error| format!("Không thể tạo thông tin diagnostics: {error}"))?;
    diagnostics.push(b'\n');
    write_new(&bundle.join("diagnostics.json"), &diagnostics)?;
    write_new(
        &bundle.join("README.txt"),
        b"This diagnostics bundle contains sanitized runtime information and recent application logs. It excludes source videos, transcripts, API keys, credential values, and voice reference audio. Review the files before sharing.\n",
    )?;
    copy_recent_logs(&app_data.join("logs"), &bundle.join("logs"))?;
    Ok(bundle.to_string_lossy().into_owned())
}

fn copy_recent_logs(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Ok(());
    }
    let mut files = fs::read_dir(source)
        .map_err(|error| format!("Không thể đọc log ứng dụng: {error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).ok()?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("log")
            {
                return None;
            }
            Some((path, metadata.modified().ok()?))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    if files.is_empty() {
        return Ok(());
    }
    fs::create_dir(destination)
        .map_err(|error| format!("Không thể tạo thư mục log diagnostics: {error}"))?;
    for (source, _) in files.into_iter().take(5) {
        let name = source
            .file_name()
            .ok_or_else(|| "Tên log ứng dụng không hợp lệ.".to_string())?;
        let metadata = fs::metadata(&source)
            .map_err(|error| format!("Không thể đọc metadata log ứng dụng: {error}"))?;
        if metadata.len() > 10 * 1024 * 1024 {
            continue;
        }
        let raw = fs::read_to_string(&source)
            .map_err(|error| format!("Không thể đọc log ứng dụng: {error}"))?;
        let scrubbed = sanitize_log(&raw);
        write_new(&destination.join(name), scrubbed.as_bytes())?;
    }
    Ok(())
}

fn sanitize_log(raw: &str) -> String {
    let mut output = String::new();
    for line in raw.lines() {
        let mut parts = line.splitn(3, ' ');
        let timestamp = parts.next().unwrap_or("");
        let level = parts.next().unwrap_or("");
        let message = parts.next().unwrap_or("");
        if message.starts_with("Application startup; version=") {
            output.push_str(timestamp);
            output.push(' ');
            output.push_str(level);
            output.push(' ');
            output.push_str(message);
        } else {
            output.push_str(timestamp);
            output.push(' ');
            output.push_str(level);
            output.push_str(" [message redacted]");
        }
        output.push('\n');
    }
    output
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Không thể ghi diagnostics: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Không thể lưu diagnostics: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_diagnostics_structure_never_contains_secret_fields() {
        let health = RuntimeHealth {
            app_version: "0.1.0".into(),
            build_commit: "unknown".into(),
            os: "windows".into(),
            architecture: "x86_64".into(),
            project_storage_available: true,
            ffmpeg_available: true,
            ffmpeg_version: Some("ffmpeg 8".into()),
            ffprobe_available: true,
            worker_available: true,
            worker_mode: "bundled".into(),
            worker_version: Some("0.1.0".into()),
            cpu_threads: 8,
        };
        let serialized = serde_json::to_string(&SanitizedDiagnostics {
            app: &health,
            settings: json!({"translationProvider":"gemini", "asrModel":"small"}),
            logs_directory: "logs".into(),
            includes_user_media: false,
            includes_transcripts: false,
            includes_credentials: false,
        })
        .unwrap()
        .to_ascii_lowercase();
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("authorization"));
        assert!(serialized.contains("includestranscripts\":false"));
        assert!(serialized.contains("includescredentials\":false"));
    }

    #[test]
    fn diagnostics_writes_only_into_a_selected_existing_directory() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("not-a-directory");
        fs::write(&file, b"x").unwrap();
        let metadata = fs::symlink_metadata(&file).unwrap();
        assert!(!metadata.is_dir());
    }

    #[test]
    fn support_log_redaction_removes_secrets_paths_and_user_messages() {
        let input = "2026-09-23T00:00:00Z INFO Application startup; version=0.1.0\n2026-09-23T00:00:01Z ERROR Transcript at C:\\Users\\Admin\\secret.mp4 key=AIza-example\n";
        let safe = sanitize_log(input);
        assert!(safe.contains("Application startup; version=0.1.0"));
        assert!(safe.contains("[message redacted]"));
        assert!(!safe.contains("secret.mp4"));
        assert!(!safe.contains("AIza-example"));
        assert!(!safe.contains("C:\\Users"));
    }
}
