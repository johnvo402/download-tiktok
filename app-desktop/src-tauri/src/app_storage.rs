//! Resolve storage before opening a window, so WebView and backend share a root.
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const INSTALLER_CONFIG: &str = "jc-ai-data-dir.txt";

pub fn resolve(app: &tauri::AppHandle) -> io::Result<PathBuf> {
    use tauri::Manager;
    if cfg!(debug_assertions) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let configured =
            std::env::var_os("JC_QA_DATA_DIR").or_else(|| std::env::var_os("JC_AI_DATA_DIR"));
        let configured = match configured {
            Some(value) => Some(value),
            None => development_env(repository)?,
        };
        return development_root(repository, configured);
    }
    let executable = std::env::current_exe()?;
    let config = executable
        .parent()
        .ok_or_else(|| io::Error::other("Không tìm thấy thư mục cài ứng dụng."))?
        .join(INSTALLER_CONFIG);
    // Older installations have no storage selection file. Keep their existing
    // data available; every new installer writes an explicit selected root.
    match fs::read(config) {
        Ok(bytes) => installed_root(&bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            app.path().app_data_dir().map_err(io::Error::other)
        }
        Err(error) => Err(error),
    }
}

fn development_env(repository: &Path) -> io::Result<Option<OsString>> {
    for name in [".env.local", ".env"] {
        let entries = match dotenvy::from_path_iter(repository.join(name)) {
            Ok(entries) => entries,
            Err(dotenvy::Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io::Error::other(error)),
        };
        for entry in entries {
            let (key, value) = entry.map_err(io::Error::other)?;
            if key == "JC_AI_DATA_DIR" {
                return Ok(Some(value.into()));
            }
        }
    }
    Ok(None)
}

fn development_root(repository: &Path, configured: Option<OsString>) -> io::Result<PathBuf> {
    let path = configured
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".app-data"));
    if path.as_os_str().is_empty() {
        return Err(io::Error::other("JC_AI_DATA_DIR không được để trống."));
    }
    std::path::absolute(if path.is_absolute() {
        path
    } else {
        repository.join(path)
    })
}

fn installed_root(bytes: &[u8]) -> io::Result<PathBuf> {
    // NSIS FileWriteUTF16LE preserves Vietnamese/Unicode directory names.
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err(io::Error::other("Cấu hình nơi lưu dữ liệu bị hỏng."));
        }
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect();
        String::from_utf16(&units).map_err(io::Error::other)?
    } else {
        String::from_utf8(bytes.to_vec()).map_err(io::Error::other)?
    };
    let text = text
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n']);
    let path = PathBuf::from(text);
    if text.is_empty() || text.contains(['\0', '\r', '\n']) || !path.is_absolute() {
        return Err(io::Error::other(
            "Nơi lưu dữ liệu phải là đường dẫn tuyệt đối hợp lệ. Hãy chạy lại trình cài đặt.",
        ));
    }
    Ok(path)
}

pub fn prepare(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    // Check the selected location; never silently fall back to AppData when a
    // chosen drive is unavailable, full, or read-only.
    let probe = tempfile::Builder::new()
        .prefix(".jc-ai-write-check-")
        .tempfile_in(root)?;
    probe.close()?;
    fs::create_dir_all(root.join("cache/tmp"))?;
    fs::create_dir_all(root.join("webview"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_is_isolated_and_relative_paths_are_repository_based() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            development_root(directory.path(), None).unwrap(),
            directory.path().join(".app-data")
        );
        assert_eq!(
            development_root(directory.path(), Some("data dev".into())).unwrap(),
            directory.path().join("data dev")
        );
        assert_eq!(
            development_root(directory.path(), Some(directory.path().into())).unwrap(),
            directory.path()
        );
        assert!(development_root(directory.path(), Some("".into())).is_err());
    }

    #[test]
    fn reads_unicode_installer_paths_in_utf16_and_utf8() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Dữ liệu JC AI");
        let value = format!("{}\r\n", path.display());
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(value.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(installed_root(&bytes).unwrap(), path);
        assert_eq!(installed_root(value.as_bytes()).unwrap(), path);
        assert_eq!(
            installed_root(format!("\u{feff}{value}").as_bytes()).unwrap(),
            path
        );
    }

    #[test]
    fn invalid_installer_config_does_not_fall_back_to_appdata() {
        for value in [
            b"".as_slice(),
            b"relative/data",
            b"\xff\xfe\x01",
            b"\xff\xfe\x00\xd8",
            b"\xff",
        ] {
            assert!(installed_root(value).is_err());
        }
        let directory = tempfile::tempdir().unwrap();
        assert!(
            installed_root(format!("{}\nextra", directory.path().display()).as_bytes()).is_err()
        );
    }

    #[test]
    fn local_env_takes_priority_and_missing_files_use_default() {
        let directory = tempfile::tempdir().unwrap();
        assert!(development_env(directory.path()).unwrap().is_none());
        fs::write(
            directory.path().join(".env"),
            "JC_AI_DATA_DIR='D:/Dữ liệu JC AI'\n",
        )
        .unwrap();
        assert_eq!(
            development_env(directory.path()).unwrap(),
            Some("D:/Dữ liệu JC AI".into())
        );
        fs::write(
            directory.path().join(".env.local"),
            "JC_AI_DATA_DIR=./local-data\n",
        )
        .unwrap();
        assert_eq!(
            development_env(directory.path()).unwrap(),
            Some("./local-data".into())
        );
    }

    #[test]
    fn creates_profile_and_temporary_directories_and_rejects_file_target() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("new-data");
        prepare(&root).unwrap();
        assert!(root.join("webview").is_dir());
        assert!(root.join("cache/tmp").is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        let file = directory.path().join("file");
        fs::write(&file, b"preserve me").unwrap();
        assert!(prepare(&file).is_err());
        assert_eq!(fs::read(file).unwrap(), b"preserve me");
    }
}
