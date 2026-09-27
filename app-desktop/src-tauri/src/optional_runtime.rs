use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

static INSTALL_LOCK: Mutex<()> = Mutex::new(());
const FEATURES: [&str; 3] = ["whisperx", "bandit", "acceleration"];
const RUNTIME_VERSION: u32 = 3;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeMarker {
    runtime_version: u32,
    profile: String,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub profile: Option<String>,
    pub installed_features: Vec<String>,
    pub nvidia_detected: bool,
}

fn root() -> Result<PathBuf, String> {
    Ok(crate::runtime_paths::app_data_dir()
        .ok_or("Không thể xác định thư mục ứng dụng.")?
        .join("runtimes")
        .join("advanced"))
}

fn ensure_runtime_root() -> Result<PathBuf, String> {
    let root = root()?;
    let parent = root.parent().ok_or("Thư mục ứng dụng không hợp lệ.")?;
    for directory in [parent, root.as_path()] {
        if !directory.exists() {
            fs::create_dir(directory).map_err(|error| format!("Không thể tạo thư mục thành phần bổ sung: {error}"))?;
        }
        let metadata = fs::symlink_metadata(directory)
            .map_err(|error| format!("Không thể kiểm tra thư mục thành phần bổ sung: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err("Thư mục thành phần bổ sung không an toàn; đã dừng cài đặt.".into());
        }
    }
    Ok(root)
}

fn python_path(directory: &Path) -> PathBuf {
    directory.join("Scripts").join("python.exe")
}

fn read_marker() -> Option<RuntimeMarker> {
    let root = root().ok()?;
    let mut marker: RuntimeMarker = serde_json::from_slice(&fs::read(root.join("current.json")).ok()?).ok()?;
    if !matches!(marker.profile.as_str(), "cpu" | "cuda")
        || !(1..=RUNTIME_VERSION).contains(&marker.runtime_version)
    {
        return None;
    }
    // Version 1 installed all optional packages together. Preserve that feature
    // set when migrating to the current runtime so users do not silently lose
    // capabilities during the repair.
    if marker.runtime_version == 1 {
        marker.features = FEATURES.iter()
            .filter(|feature| **feature != "acceleration" || marker.profile == "cuda")
            .map(|feature| (*feature).to_string())
            .collect();
    }
    if marker.features.iter().any(|feature| !FEATURES.contains(&feature.as_str())) {
        return None;
    }
    let current = root.join("current");
    let metadata = fs::symlink_metadata(&current).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return None;
    }
    let python = python_path(&current);
    let metadata = fs::symlink_metadata(&python).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return None;
    }
    crate::runtime_paths::optional_worker_source()?;
    Some(marker)
}

fn installed_marker() -> Option<RuntimeMarker> {
    let marker = read_marker()?;
    (marker.runtime_version == RUNTIME_VERSION).then_some(marker)
}

pub fn feature_installed(feature: &str) -> bool {
    installed_marker().is_some_and(|marker| marker.features.iter().any(|installed| installed == feature))
}

pub fn active_runtime() -> Option<(PathBuf, PathBuf, String)> {
    let marker = installed_marker()?;
    let source = crate::runtime_paths::optional_worker_source()?;
    Some((python_path(&root().ok()?.join("current")), source, marker.profile))
}

pub fn status() -> RuntimeStatus {
    let marker = installed_marker();
    RuntimeStatus {
        profile: marker.as_ref().map(|value| value.profile.clone()),
        installed_features: marker.map(|value| value.features).unwrap_or_default(),
        nvidia_detected: nvidia_detected(),
    }
}

