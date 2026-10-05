use std::time::Duration;
trait BoundedOutput { fn pipe_output_timeout(&mut self) -> std::io::Result<std::process::Output>; }
impl BoundedOutput for Command { fn pipe_output_timeout(&mut self) -> std::io::Result<std::process::Output> { crate::worker_manager::output_timeout(self, Duration::from_secs(300)) } }
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

static INSTALL_LOCK: Mutex<()> = Mutex::new(());
const FEATURES: [&str; 6] = [
    "whisperx",
    "bandit",
    "acceleration",
    "sttn",
    "ocr_openvino",
    "whisper_openvino",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputeBackend {
    Cpu,
    Cuda,
    OpenVino,
}

fn backend(profile: &str) -> Option<ComputeBackend> {
    match profile {
        "cpu" => Some(ComputeBackend::Cpu),
        "cuda" => Some(ComputeBackend::Cuda),
        "openvino" => Some(ComputeBackend::OpenVino),
        _ => None,
    }
}

fn valid_install(feature: &str, profile: &str) -> bool {
    crate::runtime_registry::registry().features.get(feature)
        .is_some_and(|f| f.backends.iter().any(|b| b == profile))
}
const RUNTIME_VERSION: u32 = 7;
const CUDA_ONNXRUNTIME_VERSION: &str = "1.26.0";

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
    pub feature_profiles: std::collections::BTreeMap<String, String>,
    pub runtime_version: u32,
    pub health: std::collections::BTreeMap<String, RuntimeHealthState>,
}

#[derive(Clone,Copy,Serialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum RuntimeHealthState { Ready, Missing, Broken, UpdateRequired }
fn quick_feature_health(feature:&str) -> RuntimeHealthState {
    let Ok(root)=feature_root(feature) else {return RuntimeHealthState::Missing;};
    let Some(marker)=read_marker_at(&root) else {
        return if root.join("current.json").exists() {RuntimeHealthState::Broken} else {RuntimeHealthState::Missing};
    };
    if !marker.features.iter().any(|f|f==feature) {return RuntimeHealthState::Missing;}
    if marker.runtime_version!=RUNTIME_VERSION {return RuntimeHealthState::UpdateRequired;}
    if active_runtime_for(feature).is_none() {return RuntimeHealthState::Broken;}
    RuntimeHealthState::Ready
}

fn root() -> Result<PathBuf, String> {
    Ok(crate::runtime_paths::app_data_dir()
        .ok_or("Không thể xác định thư mục ứng dụng.")?
        .join("runtimes")
        .join("advanced"))
}

fn feature_root(feature: &str) -> Result<PathBuf, String> {
    if matches!(feature, "ocr_openvino" | "whisper_openvino") {
        Ok(root()?.with_file_name("intel"))
    } else { root() }
}

fn ensure_runtime_root_at(root: PathBuf) -> Result<PathBuf, String> {
    let parent = root.parent().ok_or("Thư mục ứng dụng không hợp lệ.")?;
    for directory in [parent, root.as_path()] {
        if !directory.exists() {
            fs::create_dir(directory)
                .map_err(|error| format!("Không thể tạo thư mục thành phần bổ sung: {error}"))?;
        }
        let metadata = fs::symlink_metadata(directory)
            .map_err(|error| format!("Không thể kiểm tra thư mục thành phần bổ sung: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err("Thư mục thành phần bổ sung không an toàn; đã dừng cài đặt.".into());
        }
    }
    Ok(root)
}

fn ensure_runtime_root() -> Result<PathBuf, String> { ensure_runtime_root_at(root()?) }

fn installation_file_lock() -> Result<fs::File, String> {
    let root = ensure_runtime_root()?;
    let path = root.join("installation.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.file_type().is_symlink() || is_reparse_point(&meta) {
            return Err("Tệp khóa cài đặt không an toàn.".into());
        }
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path).map_err(|_|"Một phiên JC AI khác đang cài hoặc gỡ thành phần. Hãy chờ hoàn tất.".into())
}

fn python_path(directory: &Path) -> PathBuf {
    directory.join("Scripts").join("python.exe")
}

