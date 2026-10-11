//! Small, durable JSON documents. Backups are validated before recovery; newer
//! schemas are never downgraded, even when their payload cannot be deserialized.
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::{self, Metadata};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static IO_LOCK: Mutex<()> = Mutex::new(());
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const BACKUPS: usize = 3;
static NOTICES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
pub fn take_notices() -> Vec<String> {
    NOTICES
        .lock()
        .map(|mut notices| notices.drain(..).collect())
        .unwrap_or_default()
}

fn ordinary_file(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_file() && !metadata.file_type().is_symlink()
}

fn read_regular(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("Không thể đọc dữ liệu: {e}"))?;
    if !ordinary_file(&metadata) || metadata.len() > MAX_DOCUMENT_BYTES {
        return Err("Tệp dữ liệu không an toàn hoặc vượt dung lượng cho phép.".into());
    }
    fs::read(path).map_err(|e| format!("Không thể đọc dữ liệu: {e}"))
}

pub fn backup_path(path: &Path, index: usize) -> PathBuf {
    path.with_file_name(format!(
        "{}.bak{index}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !ordinary_file(&metadata) {
            return Err("Từ chối ghi qua liên kết hoặc tệp không hợp lệ.".into());
        }
    }
    let parent = path.parent().ok_or("Thư mục dữ liệu không hợp lệ.")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Không thể tạo tệp lưu tạm: {e}"))?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| {
            format!("Không thể lưu dữ liệu (hãy kiểm tra dung lượng và quyền ghi): {e}")
        })?;
    temp.persist(path)
        .map_err(|e| format!("Không thể cập nhật dữ liệu: {}", e.error))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Callers validate their new document first. Never replaces a valid backup
/// with corrupt JSON, and never truncates the old main document on write error.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let _guard = IO_LOCK
        .lock()
        .map_err(|_| "Không thể khóa dữ liệu để lưu.")?;
    serde_json::from_slice::<Value>(bytes)
        .map_err(|_| "Dữ liệu cần lưu không phải JSON hợp lệ.")?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err("Dữ liệu vượt dung lượng cho phép.".into());
    }
    if let Ok(old) = read_regular(path) {
        if serde_json::from_slice::<Value>(&old).is_ok() && old != bytes {
            for index in (2..=BACKUPS).rev() {
                let source = backup_path(path, index - 1);
                if let Ok(previous) = read_regular(&source) {
                    replace(&backup_path(path, index), &previous)?;
                }
            }
            replace(&backup_path(path, 1), &old)?;
        }
    }
    replace(path, bytes)
}

/// Non-JSON sidecars (SRT/ASS) retain atomic replacement without JSON recovery.
pub fn write_blob(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let _guard = IO_LOCK
        .lock()
        .map_err(|_| "Không thể khóa dữ liệu để lưu.")?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err("Dữ liệu vượt dung lượng cho phép.".into());
    }
    replace(path, bytes)
}

fn newer_schema(bytes: &[u8], maximum: Option<u32>) -> bool {
    maximum.is_some_and(|maximum| {
        serde_json::from_slice::<Value>(bytes)
            .ok()
            .and_then(|v| v.get("schemaVersion").and_then(Value::as_u64))
            .is_some_and(|version| version > u64::from(maximum))
    })
}

pub fn read<T>(
    path: &Path,
    maximum_schema: Option<u32>,
    validate: impl Fn(&[u8]) -> Result<T, String>,
) -> Result<T, String> {
    read_with_schema_guard(path, maximum_schema, |_| false, validate)
}

