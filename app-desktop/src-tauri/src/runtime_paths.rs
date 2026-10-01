use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn initialize(resource_dir: PathBuf, app_data_dir: PathBuf) {
    let _ = RESOURCE_DIR.set(resource_dir);
    let _ = APP_DATA_DIR.set(app_data_dir);
}

pub fn app_data_dir() -> Option<&'static Path> {
    APP_DATA_DIR.get().map(PathBuf::as_path)
}

pub fn optional_worker_source() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        // Use the live source tree in development so worker protocol and bug
        // fixes are immediately shared with CPU and optional CUDA runtimes.
        if let Some(source) = development_worker_dir() {
            return Some(source);
        }
        let staged = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("optional-runtime")
            .join("source");
        if staged.join("worker").join("__main__.py").is_file() {
            return Some(staged);
        }
        return development_worker_dir();
    }
    let root = resource_dir()?.join("optional-runtime").join("source");
    root.join("worker").is_dir().then_some(root)
}

pub fn optional_installer() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Some(PathBuf::from("uv"));
    }
    let path = resource_dir()?.join("optional-runtime").join("uv.exe");
    path.is_file().then_some(path)
}

pub fn resolve_ffmpeg() -> Result<PathBuf, String> {
    resolve_binary("ffmpeg")
}

pub fn resolve_ffprobe() -> Result<PathBuf, String> {
    resolve_binary("ffprobe")
}

fn resolve_binary(name: &str) -> Result<PathBuf, String> {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    if let Some(resource_dir) = resource_dir() {
        let bundled = resource_dir.join("ffmpeg").join(&file_name);
        if bundled.is_file() {
            return Ok(bundled);
        }
    }

    if cfg!(debug_assertions) {
        if let Some(directory) = std::env::var_os("LOCALIZATION_FFMPEG_DIR") {
            let candidate = PathBuf::from(directory).join(&file_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        return Ok(PathBuf::from(name));
    }

    Err(format!(
        "Thiếu thành phần ứng dụng `{file_name}`. Hãy sửa chữa hoặc cài lại JC AI."
    ))
}

pub fn ffmpeg_command() -> Result<Command, String> {
    command(resolve_ffmpeg()?)
}

pub fn ffprobe_command() -> Result<Command, String> {
    command(resolve_ffprobe()?)
}

fn command(executable: PathBuf) -> Result<Command, String> {
    let mut command = Command::new(executable);
    hide_console(&mut command);
    Ok(command)
}

pub fn packaged_worker_dir() -> Option<PathBuf> {
    let root = resource_dir()?.join("ai-worker");
    let executable = root.join(if cfg!(windows) {
        "ai-worker.exe"
    } else {
        "ai-worker"
    });
    executable.is_file().then_some(root)
}

pub fn development_worker_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("python");
    root.join("worker").is_dir().then_some(root)
}

pub fn worker_dir() -> Result<PathBuf, String> {
    let worker = if cfg!(debug_assertions) {
        development_worker_dir().or_else(packaged_worker_dir)
    } else {
        packaged_worker_dir()
    };
    worker.ok_or_else(|| "Thiếu thành phần xử lý AI. Hãy cập nhật hoặc cài lại JC AI.".to_string())
}

pub fn ai_worker_dir() -> Result<PathBuf, String> {
    ai_worker_dir_for("acceleration")
}

pub fn ai_worker_dir_for(feature: &str) -> Result<PathBuf, String> {
    if crate::optional_runtime::feature_installed(feature) {
        let (_, source, _) = crate::optional_runtime::active_runtime()
            .ok_or("Thành phần AI bổ sung chưa sẵn sàng.")?;
        return Ok(source);
    }
    worker_dir()
}

pub fn ocr_worker_dir() -> Result<PathBuf, String> {
    if crate::optional_runtime::feature_installed("ocr_openvino") {
        return ai_worker_dir_for("ocr_openvino");
    }
    ai_worker_dir()
}

pub fn worker_command(worker_root: &Path) -> Command {
    worker_command_scoped(worker_root, false)
}

pub fn ocr_worker_command(worker_root: &Path) -> Command {
    worker_command_scoped(worker_root, true)
}

fn worker_command_scoped(worker_root: &Path, ocr: bool) -> Command {
    if let Some((python, source, profile)) = crate::optional_runtime::active_runtime() {
        let has_other_features = ["acceleration", "whisperx", "bandit", "sttn"]
            .iter()
            .any(|feature| crate::optional_runtime::feature_installed(feature));
        // In development the source path is shared. Installing only Intel OCR
        // must not redirect speech/TTS to this deliberately lightweight venv.
        if worker_root == source && (ocr || has_other_features) {
            let mut command = Command::new(python);
            command
                .arg("-m")
                .arg("worker")
                .current_dir(worker_root)
                .env("PYTHONPATH", worker_root)
                .env("PYTHONUNBUFFERED", "1")
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("PYTHONUTF8", "1")
                .env("PYTHONIOENCODING", "utf-8:backslashreplace");
            if profile == "cuda" {
                command.env("JC_AI_GPU_RUNTIME", "1");
            }
            hide_console(&mut command);
            return command;
        }
    }
    // Development settings install Python extras into the selected Python
    // environment. Run that same interpreter for the source worker so newly
    // installed OCR/WhisperX packages are visible immediately.
    if cfg!(debug_assertions) {
        if let Ok(mut command) = development_python_command() {
            command
                .arg("-m")
                .arg("worker")
                .current_dir(worker_root)
                .env("PYTHONPATH", worker_root)
                .env("PYTHONUNBUFFERED", "1")
                .env("PYTHONUTF8", "1")
                .env("PYTHONIOENCODING", "utf-8:backslashreplace");
            return command;
        }
    }

    let executable = worker_root.join(if cfg!(windows) {
        "ai-worker.exe"
    } else {
        "ai-worker"
    });
    if executable.is_file() {
        let mut command = Command::new(executable);
        command.current_dir(worker_root);
        hide_console(&mut command);
        return command;
    }

    let repository_root = worker_root.parent().unwrap_or(worker_root);
    let local_python = repository_root
        .join(".venv")
        .join("Scripts")
        .join("python.exe");
    let executable = std::env::var_os("LOCALIZATION_PYTHON")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| local_python.is_file().then_some(local_python))
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "python" } else { "python3" }));
    let mut command = Command::new(executable);
    command
        .arg("-m")
        .arg("worker")
        .current_dir(worker_root)
        .env("PYTHONPATH", worker_root)
        .env("PYTHONUNBUFFERED", "1")
        // Tauri sends JSONL as UTF-8 bytes. Force Python's standard streams
        // to use the same encoding on Windows, where the active code page can
        // otherwise corrupt Vietnamese voice names before the worker parses JSON.
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8:backslashreplace");
    hide_console(&mut command);
    command
}

