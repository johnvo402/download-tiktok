use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const MAX_LOG_FILES: usize = 10;
static LOGGER: OnceLock<AppLogger> = OnceLock::new();

struct AppLogger {
    path: PathBuf,
    file: Mutex<File>,
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}\n",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            record.level(),
            record.args()
        );
        if let Ok(mut file) = self.file.lock() {
            let current = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            if current.saturating_add(line.len() as u64) > MAX_LOG_BYTES {
                if let Ok(replacement) = rotate_and_open(&self.path) {
                    *file = replacement;
                }
            }
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

pub fn initialize(log_directory: &Path) -> Result<(), String> {
    fs::create_dir_all(log_directory)
        .map_err(|error| format!("Không thể tạo thư mục nhật ký: {error}"))?;
    let path = log_directory.join("app.log");
    let file =
        rotate_and_open(&path).map_err(|error| format!("Không thể mở file nhật ký: {error}"))?;
    let logger = AppLogger {
        path,
        file: Mutex::new(file),
    };
    let _ = LOGGER.set(logger);
    if let Some(logger) = LOGGER.get() {
        log::set_logger(logger)
            .map_err(|_| "Logger ứng dụng đã được khởi tạo trước đó.".to_string())?;
        log::set_max_level(LevelFilter::Info);
    }
    Ok(())
}

fn rotate_and_open(path: &Path) -> std::io::Result<File> {
    if path
        .metadata()
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
        .unwrap_or(false)
    {
        for index in (1..MAX_LOG_FILES).rev() {
            let source = rotated_path(path, index - 1);
            let destination = rotated_path(path, index);
            if source.exists() {
                let _ = fs::remove_file(&destination);
                let _ = fs::rename(source, destination);
            }
        }
    }
    OpenOptions::new().create(true).append(true).open(path)
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        path.to_path_buf()
    } else {
        path.with_file_name(format!("app.{index}.log"))
    }
}

pub fn logs_directory(app_data: &Path) -> PathBuf {
    app_data.join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_rotation_keeps_bounded_history() {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("app.log");
        fs::write(&log, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        let _file = rotate_and_open(&log).unwrap();
        assert!(rotated_path(&log, 1).is_file());
        assert_eq!(
            fs::metadata(rotated_path(&log, 1)).unwrap().len(),
            MAX_LOG_BYTES
        );
    }
}