fn read_marker() -> Option<RuntimeMarker> {
    read_marker_at(&root().ok()?)
}
fn read_marker_at(root: &Path) -> Option<RuntimeMarker> {
    let mut marker: RuntimeMarker =
        serde_json::from_slice(&fs::read(root.join("current.json")).ok()?).ok()?;
    if backend(&marker.profile).is_none()
        || !(1..=RUNTIME_VERSION).contains(&marker.runtime_version)
    {
        return None;
    }
    // Version 1 installed all optional packages together. Preserve that feature
    // set when migrating to the current runtime so users do not silently lose
    // capabilities during the repair.
    if marker.runtime_version == 1 {
        marker.features = ["whisperx", "bandit", "acceleration"]
            .iter()
            .filter(|feature| **feature != "acceleration" || marker.profile == "cuda")
            .map(|feature| (*feature).to_string())
            .collect();
    }
    if marker
        .features
        .iter()
        .any(|feature| !FEATURES.contains(&feature.as_str()))
    {
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
    active_runtime_for(feature).is_some()
}

pub fn active_runtime_for(feature: &str) -> Option<(PathBuf, PathBuf, String)> {
    let dedicated = feature_root(feature).ok()?;
    // Reuse old mixed packs until the user installs an Intel-specific pack.
    // No copies/downloads or destructive migration happen during startup.
    let (directory, marker) = read_marker_at(&dedicated)
        .filter(|m| m.runtime_version == RUNTIME_VERSION && m.features.iter().any(|f| f == feature))
        .map(|m| (dedicated, m))
        .or_else(|| {
            let directory = root().ok()?;
            installed_marker().filter(|m| m.features.iter().any(|f| f == feature)).map(|m| (directory, m))
        })?;
    Some((python_path(&directory.join("current")), crate::runtime_paths::optional_worker_source()?, marker.profile))
}

pub fn status() -> RuntimeStatus {
    let marker = installed_marker();
    let mut features = marker.as_ref().map(|value| value.features.clone()).unwrap_or_default();
    if let Ok(intel) = feature_root("ocr_openvino") {
        if let Some(m) = read_marker_at(&intel).filter(|m| m.runtime_version == RUNTIME_VERSION) {
            features.extend(m.features);
        }
    }
    features.sort(); features.dedup();
    RuntimeStatus {
        profile: marker.as_ref().map(|value| value.profile.clone()),
        installed_features: features,
        nvidia_detected: nvidia_detected(),
        feature_profiles: FEATURES.iter().filter_map(|f| active_runtime_for(f).map(|(_,_,p)| (f.to_string(), p))).collect(),
        runtime_version: RUNTIME_VERSION,
        health: FEATURES.iter().map(|f|(f.to_string(),quick_feature_health(f))).collect(),
    }
}

fn nvidia_detected() -> bool {
    trait NvidiaOutput { fn output_nvidia_timeout(&mut self) -> std::io::Result<std::process::Output>; }
    impl NvidiaOutput for Command { fn output_nvidia_timeout(&mut self) -> std::io::Result<std::process::Output> { crate::worker_manager::output_timeout(self,Duration::from_secs(3)) } }

    let mut locations = vec![PathBuf::from("nvidia-smi")];
    if cfg!(windows) {
        locations.push(PathBuf::from(
            r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
        ));
        locations.push(PathBuf::from(r"C:\Windows\System32\nvidia-smi.exe"));
    }
    locations.into_iter().any(|program| {
        let mut command = Command::new(program);
        hide_console(&mut command);
        command
            .args(["--query-gpu=name", "--format=csv,noheader"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output_nvidia_timeout()
            .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
    })
}

fn emit(app: &AppHandle, message: &str) {
    let _ = app.emit("optional-runtime-progress", json!({ "message": message }));
}

fn verified_wheel(wheels: &Path, manifest_file: &str, prefix: &str) -> Result<PathBuf, String> {
    ensure_plain_tree(wheels)?;
    let metadata = fs::read(wheels.join(manifest_file)).map_err(|_|"Thiếu thông tin thành phần cài đặt.")?;
    let manifest: serde_json::Value = serde_json::from_slice(metadata.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&metadata))
        .map_err(|_|"Thông tin thành phần cài đặt không hợp lệ.")?;
    let name = manifest["name"].as_str().ok_or("Thiếu tên thành phần cài đặt.")?;
    if !name.starts_with(prefix) || !name.ends_with(".whl") || Path::new(name).components().count()!=1 || name.contains(['/', '\\', ':']) {
        return Err("Đường dẫn thành phần cài đặt không hợp lệ.".into());
    }
    use sha2::Digest;
    let file = wheels.join(name);
    let bytes = fs::read(&file).map_err(|_|"Không đọc được thành phần cài đặt.")?;
    let hash = format!("{:x}",sha2::Sha256::digest(bytes));
    if manifest["sha256"].as_str()!=Some(hash.as_str()) { return Err("Thành phần cài đặt bị hỏng. Hãy cài lại JC AI.".into()); }
    Ok(file)
}

fn run_uv(
    app: &AppHandle,
    uv: &Path,
    _root: &Path,
    args: &[&str],
    stage: &str,
) -> Result<(), String> {
    emit(app, stage);
    let mut command = Command::new(uv);
    hide_console(&mut command);
    let shared = ensure_runtime_root()?;
    command.args(args);
    if args.starts_with(&["pip", "install"]) {
        let wheels = crate::runtime_paths::optional_wheelhouse()
            .ok_or("Ứng dụng thiếu thành phần cài đặt. Hãy cập nhật JC AI.")?;
        verified_wheel(&wheels, "manifest.json", "antlr4_python3_runtime-4.9.3-")?;
        command.arg("--find-links").arg(wheels).args(["--only-binary", "antlr4-python3-runtime"]);
        let constraints=_root.join("staging/matrix-constraints.txt");
        if constraints.is_file() {
            let url=reqwest::Url::from_file_path(constraints).map_err(|_|"Đường dẫn phiên bản cài đặt không hợp lệ.")?;
            command.args(["--constraint",url.as_str()]);
        }
    }
    command
        .env("UV_CACHE_DIR", shared.join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", shared.join("python"))
        .env("UV_NO_PROGRESS", "1")
        .env("UV_LINK_MODE", "copy")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        ;
    let output = crate::worker_manager::output_timeout(&mut command, Duration::from_secs(2*3600))
        .map_err(|error| format!("Không thể khởi động trình cài: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    log::error!(
        "Optional AI install failed in {stage}: {}",
        detail
            .chars()
            .rev()
            .take(3000)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
    if detail.contains("No solution found") || detail.contains("Unsatisfiable requirements") {
        return Err("Thành phần mới chưa tương thích với bộ xử lý AI đang dùng. Bản cài hiện tại được giữ nguyên; hãy cập nhật JC AI rồi thử lại.".into());
    }
    Err(format!(
        "Không cài được ở bước ‘{stage}’. Kiểm tra kết nối và dung lượng ổ đĩa, rồi thử lại."
    ))
}

fn verify_onnx_gpu_replacement(python: &Path, source: &Path) -> Result<(), String> {
    let dll_setup = if cfg!(windows) {
        "import torch, os, pathlib\n_gpu_dll_directory = os.add_dll_directory(str(pathlib.Path(torch.__file__).parent / 'lib'))\n"
    } else {
        "import torch\n"
    };
    let script = format!(
        r#"{dll_setup}from importlib.metadata import version
from packaging.version import Version
import onnxruntime as ort
ort.preload_dlls()
assert Version(version('onnxruntime-gpu')) == Version('{CUDA_ONNXRUNTIME_VERSION}')
assert Version(ort.__version__) == Version('{CUDA_ONNXRUNTIME_VERSION}')
assert 'CUDAExecutionProvider' in ort.get_available_providers()
from worker.ocr.service import _cuda_runtime_probe
available, detail = _cuda_runtime_probe()
assert available, detail
"#
    );
    let mut command = Command::new(python);
    hide_console(&mut command);
    let output = command
        .arg("-c")
        .arg(script)
        .current_dir(source)
        .env("PYTHONPATH", source)
        .env("JC_AI_GPU_RUNTIME", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .pipe_output_timeout()
        .map_err(|error| format!("Không thể kiểm tra ONNX Runtime GPU: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        log::error!("ONNX Runtime GPU replacement verification failed: {detail}");
        Err("ONNX Runtime GPU chưa hoạt động đúng.".into())
    }
}

fn is_onnxruntime_gpu_metadata_conflict(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    normalized.starts_with("the package `")
        && normalized.contains("requires `onnxruntime")
        && normalized.contains("but it's not installed")
}

fn check_dependencies(
    app: &AppHandle,
    uv: &Path,
    _root: &Path,
    python: &Path,
    source: &Path,
    features: &[String],
) -> Result<(), String> {
    emit(app, "Kiểm tra xung đột dependency");
    let python_text = python.to_str().ok_or("Đường dẫn bộ xử lý không hợp lệ.")?;
    let mut command = Command::new(uv);
    hide_console(&mut command);
    let output = command
        .args(["pip", "check", "--python", python_text, "--color", "never"])
        .env("UV_CACHE_DIR", self::root()?.join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", self::root()?.join("python"))
        .env("UV_NO_PROGRESS", "1")
        .env("UV_LINK_MODE", "copy")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .pipe_output_timeout()
        .map_err(|error| format!("Không thể kiểm tra dependency: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let details = format!("{stdout}\n{stderr}");
    let incompatibilities: Vec<&str> = details
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("The package `"))
        .collect();

    // onnxruntime-gpu and onnxruntime expose the same Python module but have
    // different distribution names. VieNeu declares the CPU distribution, so
    // metadata reports it missing after the intentional CUDA replacement.
    if features.iter().any(|feature| feature == "acceleration")
        && !incompatibilities.is_empty()
        && incompatibilities
            .iter()
            .all(|line| is_onnxruntime_gpu_metadata_conflict(line))
        && verify_onnx_gpu_replacement(python, source).is_ok()
    {
        log::warn!(
            "Ignoring {} expected onnxruntime metadata conflict(s): {}",
            incompatibilities.len(),
            incompatibilities.join(" | ")
        );
        return Ok(());
    }

    let concise = details
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    log::error!("Optional AI dependency check failed: {concise}");
    Err(if concise.is_empty() {
        "Các package AI đang xung đột dependency.".into()
    } else {
        format!("Các package AI đang xung đột dependency: {concise}")
    })
}
fn safe_remove(directory: &Path, parent: &Path) -> Result<(), String> {
    if directory.parent() != Some(parent) {
        return Err("Thư mục cài đặt không hợp lệ.".into());
    }
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Không thể kiểm tra thành phần cũ: {error}")),
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
                || !metadata.is_dir() =>
        {
            Err("Thư mục thành phần cũ không hợp lệ.".into())
        }
        Ok(_) => {
            ensure_plain_tree(directory)?;
            fs::remove_dir_all(directory)
                .map_err(|error| format!("Không thể dọn thành phần cũ: {error}"))
        }
    }
}

fn ensure_plain_tree(directory: &Path) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Không thể kiểm tra thư mục cài đặt: {error}"))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(
                "Thư mục cài đặt chứa liên kết không an toàn; không thể tự động dọn.".into(),
            );
        }
        if metadata.is_dir() {
            ensure_plain_tree(&path)?;
        } else if !metadata.is_file() {
            return Err("Thư mục cài đặt chứa tệp không hợp lệ.".into());
        }
    }
    Ok(())
}

fn rollback_replacement(root: &Path) {
    let _ = safe_remove(&root.join("current"), root);
    if root.join("previous").is_dir() {
        let _ = rename_runtime(&root.join("previous"), &root.join("current"));
    }
}

fn rename_runtime(source: &Path, destination: &Path) -> std::io::Result<()> {
    // DLL unload and antivirus scanning can briefly keep a verified Windows
    // venv open. Retry only sharing/access errors within a bounded deadline;
    // leave all rollback decisions with the transaction caller.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        match fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if cfg!(windows)
                && matches!(error.raw_os_error(), Some(5 | 32 | 33))
                && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
            Err(error) => return Err(error),
        }
    }
}

fn requirement_name(feature: &str) -> &'static str {
    crate::runtime_registry::registry().features.get(feature)
        .map(|f| f.requirements.as_str()).unwrap_or("invalid-requirements")
}

fn copy_runtime(source: &Path, destination: &Path) -> Result<(), String> {
    ensure_plain_tree(source)?;
    fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let path = entry.path();
            let target = destination.join(entry.file_name());
            if path.is_dir() {
                copy_tree(&path, &target)?;
            } else {
                fs::copy(path, target)?;
            }
        }
        Ok(())
    }
    // Independent copies: pip in staging must never mutate current via hard links.
    copy_tree(source, destination)
        .map_err(|error| format!("Không thể chuẩn bị bản cài dùng chung: {error}"))
}