fn nvidia_detected() -> bool {
    let mut locations = vec![PathBuf::from("nvidia-smi")];
    if cfg!(windows) {
        locations.push(PathBuf::from(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"));
        locations.push(PathBuf::from(r"C:\Windows\System32\nvidia-smi.exe"));
    }
    locations.into_iter().any(|program| {
        let mut command = Command::new(program);
        hide_console(&mut command);
        command.args(["--query-gpu=name", "--format=csv,noheader"])
            .stdout(Stdio::piped()).stderr(Stdio::null()).output()
            .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
    })
}

fn emit(app: &AppHandle, message: &str) {
    let _ = app.emit("optional-runtime-progress", json!({ "message": message }));
}

fn run_uv(app: &AppHandle, uv: &Path, root: &Path, args: &[&str], stage: &str) -> Result<(), String> {
    emit(app, stage);
    let mut command = Command::new(uv);
    hide_console(&mut command);
    let output = command.args(args)
        .env("UV_CACHE_DIR", root.join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", root.join("python"))
        .env("UV_NO_PROGRESS", "1")
        .env("UV_LINK_MODE", "copy")
        .stdout(Stdio::null()).stderr(Stdio::piped()).output()
        .map_err(|error| format!("Không thể khởi động trình cài: {error}"))?;
    if output.status.success() { return Ok(()); }
    let detail = String::from_utf8_lossy(&output.stderr);
    log::error!("Optional AI install failed in {stage}: {}", detail.chars().rev().take(3000).collect::<String>().chars().rev().collect::<String>());
    Err(format!("Không cài được ở bước ‘{stage}’. Kiểm tra kết nối và dung lượng ổ đĩa, rồi thử lại."))
}

fn safe_remove(directory: &Path, parent: &Path) -> Result<(), String> {
    if directory.parent() != Some(parent) { return Err("Thư mục cài đặt không hợp lệ.".into()); }
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Không thể kiểm tra thành phần cũ: {error}")),
        Ok(metadata) if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_dir() => Err("Thư mục thành phần cũ không hợp lệ.".into()),
        Ok(_) => { ensure_plain_tree(directory)?; fs::remove_dir_all(directory).map_err(|error| format!("Không thể dọn thành phần cũ: {error}")) }
    }
}

fn ensure_plain_tree(directory: &Path) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| format!("Không thể kiểm tra thư mục cài đặt: {error}"))? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err("Thư mục cài đặt chứa liên kết không an toàn; không thể tự động dọn.".into());
        }
        if metadata.is_dir() { ensure_plain_tree(&path)?; }
        else if !metadata.is_file() { return Err("Thư mục cài đặt chứa tệp không hợp lệ.".into()); }
    }
    Ok(())
}

fn rollback_replacement(root: &Path) {
    let _ = safe_remove(&root.join("current"), root);
    if root.join("previous").is_dir() {
        let _ = fs::rename(root.join("previous"), root.join("current"));
    }
}

fn requirement_name(feature: &str) -> &'static str {
    match feature {
        "whisperx" => "requirements-optional-whisperx.in",
        "bandit" => "requirements-optional-bandit.in",
        _ => "requirements-optional-acceleration.in",
    }
}

fn install_feature(app: &AppHandle, uv: &Path, root: &Path, source: &Path, python: &Path, feature: &str, profile: &str) -> Result<(), String> {
    let python = python.to_str().ok_or("Đường dẫn bộ xử lý không hợp lệ.")?;
    let requirements = source.join(requirement_name(feature));
    let requirements = requirements.to_str().ok_or("Đường dẫn gói cài đặt không hợp lệ.")?;
    if !Path::new(requirements).is_file() { return Err("Ứng dụng thiếu danh sách thành phần. Hãy cập nhật hoặc cài lại JC AI.".into()); }
    let torch_index = if profile == "cuda" { "https://download.pytorch.org/whl/cu126" } else { "https://download.pytorch.org/whl/cpu" };
    let torch_label = if profile == "cuda" { "Cài bộ xử lý NVIDIA dùng chung" } else { "Cài bộ xử lý CPU dùng chung" };
    // uv checks the shared environment first. Already-installed torch and its
    // dependencies are reused by WhisperX and BandIt instead of downloaded twice.
    let mut packages = vec!["torch==2.8.0", "torchaudio==2.8.0"];
    if feature == "whisperx" { packages.push("torchvision==0.23.0"); }
    let mut args = vec!["pip", "install", "--no-progress", "--python", python, "--index-url", torch_index];
    args.extend(packages);
    run_uv(app, uv, root, &args, torch_label)?;
    if feature == "whisperx" {
        run_uv(app, uv, root, &["pip", "install", "--no-progress", "--python", python, "--index-url", "https://download.pytorch.org/whl/cpu", "torchcodec==0.7.0"], "Cài bộ đọc âm thanh")?;
    }
    run_uv(app, uv, root, &["pip", "install", "--no-progress", "--python", python, "-r", requirements], match feature {
        "whisperx" => "Cài WhisperX",
        "bandit" => "Cài tách thoại gốc",
        _ => "Cài tăng tốc NVIDIA",
    })?;
    if feature == "acceleration" {
        run_uv(app, uv, root, &["pip", "uninstall", "--python", python, "onnxruntime"], "Chuẩn bị tăng tốc đọc chữ")?;
        run_uv(app, uv, root, &["pip", "install", "--no-progress", "--python", python, "onnxruntime-gpu==1.30.0"], "Cài tăng tốc đọc chữ")?;
    }
    Ok(())
}