pub fn development_python_command() -> Result<Command, String> {
    if !cfg!(debug_assertions) {
        return Err(
            "Python package cài đặt trong ứng dụng chỉ khả dụng ở môi trường development.".into(),
        );
    }
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let local_python = repository_root
        .join(".venv")
        .join("Scripts")
        .join("python.exe");
    let executable = std::env::var_os("LOCALIZATION_PYTHON")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| local_python.is_file().then_some(local_python))
        .ok_or_else(|| "Không tìm thấy Python environment của dự án. Hãy tạo `.venv` hoặc đặt LOCALIZATION_PYTHON rồi thử lại.".to_string())?;
    let mut command = Command::new(executable);
    hide_console(&mut command);
    Ok(command)
}

pub fn binary_available(path: Result<PathBuf, String>) -> bool {
    path.ok().is_some_and(|path| {
        command(path)
            .ok()
            .map(|mut command| {
                command
                    .arg("-version")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output()
                    .is_ok_and(|output| output.status.success())
            })
            .unwrap_or(false)
    })
}

pub fn program_version(path: &Path) -> Option<String> {
    let mut command = command(path.to_path_buf()).ok()?;
    let output = command
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!line.is_empty()).then_some(line)
}

fn resource_dir() -> Option<&'static Path> {
    RESOURCE_DIR.get().map(PathBuf::as_path)
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_resolution_requires_bundled_programs() {
        let missing = PathBuf::from("ffmpeg-does-not-exist");
        assert!(!binary_available(Ok(missing)));
    }

    #[test]
    fn worker_directory_is_only_selected_when_an_executable_exists() {
        let temp = tempfile::tempdir().unwrap();
        let resources = temp.path().join("resources");
        let worker = resources.join("ai-worker");
        std::fs::create_dir_all(&worker).unwrap();
        assert!(!worker.join("ai-worker.exe").is_file());
        std::fs::write(worker.join("ai-worker.exe"), b"test binary").unwrap();
        assert!(worker.join("ai-worker.exe").is_file());
    }
}