fn frozen_constraints(python: &Path, destination: &Path) -> Result<PathBuf, String> {
    let mut command = Command::new(python);
    hide_console(&mut command);
    let output = command.args(["-c", "from importlib.metadata import distributions; print(chr(10).join(sorted(d.metadata['Name']+'=='+d.version for d in distributions() if d.metadata['Name'])))"])
        .pipe_output_timeout().map_err(|error| format!("Không thể kiểm tra các gói đang dùng: {error}"))?;
    if !output.status.success() {
        return Err("Không thể bảo vệ các thành phần AI đang dùng.".into());
    }
    let path = destination.join("existing-constraints.txt");
    fs::write(&path, &output.stdout).map_err(|error| error.to_string())?;
    Ok(path)
}

fn cleanup_previous_runtime(root: &Path) {
    // Retain downloaded wheels for later staging installs. UV_LINK_MODE=copy
    // keeps the active runtime independent of this explicitly removable cache.
    if let Err(error) = safe_remove(&root.join("previous"), root) {
        log::warn!("Could not remove previous optional runtime: {error}");
    }
}

fn recover_runtime_at(root: &Path) -> Result<(), String> {
    let current=root.join("current");
    let previous=root.join("previous");
    let marker=root.join("current.json");
    let old_marker=root.join("previous.json");
    if !previous.is_dir() { return Ok(()); }
    ensure_plain_tree(&previous)?;
    if !current.exists() {
        rename_runtime(&previous,&current).map_err(|e|e.to_string())?;
        if !marker.exists() && old_marker.is_file() { fs::rename(&old_marker,&marker).map_err(|e|e.to_string())?; }
    } else {
        let metadata=fs::read(&marker).ok().and_then(|b|serde_json::from_slice::<RuntimeMarker>(&b).ok());
        let manifest=fs::read(current.join("runtime-manifest.json")).ok().and_then(|b|serde_json::from_slice::<serde_json::Value>(&b).ok());
        let committed=metadata.zip(manifest).is_some_and(|(m,s)|
            s["runtimeVersion"]==m.runtime_version && s["backend"]==m.profile && s["features"]==json!(m.features));
        if committed {
            cleanup_previous_runtime(root);
            let _=fs::remove_file(old_marker);
        } else {
            safe_remove(&current,root)?;
            rename_runtime(&previous,&current).map_err(|e|e.to_string())?;
            if old_marker.is_file() {
                if marker.is_file() { fs::remove_file(&marker).map_err(|e|e.to_string())?; }
                fs::rename(old_marker,marker).map_err(|e|e.to_string())?;
            }
        }
    }
    Ok(())
}

pub fn recover_installations() -> Result<(), String> {
    let directory=root()?;
    if !directory.exists() { return Ok(()); }
    let _guard=INSTALL_LOCK.try_lock().map_err(|_|"Đang cài bộ xử lý.")?;
    let _file_lock=installation_file_lock()?;
    recover_runtime_at(&directory)?;
    let intel=feature_root("ocr_openvino")?;
    if intel.exists() { recover_runtime_at(&intel)?; }
    Ok(())
}

fn clear_installation_cache_at(
    root: &Path,
    install_lock: &Mutex<()>,
    clean: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<u64, String> {
    let _guard = install_lock.try_lock().map_err(|_| {
        "Đang cài tính năng AI. Hãy chờ cài xong rồi xóa cache cài đặt.".to_string()
    })?;
    let cache = root.join("cache");
    match fs::symlink_metadata(&cache) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("Không thể kiểm tra cache cài đặt: {error}")),
        Ok(metadata)
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_reparse_point(&metadata) =>
        {
            return Err("Thư mục cache cài đặt không an toàn.".into());
        }
        Ok(_) => ensure_plain_tree(&cache)?,
    }
    let bytes = crate::storage_service::directory_size(&cache);
    clean(&cache)?;
    Ok(bytes.saturating_sub(crate::storage_service::directory_size(&cache)))
}