fn verify(app: &AppHandle, python: &Path, source: &Path, features: &[String], profile: &str) -> Result<(), String> {
    emit(app, "Kiểm tra các tính năng đã cài");
    let mut script = String::from(
        "from packaging.version import Version\nimport torch\nassert Version(torch.__version__.split('+')[0]).release[:2] == (2, 8)\n",
    );
    if profile == "cuda" && cfg!(windows) {
        script.push_str("import os, pathlib; gpu_dlls = os.add_dll_directory(str(pathlib.Path(torch.__file__).parent / 'lib'))\n");
    }
    if features.iter().any(|feature| feature == "whisperx") {
        script.push_str(
            "import faster_whisper, whisperx, transformers, huggingface_hub\n\
            assert Version('4.57') <= Version(transformers.__version__) < Version('5')\n\
            assert Version(huggingface_hub.__version__) == Version('0.36.2')\n",
        );
    }
    if features.iter().any(|feature| feature == "bandit") {
        script.push_str("import bandit_infer, soundfile\n");
    }
    if features.iter().any(|feature| feature == "acceleration") {
        script.push_str(
            "import faster_whisper, rapidocr, vieneu, onnxruntime, huggingface_hub\n\
            assert Version(huggingface_hub.__version__) == Version('0.36.2')\n\
            assert 'CUDAExecutionProvider' in onnxruntime.get_available_providers()\n",
        );
    }
    if profile == "cuda" {
        script.push_str("assert torch.cuda.is_available()\n");
        if features.iter().any(|feature| feature == "whisperx" || feature == "acceleration") {
            script.push_str("import ctranslate2; assert ctranslate2.get_cuda_device_count() > 0\n");
        }
    }
    let mut command = Command::new(python);
    hide_console(&mut command);
    let output = command.arg("-c").arg(&script).stdout(Stdio::null()).stderr(Stdio::piped()).output()
        .map_err(|error| format!("Không thể kiểm tra thành phần vừa cài: {error}"))?;
    if !output.status.success() {
        log::error!("Optional AI verification failed: {}", String::from_utf8_lossy(&output.stderr));
        return Err("Thành phần vừa tải chưa hoạt động đúng trên máy này. Hãy kiểm tra kết nối và thử lại.".into());
    }
    let mut command = Command::new(python);
    hide_console(&mut command);
    let output = command.arg("-c").arg("import worker.__main__")
        .current_dir(source).env("PYTHONPATH", source)
        .env("PYTHONUTF8", "1")
        .stdout(Stdio::null()).stderr(Stdio::piped()).output()
        .map_err(|error| format!("Không thể kiểm tra bộ xử lý AI: {error}"))?;
    if !output.status.success() {
        log::error!("Optional AI worker startup failed: {}", String::from_utf8_lossy(&output.stderr));
        return Err("Bộ xử lý AI không khởi động được sau khi cài. Thành phần đang dùng được giữ nguyên.".into());
    }
    Ok(())
}