/// Additional versioned subdocuments must also fail closed on future schemas.
pub fn read_with_schema_guard<T>(
    path: &Path,
    maximum_schema: Option<u32>,
    unsupported_schema: impl Fn(&[u8]) -> bool,
    validate: impl Fn(&[u8]) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = IO_LOCK
        .lock()
        .map_err(|_| "Không thể khóa dữ liệu để đọc.")?;
    // A symlink, access denial or oversized document must not trigger replacement.
    match fs::symlink_metadata(path) {
        Ok(metadata) if !ordinary_file(&metadata) || metadata.len() > MAX_DOCUMENT_BYTES => {
            return Err("Tệp dữ liệu không an toàn hoặc vượt dung lượng cho phép.".into())
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("Không thể đọc dữ liệu: {e}"))
        }
        _ => {}
    }
    let original = if path.exists() {
        Some(read_regular(path)?)
    } else {
        None
    };
    if original
        .as_deref()
        .is_some_and(|bytes| newer_schema(bytes, maximum_schema) || unsupported_schema(bytes))
    {
        return Err("Dữ liệu được tạo bằng phiên bản ứng dụng mới hơn. Hãy cập nhật JC AI để mở; tệp được giữ nguyên.".into());
    }
    let error = if let Some(bytes) = &original {
        match validate(bytes) {
            Ok(value) => return Ok(value),
            Err(e) => e,
        }
    } else {
        "Không tìm thấy tệp dữ liệu hoặc bản sao lưu hợp lệ.".into()
    };
    for index in 1..=BACKUPS {
        let Ok(bytes) = read_regular(&backup_path(path, index)) else {
            continue;
        };
        if newer_schema(&bytes, maximum_schema) || unsupported_schema(&bytes) {
            continue;
        }
        let Ok(value) = validate(&bytes) else {
            continue;
        };
        if let Some(original) = &original {
            preserve_corrupt(path, original)?;
        }
        replace(path, &bytes)?;
        if let Ok(mut notices) = NOTICES.lock() {
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some("project.json") => "thông tin dự án",
                Some("transcript.json") => "transcript",
                Some("settings.json") => "cài đặt",
                Some("jobs.json") => "lịch sử tác vụ",
                _ => "dữ liệu ứng dụng",
            };
            notices.push_back(format!("Đã khôi phục {name} từ bản sao lưu hợp lệ gần nhất. Bản lỗi được giữ riêng; hãy kiểm tra những thay đổi cuối cùng."));
            while notices.len() > 20 {
                notices.pop_front();
            }
        }
        log::warn!(
            "Recovered {} from validated backup {index}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        return Ok(value);
    }
    Err(error)
}

pub fn preserve_corrupt(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let backup = path.with_file_name(format!(
        "{}.corrupt-{}.bak",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(|e| format!("Không thể giữ bản dữ liệu bị lỗi: {e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Không thể sao lưu dữ liệu bị lỗi: {e}"))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(bytes: &[u8]) -> Result<Value, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
    #[test]
    fn recovers_latest_valid_backup_and_preserves_corrupt_main() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dữ liệu.json");
        write(&path, br#"{"schemaVersion":1,"value":1}"#).unwrap();
        write(&path, br#"{"schemaVersion":1,"value":2}"#).unwrap();
        write(&path, br#"{"schemaVersion":1,"value":3}"#).unwrap();
        fs::write(&path, b"broken").unwrap();
        fs::write(backup_path(&path, 1), b"also broken").unwrap();
        assert_eq!(read(&path, Some(1), parse).unwrap()["value"], 1);
        assert!(fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("corrupt-")));
    }
    #[test]
    fn future_schema_never_recovers_or_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        write(&path, br#"{"schemaVersion":1}"#).unwrap();
        write(&path, br#"{"schemaVersion":99,"unknown":true}"#).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(read(&path, Some(1), parse).is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }
    #[test]
    fn semantic_validation_skips_wrong_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.json");
        write(&path, br#"{"projectId":"correct"}"#).unwrap();
        write(&path, br#"{"projectId":"wrong"}"#).unwrap();
        fs::write(&path, b"bad").unwrap();
        let recovered = read(&path, None, |bytes| {
            let v = parse(bytes)?;
            if v["projectId"] != "correct" {
                Err("wrong project".into())
            } else {
                Ok(v)
            }
        })
        .unwrap();
        assert_eq!(recovered["projectId"], "correct");
    }
    #[test]
    fn failed_write_keeps_existing_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        write(&path, b"{\"value\":1}").unwrap();
        fs::create_dir(backup_path(&path, 1)).unwrap();
        assert!(write(&path, b"{\"value\":2}").is_err());
        assert_eq!(parse(&fs::read(path).unwrap()).unwrap()["value"], 1);
    }
    #[test]
    fn missing_main_recovers_but_no_backups_does_not_create_empty_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        assert!(read(&path, None, parse).is_err());
        fs::write(backup_path(&path, 1), b"{\"value\":1}").unwrap();
        assert!(read(&path, None, parse).is_ok());
        assert!(path.exists());
    }
}