pub fn clear_installation_cache() -> Result<u64, String> {
    let root = ensure_runtime_root()?;
    clear_installation_cache_at(&root, &INSTALL_LOCK, |cache| {
        let uv = crate::runtime_paths::optional_installer()
            .ok_or("Ứng dụng thiếu bộ dọn cache cài đặt. Hãy cập nhật JC AI.")?;
        // uv owns its cache layout and takes a cross-process cache lock. Never
        // remove its files directly while another app instance could use them.
        let mut command = Command::new(uv);
        hide_console(&mut command);
        let output = command
            .args(["cache", "clean"])
            .env("UV_CACHE_DIR", cache)
            .env("UV_LOCK_TIMEOUT", "1")
            .env("UV_NO_PROGRESS", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .pipe_output_timeout()
            .map_err(|error| format!("Không thể mở bộ dọn cache: {error}"))?;
        if !output.status.success() {
            log::warn!(
                "AI installation cache cleanup failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return Err(
                "Chưa thể xóa cache cài đặt. Hãy chờ các lượt cài AI kết thúc rồi thử lại.".into(),
            );
        }
        Ok(())
    })
}

fn install_feature(
    app: &AppHandle,
    uv: &Path,
    root: &Path,
    source: &Path,
    python: &Path,
    feature: &str,
    profile: &str,
) -> Result<(), String> {
    let python = python.to_str().ok_or("Đường dẫn bộ xử lý không hợp lệ.")?;
    let mut requirements = source.join(requirement_name(feature));
    if feature == "bandit" {
        let wheels = crate::runtime_paths::optional_wheelhouse().ok_or("Thiếu thành phần cài đặt BandIt. Hãy cập nhật JC AI.")?;
        let wheel = verified_wheel(&wheels, "bandit-manifest.json", "bandit_infer-0.1.0-")?;
        let url = reqwest::Url::from_file_path(wheel).map_err(|_|"Đường dẫn cài BandIt không hợp lệ.")?;
        let original = fs::read_to_string(&requirements).map_err(|e|e.to_string())?;
        let contents = original.lines().map(|line|if line.starts_with("bandit-infer @ ") { format!("bandit-infer @ {url}") } else {line.to_string()}).collect::<Vec<_>>().join("\n");
        requirements = Path::new(python).parent().and_then(Path::parent).ok_or("Đường dẫn bộ xử lý không hợp lệ.")?.join("bandit-requirements.txt");
        fs::write(&requirements,contents).map_err(|e|e.to_string())?;
    }
    let requirements = requirements
        .to_str()
        .ok_or("Đường dẫn gói cài đặt không hợp lệ.")?;
    if !Path::new(requirements).is_file() {
        return Err("Ứng dụng thiếu danh sách thành phần. Hãy cập nhật hoặc cài lại JC AI.".into());
    }
    if matches!(feature, "ocr_openvino" | "whisper_openvino") {
        let constraints = Path::new(python)
            .parent()
            .and_then(Path::parent)
            .map(|dir| dir.join("existing-constraints.txt"));
        let mut args = vec![
            "pip",
            "install",
            "--no-progress",
            "--python",
            python,
            "-r",
            requirements,
        ];
        // uv splits constraint paths on spaces; a file URI preserves Unicode
        // and spaces in Windows account/data paths.
        let constraints_text = constraints.as_ref().filter(|path| path.is_file())
            .and_then(|path| reqwest::Url::from_file_path(path).ok())
            .map(|url| url.to_string());
        if let Some(value) = constraints_text.as_deref() {
            args.extend(["--constraint", value]);
        }
        run_uv(app, uv, root, &args, "Cài tăng tốc Intel")?;
        if feature == "whisper_openvino" {
            // Shared CPU fallback stays available without Torch. Existing CUDA
            // features already contain faster-whisper; preserve their ORT GPU.
            let mut fallback = vec![
                "pip",
                "install",
                "--no-progress",
                "--python",
                python,
                "faster-whisper==1.2.1",
            ];
            if profile == "cuda" {
                fallback.push("--no-deps");
            }
            if let Some(value) = constraints_text.as_deref() {
                fallback.extend(["--constraint", value]);
            }
            run_uv(app, uv, root, &fallback, "Chuẩn bị nhận dạng CPU dự phòng")?;
        }
        if feature == "ocr_openvino" && profile != "cuda" {
            let mut cpu_args = vec![
                "pip",
                "install",
                "--no-progress",
                "--python",
                python,
                "onnxruntime==1.30.0",
            ];
            if let Some(value) = constraints_text.as_deref() {
                cpu_args.extend(["--constraint", value]);
            }
            run_uv(app, uv, root, &cpu_args, "Chuẩn bị bộ đọc chữ CPU")?;
        }
        return Ok(());
    }
    let constraints = Path::new(python).parent().and_then(Path::parent).map(|p|p.join("existing-constraints.txt"));
    let constraint_uri = constraints.as_ref().filter(|p|p.is_file()).and_then(|p|reqwest::Url::from_file_path(p).ok()).map(|p|p.to_string());
    let mut pinned = Vec::new();
    if let Some(uri) = constraint_uri.as_deref() { pinned.extend(["--constraint",uri]); }
    let torch_index = if profile == "cuda" {
        "https://download.pytorch.org/whl/cu128"
    } else {
        "https://download.pytorch.org/whl/cpu"
    };
    let torch_label = if profile == "cuda" {
        "Cài bộ xử lý NVIDIA dùng chung"
    } else {
        "Cài bộ xử lý CPU dùng chung"
    };
    // uv checks the shared environment first. Already-installed torch and its
    // dependencies are reused by WhisperX and BandIt instead of downloaded twice.
    let mut packages = vec!["torch==2.8.0", "torchaudio==2.8.0"];
    if feature == "whisperx" {
        packages.push("torchvision==0.23.0");
    }
    let mut args = vec![
        "pip",
        "install",
        "--no-progress",
        "--python",
        python,
        "--index-url",
        torch_index,
    ];
    args.extend(packages);
    args.extend(pinned.iter().copied());
    run_uv(app, uv, root, &args, torch_label)?;
    if feature == "whisperx" {
        run_uv(
            app,
            uv,
            root,
            &[
                "pip",
                "install",
                "--no-progress",
                "--python",
                python,
                "--index-url",
                "https://download.pytorch.org/whl/cpu",
                "torchcodec==0.7.0",
            ],
            "Cài bộ đọc âm thanh",
        )?;
    }
    let mut feature_args = vec!["pip", "install", "--no-progress", "--python", python, "-r", requirements];
    feature_args.extend(pinned.iter().copied());
    run_uv(app,uv,root,&feature_args,match feature {
        "whisperx" => "Cài WhisperX", "bandit" => "Cài tách thoại gốc",
        "acceleration" => "Cài tăng tốc NVIDIA", _ => "Cài bộ xử lý STTN",
    })?;
    if feature == "acceleration" {
        run_uv(
            app,
            uv,
            root,
            &["pip", "uninstall", "--python", python, "onnxruntime"],
            "Chuẩn bị tăng tốc đọc chữ",
        )?;
        run_uv(
            app,
            uv,
            root,
            &[
                "pip",
                "install",
                "--no-progress",
                "--python",
                python,
                "onnxruntime-gpu==1.26.0",
            ],
            "Cài tăng tốc đọc chữ",
        )?;
    }
    Ok(())
}

fn verification_script(features: &[String], profile: &str) -> String {
    verification_script_scoped(features,profile,features.iter().any(|f|f=="acceleration"))
}
fn verification_script_scoped(features: &[String], profile: &str, cuda_ort: bool) -> String {
    let mut script = String::from("from packaging.version import Version\n");
    if features
        .iter()
        .any(|feature| !matches!(feature.as_str(), "ocr_openvino" | "whisper_openvino"))
    {
        script.push_str(
            "import torch\nassert Version(torch.__version__.split('+')[0]).release[:2] == (2, 8)\n",
        );
    }
    if features.iter().any(|feature| feature == "ocr_openvino") {
        script.push_str("from importlib.metadata import version\nassert version('openvino') == '2026.4.0'\nimport rapidocr, onnxruntime\nfrom worker.ocr.backends import probe_openvino_gpu\ncapability = probe_openvino_gpu()\nassert capability.available, capability.message\n");
    }
    if features.iter().any(|feature| feature == "whisper_openvino") {
        script.push_str("from importlib.metadata import version\nassert version('openvino') == '2026.4.0'\nassert version('openvino-genai') == '2026.4.0.0'\nassert version('openvino-tokenizers') == '2026.4.0.0'\nimport openvino_genai, faster_whisper\nfrom worker.ocr.backends import probe_openvino_gpu\nassert probe_openvino_gpu().available\n");
    }
    if profile == "cuda" && cfg!(windows) {
        script.push_str("import os, pathlib; gpu_dlls = os.add_dll_directory(str(pathlib.Path(torch.__file__).parent / 'lib'))\nimport onnxruntime; onnxruntime.preload_dlls()\n");
    }
    if features.iter().any(|feature| feature == "whisperx") {
        script.push_str(
            "import faster_whisper, whisperx, transformers, huggingface_hub\n\
            assert Version(transformers.__version__) == Version('4.57.6')\n\
            assert Version(huggingface_hub.__version__) == Version('0.36.2')\n",
        );
    }
    if features.iter().any(|feature| feature == "bandit") {
        script.push_str("import bandit_infer, soundfile\n");
    }
    if features.iter().any(|feature| feature == "sttn") {
        let expected_onnx_version = if cuda_ort {
            CUDA_ONNXRUNTIME_VERSION
        } else {
            "1.30.0"
        };
        script.push_str(&format!("import rapidocr, onnxruntime\nassert Version(onnxruntime.__version__) == Version('{expected_onnx_version}')\n"));
    }
    if features.iter().any(|feature| feature == "acceleration") {
        script.push_str(
            "from importlib.metadata import version as package_version\n\
            import faster_whisper, rapidocr, vieneu, gradio, onnxruntime, transformers, huggingface_hub\n\
            assert Version(gradio.__version__) == Version('5.49.1')\n\
            assert Version(transformers.__version__) == Version('4.57.6')\n\
            assert Version(huggingface_hub.__version__) == Version('0.36.2')\n\
            assert Version(package_version('onnxruntime-gpu')) == Version('1.26.0')\n\
            assert Version(onnxruntime.__version__) == Version('1.26.0')\n\
            assert 'CUDAExecutionProvider' in onnxruntime.get_available_providers()\n",
        );
        script.push_str(
            "from worker.ocr.service import _cuda_runtime_probe\n\
            cuda_ready, cuda_error = _cuda_runtime_probe()\n\
            assert cuda_ready, cuda_error\n",
        );
    }
    if profile == "cuda" && !features.is_empty() {
        script.push_str("assert torch.cuda.is_available()\n");
        if features
            .iter()
            .any(|feature| feature == "whisperx" || feature == "acceleration")
        {
            script.push_str("import ctranslate2; assert ctranslate2.get_cuda_device_count() > 0\n");
        }
    }
    script
}

fn verify(
    app: &AppHandle,
    python: &Path,
    source: &Path,
    features: &[String],
    profile: &str,
) -> Result<(), String> {
    emit(app, "Kiểm tra các tính năng đã cài");
    let directory = python.parent().and_then(Path::parent);
    let pack = directory.filter(|path|path.file_name().is_some_and(|n|n=="current"))
        .and_then(Path::parent).and_then(read_marker_at);
    let cuda_ort = features.iter().any(|f|f=="acceleration") || pack.is_some_and(|m|m.features.iter().any(|f|f=="acceleration"));
    let script = if features.len() > 1 { verification_script(features,profile) } else { verification_script_scoped(features, profile, cuda_ort) };
    let mut command = Command::new(python);
    hide_console(&mut command);
    let output = command
        .arg("-c")
        .arg(&script)
        .current_dir(source)
        .env("PYTHONPATH", source)
        .env(
            "JC_AI_GPU_RUNTIME",
            if profile == "cuda" { "1" } else { "0" },
        )
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .pipe_output_timeout()
        .map_err(|error| format!("Không thể kiểm tra thành phần vừa cài: {error}"))?;
    if !output.status.success() {
        log::error!(
            "Optional AI verification failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(
            "Thành phần vừa tải chưa hoạt động đúng trên máy này. Hãy kiểm tra kết nối và thử lại."
                .into(),
        );
    }
    let mut command = Command::new(python);
    hide_console(&mut command);
    let output = command
        .arg("-c")
        .arg("import worker.__main__")
        .current_dir(source)
        .env("PYTHONPATH", source)
        .env(
            "JC_AI_GPU_RUNTIME",
            if profile == "cuda" { "1" } else { "0" },
        )
        .env("PYTHONUTF8", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .pipe_output_timeout()
        .map_err(|error| format!("Không thể kiểm tra bộ xử lý AI: {error}"))?;
    if !output.status.success() {
        log::error!(
            "Optional AI worker startup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(
            "Bộ xử lý AI không khởi động được sau khi cài. Thành phần đang dùng được giữ nguyên."
                .into(),
        );
    }
    Ok(())
}

pub fn install(app: &AppHandle, feature: &str, profile: &str) -> Result<RuntimeStatus, String> {
    if !valid_install(feature, profile) {
        return Err("Lựa chọn cài đặt không hợp lệ.".into());
    }
    let _guard = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Một lượt cài đặt khác đang chạy. Hãy chờ hoàn tất rồi thử lại.".to_string())?;
    if crate::runtime_registry::registry().protocol_version != crate::worker_manager::PROTOCOL_VERSION {
        return Err("Phiên bản bộ xử lý không tương thích với ứng dụng.".into());
    }
    let _file_lock = installation_file_lock()?;
    recover_runtime_at(&feature_root(feature)?)?;
    install_locked(app, feature, profile)
}

fn install_locked(app: &AppHandle, feature: &str, profile: &str) -> Result<RuntimeStatus, String> {
    if profile == "cuda" && !nvidia_detected() {
        return Err("Windows chưa nhận diện GPU NVIDIA. Kiểm tra driver và kết nối GPU trước khi cài tăng tốc.".into());
    }
    let uv = crate::runtime_paths::optional_installer()
        .ok_or("Bộ cài thành phần bổ sung chưa có trong ứng dụng. Hãy cập nhật JC AI.")?;
    let source = crate::runtime_paths::optional_worker_source()
        .ok_or("Ứng dụng thiếu bộ xử lý AI. Hãy cập nhật JC AI.")?;
    if !matches!(feature, "ocr_openvino" | "whisper_openvino") {
        if let Some(legacy) = read_marker() {
            let intel_features: Vec<_> = legacy.features.iter().filter(|f| matches!(f.as_str(), "ocr_openvino" | "whisper_openvino")).cloned().collect();
            for intel in &intel_features {
                let dedicated = read_marker_at(&feature_root(intel)?);
                if !dedicated.is_some_and(|m| m.features.contains(intel)) {
                    emit(app, "Tách bộ xử lý Intel đã cài để giữ tương thích với NVIDIA/CPU");
                    install_locked(app, intel, "openvino")?;
                }
            }
            // Leave old packages intact. Subsequent staging only owns non-Intel
            // features; a dedicated Intel pack is verified before this migration.
            if !intel_features.is_empty() {
                let mut marker = legacy;
                marker.features.retain(|f| !intel_features.contains(f));
                write_marker(&root()?.join("current.json"), &marker)?;
            }
        }
    }
    let root = ensure_runtime_root_at(feature_root(feature)?)?;
    let shared = ensure_runtime_root()?;
    for name in ["cache", "python"] {
        let directory = shared.join(name);
        fs::create_dir_all(&directory)
            .map_err(|error| format!("Không thể tạo thư mục cài đặt: {error}"))?;
        let metadata = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err("Thư mục cài đặt không an toàn.".into());
        }
    }
    let old = read_marker_at(&root);
    let mut reuse_current = old.as_ref().is_some_and(|marker|
        marker.runtime_version == RUNTIME_VERSION && marker.profile == profile);
    if profile == "cpu"
        && old
            .as_ref()
            .is_some_and(|marker| marker.features.iter().any(|item| item == "acceleration"))
    {
        return Err("Máy đang dùng thành phần tăng tốc NVIDIA. Hãy chọn NVIDIA để dùng chung bộ xử lý đã cài.".into());
    }

    let already_installed = old.as_ref().is_some_and(|marker| {
        marker.runtime_version == RUNTIME_VERSION
            && marker.profile == profile
            && marker.features.iter().any(|item| item == feature)
    });
    if already_installed {
        let current_python = python_path(&root.join("current"));
        if verify(
            app,
            &current_python,
            &source,
            &old.as_ref().unwrap().features,
            profile,
        )
        .is_ok()
        {
            return Ok(status());
        }
        emit(app, "Phát hiện runtime AI không tương thích; đang sửa chữa");
        // A corrupt/incompatible pack must be rebuilt from pinned dependencies;
        // freezing its broken versions would prevent repair indefinitely.
        reuse_current=false;
    }

    let mut features = old
        .as_ref()
        .map(|marker| marker.features.clone())
        .unwrap_or_default();
    if !features.iter().any(|item| item == feature) {
        features.push(feature.to_string());
    }
    // Build every feature change in staging. The active runtime is left untouched
    // until installation, dependency checks and runtime verification all pass.
    // This prevents a failed VieNeu/OCR/BandIt install from corrupting a working
    // WhisperX environment.
    let replace = true;
    let destination = root.join("staging");
    safe_remove(&destination, &root)?;
    if reuse_current {
        emit(app, "Chuẩn bị bản cài dùng lại các thành phần hiện có");
        if let Err(error) = copy_runtime(&root.join("current"), &destination) {
            let _ = safe_remove(&destination, &root);
            return Err(error);
        }
        if let Err(error) = frozen_constraints(&python_path(&destination), &destination) {
            let _ = safe_remove(&destination, &root);
            return Err(error);
        }
    } else {
        run_uv(
            app,
            &uv,
            &root,
            &[
                "venv",
                "--no-project",
                "--managed-python",
                "--python",
                &crate::runtime_registry::registry().python_version,
                "--no-progress",
                destination
                    .to_str()
                    .ok_or("Đường dẫn cài đặt không hợp lệ.")?,
            ],
            "Chuẩn bị bộ xử lý",
        )?;
    }
    let python = python_path(&destination);
    let constraints=crate::runtime_registry::registry().pins.iter()
        .map(|(name,version)|format!("{name}=={version}"))
        .collect::<Vec<_>>().join("\n");
    fs::write(destination.join("matrix-constraints.txt"),constraints).map_err(|e|e.to_string())?;
    let install_result = (|| {
        let mut requested: Vec<&str> = if reuse_current {
            vec![feature]
        } else {
            features.iter().map(String::as_str).collect()
        };
        // OCR's optional CUDA build replaces the CPU onnxruntime package, so
        // install it after STTN's CPU OCR dependencies when both are selected.
        requested.sort_by_key(|item| *item == "acceleration");
        for item in requested {
            install_feature(app, &uv, &root, &source, &python, item, profile)?;
        }
        // Adding another feature can resolve the CPU ORT distribution again.
        // Restore the exclusive CUDA provider and its files before verification.
        if features.iter().any(|f| f == "acceleration") {
            let py = python.to_str().ok_or("Đường dẫn Python không hợp lệ.")?;
            run_uv(app, &uv, &root, &["pip", "uninstall", "--python", py, "onnxruntime"], "Kiểm tra bộ đọc chữ NVIDIA")?;
            run_uv(app, &uv, &root, &["pip", "install", "--python", py, "--reinstall-package", "onnxruntime-gpu", "onnxruntime-gpu==1.26.0"], "Hoàn thiện bộ đọc chữ NVIDIA")?;
        }
        check_dependencies(app, &uv, &root, &python, &source, &features)?;
        verify(app, &python, &source, &features, profile)
    })();
    if let Err(error) = install_result {
        if replace {
            let _ = safe_remove(&destination, &root);
        }
        return Err(error);
    }
    let marker = RuntimeMarker {
        runtime_version: RUNTIME_VERSION,
        profile: profile.into(),
        features,
    };
    fs::write(destination.join("runtime-manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schemaVersion": 1, "runtimeVersion": RUNTIME_VERSION, "protocolVersion": crate::worker_manager::PROTOCOL_VERSION,
        "pythonVersion": crate::runtime_registry::registry().python_version, "backend": profile,
        "features": marker.features, "installedAt": chrono::Utc::now().to_rfc3339(),
        "requirements": marker.features.iter().map(|f| requirement_name(f)).collect::<Vec<_>>()
    })).map_err(|e|e.to_string())?).map_err(|e|format!("Không thể lưu thông tin bộ xử lý: {e}"))?;
    if let Err(error)=promote(&root, &destination, &marker) {
        let _=safe_remove(&destination,&root);
        let _=fs::remove_file(root.join("current.json.tmp"));
        return Err(error);
    }
    // Keep downloaded packages until the user explicitly clears them in
    // Settings. Ordinary install success/failure must not evict this cache.
    emit(app, "Đã cài tính năng được chọn");
    Ok(status())
}