pub fn install(app: &AppHandle, feature: &str, profile: &str) -> Result<RuntimeStatus, String> {
    if !FEATURES.contains(&feature) || !matches!(profile, "cpu" | "cuda") || (feature == "acceleration" && profile != "cuda") {
        return Err("Lựa chọn cài đặt không hợp lệ.".into());
    }
    let _guard = INSTALL_LOCK.lock().map_err(|_| "Không thể khóa thao tác cài đặt.".to_string())?;
    if profile == "cuda" && !nvidia_detected() {
        return Err("Windows chưa nhận diện GPU NVIDIA. Kiểm tra driver và kết nối GPU trước khi cài tăng tốc.".into());
    }
    let uv = crate::runtime_paths::optional_installer().ok_or("Bộ cài thành phần bổ sung chưa có trong ứng dụng. Hãy cập nhật JC AI.")?;
    let source = crate::runtime_paths::optional_worker_source().ok_or("Ứng dụng thiếu bộ xử lý AI. Hãy cập nhật JC AI.")?;
    let root = ensure_runtime_root()?;
    for name in ["cache", "python"] {
        let directory = root.join(name);
        fs::create_dir_all(&directory).map_err(|error| format!("Không thể tạo thư mục cài đặt: {error}"))?;
        let metadata = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err("Thư mục cài đặt không an toàn.".into());
        }
    }
    let old = read_marker();
    if profile == "cpu" && old.as_ref().is_some_and(|marker| marker.features.iter().any(|item| item == "acceleration")) {
        return Err("Máy đang dùng thành phần tăng tốc NVIDIA. Hãy chọn NVIDIA để dùng chung bộ xử lý đã cài.".into());
    }

    let already_installed = old.as_ref().is_some_and(|marker| {
        marker.runtime_version == RUNTIME_VERSION
            && marker.profile == profile
            && marker.features.iter().any(|item| item == feature)
    });
    let mut repair_current = false;
    if already_installed {
        let current_python = python_path(&root.join("current"));
        if verify(app, &current_python, &source, &old.as_ref().unwrap().features, profile).is_ok() {
            return Ok(status());
        }
        emit(app, "Phát hiện runtime AI không tương thích; đang sửa chữa");
        repair_current = true;
    }

    let mut features = old.as_ref().map(|marker| marker.features.clone()).unwrap_or_default();
    if !features.iter().any(|item| item == feature) { features.push(feature.to_string()); }
    // Schema/runtime dependency changes are repaired atomically in staging.
    // Same-profile additions can still reuse the current environment.
    let replace = old.as_ref().is_none_or(|marker| {
        marker.profile != profile || marker.runtime_version != RUNTIME_VERSION
    }) || repair_current;
    let destination = if replace { root.join("staging") } else { root.join("current") };
    if replace {
        safe_remove(&destination, &root)?;
        run_uv(app, &uv, &root, &["venv", "--no-project", "--managed-python", "--python", "3.12", "--no-progress", destination.to_str().ok_or("Đường dẫn cài đặt không hợp lệ.")?], "Chuẩn bị bộ xử lý")?;
    }
    let python = python_path(&destination);
    let install_result = (|| {
        let requested: Vec<&str> = if replace { features.iter().map(String::as_str).collect() } else { vec![feature] };
        for item in requested { install_feature(app, &uv, &root, &source, &python, item, profile)?; }
        run_uv(
            app,
            &uv,
            &root,
            &["pip", "check", "--python", python.to_str().ok_or("Đường dẫn bộ xử lý không hợp lệ.")?],
            "Kiểm tra xung đột dependency",
        )?;
        verify(app, &python, &source, &features, profile)
    })();
    if let Err(error) = install_result {
        if replace { let _ = safe_remove(&destination, &root); }
        return Err(error);
    }
    let marker = RuntimeMarker { runtime_version: RUNTIME_VERSION, profile: profile.into(), features };
    let marker_temp = root.join("current.json.tmp");
    let marker_file = root.join("current.json");
    let marker_previous = root.join("previous.json");
    fs::write(&marker_temp, serde_json::to_vec(&marker).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Không thể lưu trạng thái cài đặt: {error}"))?;
    if replace {
        let current = root.join("current");
        let previous = root.join("previous");
        safe_remove(&previous, &root)?;
        if current.exists() { fs::rename(&current, &previous).map_err(|error| format!("Không thể thay bộ xử lý cũ: {error}"))?; }
        if let Err(error) = fs::rename(&destination, &current) {
            if previous.exists() { let _ = fs::rename(&previous, &current); }
            return Err(format!("Không thể hoàn tất cài đặt: {error}"));
        }
    }
    if marker_previous.is_file() {
        if let Err(error) = fs::remove_file(&marker_previous) {
            if replace { rollback_replacement(&root); }
            return Err(format!("Không thể dọn trạng thái cũ: {error}"));
        }
    }
    if marker_file.is_file() {
        if let Err(error) = fs::rename(&marker_file, &marker_previous) {
            if replace { rollback_replacement(&root); }
            return Err(format!("Không thể cập nhật trạng thái cài đặt: {error}"));
        }
    }
    if let Err(error) = fs::rename(&marker_temp, &marker_file) {
        if marker_previous.is_file() { let _ = fs::rename(&marker_previous, &marker_file); }
        if replace { rollback_replacement(&root); }
        return Err(format!("Không thể hoàn tất trạng thái cài đặt: {error}"));
    }
    let _ = fs::remove_file(&marker_previous);
    if replace { let _ = safe_remove(&root.join("previous"), &root); }
    // Packages already installed in the shared environment are reused. Once
    // installation succeeds, the download cache only duplicates disk usage.
    let _ = safe_remove(&root.join("cache"), &root);
    emit(app, "Đã cài tính năng được chọn");
    Ok(status())
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x0400 != 0;
    }
    #[cfg(not(windows))] { let _ = metadata; false }
}