fn promote(root: &Path, destination: &Path, marker: &RuntimeMarker) -> Result<(), String> {
    if !marker.features.is_empty() {
        let source = crate::runtime_paths::optional_worker_source().ok_or("Không tìm thấy bộ xử lý AI.")?;
        let mut command = Command::new(python_path(destination));
        hide_console(&mut command);
        let output = command.args(["-m", "worker.runtime_dependencies", r#"{"inventory":true}"#])
            .current_dir(&source).env("PYTHONPATH", &source).pipe_output_timeout().map_err(|e|e.to_string())?;
        if !output.status.success() { return Err("Không đọc được danh sách thành phần vừa cài.".into()); }
        let inventory: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_|"Danh sách thành phần không hợp lệ.")?;
        fs::write(destination.join("python-packages.json"),serde_json::to_vec_pretty(&inventory).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    }
    crate::worker_manager::with_runtime_idle(&root.join("current"), || promote_idle(root, destination, marker))
}
fn promote_idle(root: &Path, destination: &Path, marker: &RuntimeMarker) -> Result<(), String> {
    write_marker(&root.join("current.json.tmp"), marker)?;
    let manifest = serde_json::json!({
        "schemaVersion": 1, "runtimeVersion": RUNTIME_VERSION, "protocolVersion": crate::worker_manager::PROTOCOL_VERSION,
        "pythonVersion": crate::runtime_registry::registry().python_version, "backend": marker.profile,
        "features": marker.features, "installedAt": chrono::Utc::now().to_rfc3339(),
        "requirements": marker.features.iter().map(|f| requirement_name(f)).collect::<Vec<_>>()
    });
    fs::write(destination.join("runtime-manifest.json"), serde_json::to_vec_pretty(&manifest).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let marker_temp = root.join("current.json.tmp");
    let marker_file = root.join("current.json");
    let marker_previous = root.join("previous.json");
    {
        let current = root.join("current");
        let previous = root.join("previous");
        safe_remove(&previous, &root)?;
        if current.exists() {
            rename_runtime(&current, &previous)
                .map_err(|error| format!("Không thể thay bộ xử lý cũ: {error}"))?;
        }
        if let Err(error) = rename_runtime(&destination, &current) {
            if previous.exists() {
                let _ = rename_runtime(&previous, &current);
            }
            return Err(format!("Không thể hoàn tất cài đặt: {error}"));
        }
    }
    if marker_previous.is_file() {
        if let Err(error) = fs::remove_file(&marker_previous) {
            {
                rollback_replacement(&root);
            }
            return Err(format!("Không thể dọn trạng thái cũ: {error}"));
        }
    }
    if marker_file.is_file() {
        if let Err(error) = fs::rename(&marker_file, &marker_previous) {
            {
                rollback_replacement(&root);
            }
            return Err(format!("Không thể cập nhật trạng thái cài đặt: {error}"));
        }
    }
    if let Err(error) = fs::rename(&marker_temp, &marker_file) {
        if marker_previous.is_file() {
            let _ = fs::rename(&marker_previous, &marker_file);
        }
        {
            rollback_replacement(&root);
        }
        return Err(format!("Không thể hoàn tất trạng thái cài đặt: {error}"));
    }
    let _ = fs::remove_file(&marker_previous);
    {
        cleanup_previous_runtime(&root);
    }
    Ok(())
}

fn write_marker(path: &Path, marker: &RuntimeMarker) -> Result<(), String> {
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().ok_or("Đường dẫn cài đặt không hợp lệ.")?)
        .map_err(|e|e.to_string())?;
    temporary.write_all(&serde_json::to_vec_pretty(marker).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    temporary.as_file().sync_all().map_err(|e|e.to_string())?;
    temporary.persist(path).map_err(|e|e.to_string())?;
    Ok(())
}

pub fn plan(feature: &str, profile: &str) -> Result<crate::runtime_registry::InstallPlan, String> {
    if !valid_install(feature, profile) { return Err("Lựa chọn cài đặt không hợp lệ.".into()); }
    let directory = feature_root(feature)?;
    let marker = read_marker_at(&directory).filter(|m| m.runtime_version == RUNTIME_VERSION && m.profile == profile);
    let installed = marker.as_ref().map(|m|m.features.clone()).unwrap_or_default();
    let staging = if marker.is_some() { crate::storage_service::directory_size(&directory.join("current")) } else { 0 };
    crate::runtime_registry::plan(feature, profile, &installed, staging)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureHealth { pub state: RuntimeHealthState, pub feature: String, pub installed: bool, pub verified: bool, pub backend: Option<String>, pub message: String }
pub fn check_feature(app: &AppHandle, feature: &str) -> Result<FeatureHealth, String> {
    if !FEATURES.contains(&feature) { return Err("Tính năng không hợp lệ.".into()); }
    let Some((python, source, profile)) = active_runtime_for(feature) else {
        return Ok(FeatureHealth { state:quick_feature_health(feature), feature:feature.into(), installed:false, verified:false, backend:None, message:match quick_feature_health(feature) {RuntimeHealthState::UpdateRequired=>"Bộ xử lý cần cập nhật. Hãy cài lại tính năng này; các model được giữ nguyên.",RuntimeHealthState::Broken=>"Bộ xử lý bị lỗi hoặc thiếu tệp. Hãy dùng Sửa bộ xử lý.",_=>"Chưa cài bộ xử lý."}.into() });
    };
    let verification_profile = if matches!(feature, "ocr_openvino" | "whisper_openvino") { "openvino" } else { &profile };
    let result = verify(app, &python, &source, &[feature.into()], verification_profile);
    Ok(FeatureHealth { state:if result.is_ok() {RuntimeHealthState::Ready} else {RuntimeHealthState::Broken}, feature:feature.into(), installed:true, verified:result.is_ok(), backend:Some(profile), message:result.err().unwrap_or_else(|| "Bộ xử lý đã sẵn sàng.".into()) })
}

pub fn uninstall(app: &AppHandle, feature: &str) -> Result<RuntimeStatus, String> {
    if !FEATURES.contains(&feature) { return Err("Tính năng không hợp lệ.".into()); }
    let _guard = INSTALL_LOCK.try_lock().map_err(|_|"Một lượt cài đặt khác đang chạy.")?;
    let _file_lock = installation_file_lock()?;
    let Some((python, source, profile)) = active_runtime_for(feature) else { return Ok(status()); };
    let current = python.parent().and_then(Path::parent).ok_or("Đường dẫn bộ xử lý không hợp lệ.")?;
    let directory = current.parent().ok_or("Đường dẫn bộ xử lý không hợp lệ.")?;
    // uv keeps a managed-Python alias junction next to the runtime. It is not
    // part of the venv being replaced or removed: validate only that venv and
    // the parent directories, leaving the shared Python/cache trees untouched.
    ensure_runtime_root_at(directory.to_path_buf())?;
    ensure_plain_tree(current)?;
    let mut marker = read_marker_at(directory).ok_or("Không đọc được trạng thái bộ xử lý.")?;
    marker.features.retain(|f| f != feature);
    let destination = directory.join("staging");
    safe_remove(&destination, directory)?;
    if marker.features.is_empty() {
        // Empty staged pack atomically disconnects the old runtime before it is
        // cleaned up. Never uninstall shared dependencies from a live venv.
        fs::create_dir(&destination).map_err(|e|e.to_string())?;
        promote(directory,&destination,&marker)?;
        safe_remove(&directory.join("current"),directory)?;
        fs::remove_file(directory.join("current.json")).map_err(|e|e.to_string())?;
        return Ok(status());
    }
    copy_runtime(current, &destination)?;
    let result = (|| {
        let uv = crate::runtime_paths::optional_installer().ok_or("Thiếu bộ cài thành phần ứng dụng.")?;
        let staged_python = python_path(&destination);
        let py = staged_python.to_str().ok_or("Đường dẫn không hợp lệ.")?;
        let _owned = crate::runtime_registry::removable_packages(feature, &marker.features, &profile)?;
        let components = crate::runtime_registry::components(&marker.features,&profile)?;
        let roots = components.iter().flat_map(|id| crate::runtime_registry::registry().components[id].packages.clone()).collect::<Vec<_>>();
        let cuda_ort = marker.features.iter().any(|f| f == "acceleration");
        let request = serde_json::json!({"roots":roots,"cudaOrt":cuda_ort});
        let mut command = Command::new(&staged_python);
        hide_console(&mut command);
        let output = command.args(["-m","worker.runtime_dependencies", &request.to_string()])
            .current_dir(&source).env("PYTHONPATH",&source).pipe_output_timeout().map_err(|e|e.to_string())?;
        if !output.status.success() { return Err("Không xác định được các thành phần dùng chung; đã dừng gỡ để bảo vệ bộ xử lý.".into()); }
        let packages: Vec<String> = serde_json::from_slice(&output.stdout).map_err(|_|"Danh sách thành phần dùng chung không hợp lệ.")?;
        if !packages.is_empty() {
            let mut args = vec!["pip","uninstall","--python",py];
            args.extend(packages.iter().map(String::as_str));
            run_uv(app,&uv,directory,&args,"Gỡ tính năng được chọn; giữ bộ dùng chung")?;
        }
        if feature == "acceleration" && !marker.features.is_empty() {
            run_uv(app,&uv,directory,&["pip","install","--python",py,"--reinstall-package","onnxruntime","onnxruntime==1.30.0"],"Khôi phục bộ đọc chữ CPU")?;
        }
        check_dependencies(app,&uv,directory,&staged_python,&source,&marker.features)?;
        verify(app,&staged_python,&source,&marker.features,&profile)?;
        promote(directory,&destination,&marker)
    })();
    if let Err(error) = result { let _=safe_remove(&destination,directory); return Err(error); }
    emit(app,"Đã gỡ tính năng được chọn; các model được giữ nguyên");
    Ok(status())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn directory_promotion_retries_a_temporary_windows_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("staging");
        let target = temp.path().join("current");
        std::fs::create_dir(&source).unwrap();
        let handle = std::fs::OpenOptions::new().read(true).share_mode(0)
            .custom_flags(0x02000000).open(&source).unwrap(); // FILE_FLAG_BACKUP_SEMANTICS
        let unlock = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            drop(handle);
        });
        super::rename_runtime(&source, &target).unwrap();
        unlock.join().unwrap();
        assert!(target.is_dir());
        assert!(!source.exists());
    }
    use super::{
        cleanup_previous_runtime, clear_installation_cache_at,
        is_onnxruntime_gpu_metadata_conflict, safe_remove,
    };
    use std::{fs, sync::Mutex};
    use tempfile::tempdir;

    #[test]
    fn intel_install_is_independent_and_profiles_are_validated() {
        assert!(super::valid_install("ocr_openvino", "openvino"));
        assert!(super::valid_install("whisper_openvino", "openvino"));
        assert!(!super::valid_install("whisper_openvino", "cuda"));
        let whisper = super::verification_script(
            &["ocr_openvino".into(), "whisper_openvino".into()],
            "openvino",
        );
        assert!(whisper.contains("openvino_genai"));
        assert!(!whisper.contains("import torch"));
        assert!(!super::valid_install("ocr_openvino", "cuda"));
        assert!(!super::valid_install("whisperx", "openvino"));
        assert!(!super::valid_install("invented", "openvino"));
        assert!(super::valid_install("acceleration", "cuda"));
        assert!(super::valid_install("sttn", "cpu"));
        assert_eq!(
            super::requirement_name("ocr_openvino"),
            "requirements-optional-ocr-openvino.in"
        );
        let requirements = fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../python/requirements-optional-ocr-openvino.in"),
        )
        .unwrap();
        for unwanted in [
            "torch==",
            "torchaudio",
            "whisperx==",
            "transformers",
            "onnxruntime-gpu",
        ] {
            assert!(!requirements.contains(unwanted));
        }
        assert!(requirements.contains("openvino==2026.4.0"));
    }

    #[test]
    fn intel_only_verification_does_not_import_speech_or_torch() {
        let script = super::verification_script(&["ocr_openvino".into()], "openvino");
        assert!(!script.contains("import torch"));
        assert!(!script.contains("import faster_whisper"));
        assert!(script.contains("probe_openvino_gpu()"));
        assert!(script.contains("assert capability.available"));
        let combined =
            super::verification_script(&["ocr_openvino".into(), "whisperx".into()], "cuda");
        assert!(combined.contains("import torch"));
        assert!(combined.contains("assert torch.cuda.is_available()"));
        assert!(combined.contains("probe_openvino_gpu()"));
    }

    #[test]
    fn staged_runtime_copy_is_independent_and_rollback_preserves_current() {
        let temp = tempdir().unwrap();
        let current = temp.path().join("current");
        let staging = temp.path().join("staging");
        fs::create_dir_all(current.join("Lib/site-packages")).unwrap();
        fs::write(current.join("Lib/site-packages/package.py"), b"original").unwrap();
        super::copy_runtime(&current, &staging).unwrap();
        fs::write(staging.join("Lib/site-packages/package.py"), b"changed").unwrap();
        assert_eq!(
            fs::read(current.join("Lib/site-packages/package.py")).unwrap(),
            b"original"
        );
        super::safe_remove(&staging, temp.path()).unwrap();
        fs::rename(&current, temp.path().join("previous")).unwrap();
        fs::create_dir(&current).unwrap();
        fs::write(current.join("bad.py"), b"broken").unwrap();
        super::rollback_replacement(temp.path());
        assert_eq!(
            fs::read(current.join("Lib/site-packages/package.py")).unwrap(),
            b"original"
        );
        assert!(!current.join("bad.py").exists());
    }

    #[test]
    fn successful_install_cleanup_keeps_downloads_and_current_runtime() {
        let temp = tempdir().unwrap();
        for name in ["cache", "current", "previous", "python", "models"] {
            fs::create_dir(temp.path().join(name)).unwrap();
            fs::write(temp.path().join(name).join("keep.bin"), b"data").unwrap();
        }
        cleanup_previous_runtime(temp.path());
        assert!(!temp.path().join("previous").exists());
        for name in ["cache", "current", "python", "models"] {
            assert_eq!(
                fs::read(temp.path().join(name).join("keep.bin")).unwrap(),
                b"data"
            );
        }
    }

    #[test]
    fn failed_marker_promotion_rolls_back_runtime_and_metadata() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("current")).unwrap();
        fs::create_dir(root.join("staging")).unwrap();
        fs::create_dir(root.join("previous.json")).unwrap(); // simulate a marker write failure
        fs::write(root.join("current/old.bin"),b"working").unwrap();
        fs::write(root.join("staging/new.bin"),b"replacement").unwrap();
        fs::write(root.join("current.json"),b"old metadata").unwrap();
        let marker=super::RuntimeMarker {runtime_version:7,profile:"cpu".into(),features:vec![]};
        assert!(super::promote_idle(root,&root.join("staging"),&marker).is_err());
        assert_eq!(fs::read(root.join("current/old.bin")).unwrap(),b"working");
        assert_eq!(fs::read(root.join("current.json")).unwrap(),b"old metadata");
        assert!(!root.join("current/new.bin").exists());
    }

    #[test]
    fn failed_manifest_write_never_replaces_current() {
        let temp=tempdir().unwrap();
        let root=temp.path();
        fs::create_dir(root.join("current")).unwrap();
        fs::create_dir_all(root.join("staging/runtime-manifest.json")).unwrap();
        fs::write(root.join("current/old.bin"),b"working").unwrap();
        let marker=super::RuntimeMarker {runtime_version:7,profile:"cpu".into(),features:vec![]};
        assert!(super::promote_idle(root,&root.join("staging"),&marker).is_err());
        assert_eq!(fs::read(root.join("current/old.bin")).unwrap(),b"working");
    }

    #[test]
    fn startup_recovers_interrupted_directory_and_marker_switches() {
        for interrupted_marker in [false,true] {
            let temp=tempdir().unwrap(); let root=temp.path();
            fs::create_dir(root.join("previous")).unwrap();
            fs::write(root.join("previous/old.bin"),b"working").unwrap();
            let marker=super::RuntimeMarker {runtime_version:7,profile:"cpu".into(),features:vec!["sttn".into()]};
            let filename=if interrupted_marker {"previous.json"} else {"current.json"};
            super::write_marker(&root.join(filename),&marker).unwrap();
            if interrupted_marker {
                fs::create_dir(root.join("current")).unwrap();
                fs::write(root.join("current/new.bin"),b"uncommitted").unwrap();
            }
            super::recover_runtime_at(root).unwrap();
            assert_eq!(fs::read(root.join("current/old.bin")).unwrap(),b"working");
            assert!(root.join("current.json").is_file());
            assert!(!root.join("current/new.bin").exists());
        }
    }

    #[test]
    fn wheel_hash_and_path_are_validated_with_windows_bom() {
        use sha2::Digest;
        let temp=tempdir().unwrap(); let root=temp.path();
        let name="bandit_infer-0.1.0-py3-none-any.whl";
        fs::write(root.join(name),b"fixture wheel").unwrap();
        let hash=format!("{:x}",sha2::Sha256::digest(b"fixture wheel"));
        let manifest=serde_json::json!({"name":name,"sha256":hash});
        let mut encoded=b"\xef\xbb\xbf".to_vec();encoded.extend(serde_json::to_vec(&manifest).unwrap());
        fs::write(root.join("manifest.json"),encoded).unwrap();
        assert!(super::verified_wheel(root,"manifest.json","bandit_infer-0.1.0-").is_ok());
        fs::write(root.join(name),b"broken wheel").unwrap();
        assert!(super::verified_wheel(root,"manifest.json","bandit_infer-0.1.0-").is_err());
        fs::write(root.join("manifest.json"),br#"{"name":"../outside.whl","sha256":"unused"}"#).unwrap();
        assert!(super::verified_wheel(root,"manifest.json","bandit_infer-").is_err());
    }

    #[test]
    fn explicit_cache_cleanup_is_isolated_and_refuses_active_install() {
        let temp = tempdir().unwrap();
        let lock = Mutex::new(());
        for name in ["cache", "current", "staging", "python", "models"] {
            fs::create_dir(temp.path().join(name)).unwrap();
            fs::write(temp.path().join(name).join("keep.bin"), b"data").unwrap();
        }
        let clean = |cache: &std::path::Path| safe_remove(cache, temp.path());
        let guard = lock.lock().unwrap();
        assert!(clear_installation_cache_at(temp.path(), &lock, clean).is_err());
        assert!(temp.path().join("cache/keep.bin").exists());
        drop(guard);
        assert_eq!(
            clear_installation_cache_at(temp.path(), &lock, clean).unwrap(),
            4
        );
        assert!(!temp.path().join("cache").exists());
        assert_eq!(
            clear_installation_cache_at(temp.path(), &lock, clean).unwrap(),
            0
        );
        for name in ["current", "staging", "python", "models"] {
            assert_eq!(
                fs::read(temp.path().join(name).join("keep.bin")).unwrap(),
                b"data"
            );
        }
    }

    #[test]
    fn accepts_expected_onnxruntime_gpu_metadata_gap() {
        assert!(is_onnxruntime_gpu_metadata_conflict(
            "The package `vieneu` requires `onnxruntime>=1.20.0`, but it's not installed"
        ));
        assert!(is_onnxruntime_gpu_metadata_conflict(
            "The package `rapidocr` requires `onnxruntime>=1.7.0`, but it's not installed"
        ));
    }

    #[test]
    fn rejects_unrelated_dependency_conflicts() {
        assert!(!is_onnxruntime_gpu_metadata_conflict(
            "The package `foo` requires `bar>=2`, but it's not installed"
        ));
        assert!(!is_onnxruntime_gpu_metadata_conflict(
            "The package `vieneu` requires `onnxruntime>=2.0`, but you have `onnxruntime==1.30.0`"
        ));
    }
}
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x0400 != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}
