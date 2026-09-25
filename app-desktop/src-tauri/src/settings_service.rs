use crate::project_service::ProjectStore;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tempfile::NamedTempFile;

const SETTINGS_FILENAME: &str = "app-settings.json";
const KEYRING_SERVICE: &str = "com.ai.video-localization";
const LEGACY_TTS_VOICE: &str = "Ngọc Lan";
const DEFAULT_TTS_VOICE: &str = "Ngọc Huyền";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TranslationProviderId {
    Gemini,
    GoogleCloud,
    CodexCli,
}

impl TranslationProviderId {
    pub fn keyring_user(self) -> Option<&'static str> {
        match self {
            Self::Gemini => Some("translation.gemini.api-key"),
            Self::GoogleCloud => Some("translation.google-cloud.api-key"),
            Self::CodexCli => None,
        }
    }

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::GoogleCloud => "googleCloud",
            Self::CodexCli => "codexCli",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSettings {
    pub provider: TranslationProviderId,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_max_request_bytes")]
    pub max_request_bytes: usize,
    #[serde(default = "default_context_segments")]
    pub context_segments: usize,
    #[serde(
        default,
        rename = "legacyCustomPrompt",
        alias = "customPrompt",
        skip_serializing_if = "String::is_empty"
    )]
    pub legacy_custom_prompt: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub gemini: GeminiSettings,
    #[serde(default)]
    pub google_cloud: GoogleCloudSettings,
    #[serde(default)]
    pub codex_cli: CodexCliSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeminiSettings {
    #[serde(default = "default_gemini_model")]
    pub model_id: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCloudSettings {
    #[serde(default = "default_google_cloud_model")]
    pub model: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexCliSettings {
    #[serde(default = "default_codex_executable")]
    pub executable: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    #[serde(default)]
    pub translation: TranslationSettings,
    #[serde(default)]
    pub tts: TtsSettings,
    #[serde(default)]
    pub asr: AsrSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AsrSettings {
    #[serde(default = "default_asr_engine")]
    pub engine: String,
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default = "default_asr_device")]
    pub device: String,
    #[serde(default = "default_asr_language")]
    pub language: String,
    #[serde(default = "default_ocr_model")]
    pub ocr_model: String,
    #[serde(default = "default_ocr_device")]
    pub ocr_device: String,
}

impl Default for AsrSettings {
    fn default() -> Self {
        Self {
            engine: default_asr_engine(),
            model: default_asr_model(),
            device: default_asr_device(),
            language: default_asr_language(),
            ocr_model: default_ocr_model(),
            ocr_device: default_ocr_device(),
        }
    }
}

fn default_asr_engine() -> String {
    "fasterWhisper".to_string()
}
fn default_asr_model() -> String {
    "small".to_string()
}
fn default_asr_device() -> String {
    "auto".to_string()
}
fn default_asr_language() -> String {
    "auto".to_string()
}
fn default_ocr_model() -> String { "tiny".to_string() }
fn default_ocr_device() -> String { "auto".to_string() }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TtsSettings {
    #[serde(default = "default_tts_provider")]
    pub provider: String,
    #[serde(default = "default_tts_model")]
    pub model_id: String,
    #[serde(default = "default_tts_device")]
    pub device: String,
    #[serde(default = "default_tts_voice")]
    pub default_voice: String,
    #[serde(default = "default_tts_speed")]
    pub speed: f64,
    #[serde(default)]
    pub fit_to_duration: bool,
    #[serde(default)]
    pub voice_profiles: Vec<TtsVoiceProfile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceProfile {
    pub id: String,
    pub name: String,
    pub audio_path: String,
}

fn default_tts_model() -> String {
    "v3-turbo".to_string()
}
fn default_tts_provider() -> String {
    "vieneu".to_string()
}
fn default_tts_device() -> String {
    "auto".to_string()
}
fn default_tts_voice() -> String {
    DEFAULT_TTS_VOICE.to_string()
}
fn default_tts_speed() -> f64 {
    1.0
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            provider: default_tts_provider(),
            model_id: default_tts_model(),
            device: default_tts_device(),
            default_voice: default_tts_voice(),
            speed: default_tts_speed(),
            fit_to_duration: false,
            voice_profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsResponse {
    pub settings: AppSettings,
    pub gemini_credential_configured: bool,
    pub google_cloud_credential_configured: bool,
}

const fn default_batch_size() -> usize {
    30
}
const fn default_max_request_bytes() -> usize {
    20_000
}
const fn default_context_segments() -> usize {
    2
}
const fn default_timeout_seconds() -> u64 {
    90
}
const fn default_temperature() -> f32 {
    0.2
}
fn default_gemini_model() -> String {
    "gemini-3.8-flash".to_string()
}
fn default_google_cloud_model() -> String {
    "nmt".to_string()
}
fn default_codex_executable() -> String {
    "codex".to_string()
}

impl Default for GeminiSettings {
    fn default() -> Self {
        Self {
            model_id: default_gemini_model(),
            temperature: default_temperature(),
        }
    }
}

impl Default for GoogleCloudSettings {
    fn default() -> Self {
        Self {
            model: default_google_cloud_model(),
        }
    }
}

impl Default for CodexCliSettings {
    fn default() -> Self {
        Self {
            executable: default_codex_executable(),
            model_id: None,
            timeout_seconds: default_timeout_seconds(),
        }
    }
}

impl Default for TranslationSettings {
    fn default() -> Self {
        Self {
            provider: TranslationProviderId::Gemini,
            batch_size: default_batch_size(),
            max_request_bytes: default_max_request_bytes(),
            context_segments: default_context_segments(),
            legacy_custom_prompt: String::new(),
            timeout_seconds: default_timeout_seconds(),
            gemini: GeminiSettings::default(),
            google_cloud: GoogleCloudSettings::default(),
            codex_cli: CodexCliSettings::default(),
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 4,
            translation: TranslationSettings::default(),
            tts: TtsSettings::default(),
            asr: AsrSettings::default(),
        }
    }
}

pub fn get_settings(app: &AppHandle) -> Result<AppSettingsResponse, String> {
    let mut settings = read_settings(&settings_path(app)?)?;
    // Legacy project-wide prompts are migrated to project metadata and are never
    // returned as part of the global settings API.
    settings.translation.legacy_custom_prompt.clear();
    Ok(AppSettingsResponse {
        settings,
        gemini_credential_configured: credential_configured(TranslationProviderId::Gemini)?,
        google_cloud_credential_configured: credential_configured(
            TranslationProviderId::GoogleCloud,
        )?,
    })
}

pub fn save_settings(
    app: &AppHandle,
    mut settings: AppSettings,
) -> Result<AppSettingsResponse, String> {
    let existing = read_settings(&settings_path(app)?)?;
    if settings.translation.legacy_custom_prompt.is_empty() {
        settings.translation.legacy_custom_prompt = existing.translation.legacy_custom_prompt;
    }
    validate_settings(&settings.translation)?;
    validate_tts_settings(&settings.tts)?;
    validate_asr_settings(&settings.asr)?;
    settings.schema_version = 4;
    write_settings(&settings_path(app)?, &settings)?;
    get_settings(app)
}

pub fn migrate_legacy_custom_prompt(app: &AppHandle) -> Result<(), String> {
    let path = settings_path(app)?;
    let mut settings = read_settings(&path)?;
    let prompt = settings.translation.legacy_custom_prompt.trim().to_string();
    if prompt.is_empty() {
        return Ok(());
    }

    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Không thể xác định thư mục dữ liệu ứng dụng: {error}"))?
        .join("projects");
    let store = ProjectStore::new(root);
    let listing = store.list()?;
    if listing.projects.is_empty() {
        // Keep the legacy value until a project exists, then seed it once.
        return Ok(());
    }
    for mut project in listing.projects {
        if project.translation.custom_prompt.trim().is_empty() {
            project.translation.custom_prompt = prompt.clone();
            store.update_project(&project)?;
        }
    }
    if !listing.warnings.is_empty() {
        // Preserve the source until every managed project was readable.
        return Ok(());
    }
    settings.translation.legacy_custom_prompt.clear();
    write_settings(&path, &settings)
}

pub fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join(SETTINGS_FILENAME))
        .map_err(|error| format!("Không thể xác định nơi lưu cài đặt: {error}"))
}

pub fn read_settings(path: &std::path::Path) -> Result<AppSettings, String> {
    match fs::read(path) {
        Ok(bytes) => match parse_settings(&bytes) {
            Ok(mut settings) => {
                if migrate_legacy_tts_voice(&mut settings) {
                    write_settings(path, &settings)?;
                }
                Ok(settings)
            }
            Err(error) => {
                let backup = backup_settings(path, &bytes)?;
                log::warn!(
                    "Invalid global settings moved to backup {}: {}",
                    backup.display(),
                    error
                );
                Ok(AppSettings::default())
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(error) => Err(format!("Không thể đọc cài đặt: {error}")),
    }
}

fn migrate_legacy_tts_voice(settings: &mut AppSettings) -> bool {
    if settings.tts.default_voice == LEGACY_TTS_VOICE
        && !settings
            .tts
            .voice_profiles
            .iter()
            .any(|profile| profile.name == LEGACY_TTS_VOICE)
    {
        settings.tts.default_voice = DEFAULT_TTS_VOICE.to_string();
        true
    } else {
        false
    }
}

fn parse_settings(bytes: &[u8]) -> Result<AppSettings, String> {
    let mut settings: AppSettings = serde_json::from_slice(bytes)
        .map_err(|error| format!("File cài đặt bị lỗi hoặc không hợp lệ: {error}"))?;
    if !(1..=4).contains(&settings.schema_version) {
        return Err(format!(
            "Phiên bản cài đặt {} chưa được hỗ trợ.",
            settings.schema_version
        ));
    }
    settings.schema_version = 4;
    validate_settings(&settings.translation)?;
    validate_tts_settings(&settings.tts)?;
    validate_asr_settings(&settings.asr)?;
    Ok(settings)
}

fn backup_settings(path: &std::path::Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Thư mục cài đặt không hợp lệ.".to_string())?;
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SETTINGS_FILENAME);
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%f");
    let backup = parent.join(format!("{stem}.corrupt-{timestamp}.bak"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(|error| {
            format!("Không thể sao lưu cài đặt bị lỗi; dữ liệu gốc vẫn được giữ nguyên: {error}")
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Không thể hoàn tất bản sao lưu cài đặt: {error}"))?;
    Ok(backup)
}

pub fn validate_asr_settings(settings: &AsrSettings) -> Result<(), String> {
    if !["fasterWhisper", "whisperX"].contains(&settings.engine.as_str()) {
        return Err("Engine Speech Recognition không được hỗ trợ.".to_string());
    }
    if !["tiny", "base", "small", "medium", "large-v3"].contains(&settings.model.as_str()) {
        return Err("Model Speech Recognition không được hỗ trợ.".to_string());
    }
    if !["auto", "cpu", "cuda"].contains(&settings.device.as_str()) {
        return Err("Thiết bị Speech Recognition phải là Auto, CPU hoặc CUDA.".to_string());
    }
    if !["auto", "en", "vi", "ja", "ko", "zh", "fr", "de", "es"]
        .contains(&settings.language.as_str())
    {
        return Err("Ngôn ngữ Speech Recognition không được hỗ trợ.".to_string());
    }
    if !["tiny", "small", "medium"].contains(&settings.ocr_model.as_str()) {
        return Err("Model OCR phải là PP-OCRv6 Tiny, Small hoặc Medium.".to_string());
    }
    if !["auto", "cpu", "cuda"].contains(&settings.ocr_device.as_str()) {
        return Err("Thiết bị OCR phải là Auto, CPU hoặc CUDA.".to_string());
    }
    Ok(())
}

fn validate_tts_settings(settings: &TtsSettings) -> Result<(), String> {
    if settings.provider != "vieneu" {
        return Err("Provider TTS hiện hỗ trợ VieNeu-TTS cục bộ.".to_string());
    }
    if settings.model_id != "v3-turbo" {
        return Err("Model VieNeu hiện hỗ trợ v3 Turbo.".to_string());
    }
    if !matches!(settings.device.as_str(), "auto" | "cpu" | "cuda") {
        return Err("Thiết bị TTS phải là Auto, CPU hoặc CUDA.".to_string());
    }
    if settings.default_voice.trim().is_empty() || settings.default_voice.len() > 100 {
        return Err("Tên giọng VieNeu không hợp lệ.".to_string());
    }
    if !settings.speed.is_finite() || !(0.85..=2.0).contains(&settings.speed) {
        return Err("Tốc độ giọng đọc phải nằm trong khoảng 0.85–2.00×.".to_string());
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for profile in &settings.voice_profiles {
        if uuid::Uuid::parse_str(&profile.id).is_err() || !ids.insert(profile.id.as_str()) {
            return Err("Voice profile có ID không hợp lệ hoặc bị trùng.".to_string());
        }
        let name = profile.name.trim();
        if name.is_empty()
            || name.chars().count() > 48
            || name.chars().any(char::is_control)
            || !names.insert(name.to_lowercase())
        {
            return Err("Tên voice profile trống, quá dài hoặc bị trùng.".to_string());
        }
        let path = std::path::Path::new(&profile.audio_path);
        if path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || !profile.audio_path.starts_with("voice-profiles/")
        {
            return Err("Đường dẫn voice profile không an toàn.".to_string());
        }
    }
    Ok(())
}

pub fn write_settings(path: &std::path::Path, settings: &AppSettings) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Đường dẫn cài đặt không hợp lệ.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Không thể tạo thư mục cài đặt: {error}"))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Không thể tạo JSON cài đặt: {error}"))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Không thể tạo file cài đặt tạm: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("Không thể ghi cài đặt: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Không thể lưu cài đặt: {}", error.error))?;
    Ok(())
}

pub fn validate_settings(settings: &TranslationSettings) -> Result<(), String> {
    if !(1..=100).contains(&settings.batch_size) {
        return Err("Batch size phải nằm trong khoảng 1–100.".to_string());
    }
    if !(2_000..=28_000).contains(&settings.max_request_bytes) {
        return Err("Giới hạn nội dung request phải nằm trong khoảng 2–28 KB.".to_string());
    }
    if settings.context_segments > 8 {
        return Err("Số segment ngữ cảnh không được vượt quá 8.".to_string());
    }
    if !(5..=600).contains(&settings.timeout_seconds) {
        return Err("Timeout dịch phải nằm trong khoảng 5–600 giây.".to_string());
    }
    let model = settings.gemini.model_id.trim();
    if model.is_empty()
        || model.len() > 120
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
        return Err(
            "Gemini model ID chỉ được chứa chữ, số, dấu chấm, gạch dưới hoặc gạch ngang."
                .to_string(),
        );
    }
    if !settings.gemini.temperature.is_finite()
        || !(0.0..=2.0).contains(&settings.gemini.temperature)
    {
        return Err("Gemini temperature phải nằm trong khoảng 0–2.".to_string());
    }
    if settings.google_cloud.model != "nmt" {
        return Err(
            "Cloud Translation API v2 hiện chỉ cấu hình model NMT trong ứng dụng này.".to_string(),
        );
    }
    let executable = settings.codex_cli.executable.trim();
    if executable.is_empty() || executable.len() > 1_024 || executable.chars().any(char::is_control)
    {
        return Err("Codex CLI executable không hợp lệ.".to_string());
    }
    if let Some(model) = settings.codex_cli.model_id.as_deref() {
        if model.len() > 120 || model.chars().any(char::is_control) {
            return Err("Codex CLI model ID không hợp lệ.".to_string());
        }
    }
    if !(5..=600).contains(&settings.codex_cli.timeout_seconds) {
        return Err("Codex CLI timeout phải nằm trong khoảng 5–600 giây.".to_string());
    }
    Ok(())
}

pub fn set_credential(provider: TranslationProviderId, secret: &str) -> Result<(), String> {
    let username = provider.keyring_user().ok_or_else(|| {
        "Codex CLI sử dụng phiên đăng nhập riêng, không lưu API key tại đây.".to_string()
    })?;
    let value = secret.trim();
    if value.is_empty() || value.len() > 16_384 || value.chars().any(char::is_control) {
        return Err("Credential không hợp lệ.".to_string());
    }
    Entry::new(KEYRING_SERVICE, username)
        .and_then(|entry| entry.set_password(value))
        .map_err(|_| "Không thể lưu credential vào kho bảo mật của hệ điều hành.".to_string())
}

pub fn get_credential(provider: TranslationProviderId) -> Result<String, String> {
    let username = provider
        .keyring_user()
        .ok_or_else(|| "Codex CLI không dùng API key trong ứng dụng.".to_string())?;
    match Entry::new(KEYRING_SERVICE, username).and_then(|entry| entry.get_password()) {
        Ok(secret) => Ok(secret),
        Err(keyring::Error::NoEntry) => Err(
            "Provider chưa có credential. Hãy thêm API key trong Cài đặt → Translation."
                .to_string(),
        ),
        Err(_) => Err("Không thể đọc credential từ kho bảo mật của hệ điều hành.".to_string()),
    }
}

pub fn delete_credential(provider: TranslationProviderId) -> Result<(), String> {
    let Some(username) = provider.keyring_user() else {
        return Ok(());
    };
    match Entry::new(KEYRING_SERVICE, username).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Không thể xóa credential khỏi kho bảo mật của hệ điều hành.".to_string()),
    }
}

fn credential_configured(provider: TranslationProviderId) -> Result<bool, String> {
    let username = provider
        .keyring_user()
        .ok_or_else(|| "Codex CLI không dùng API key trong ứng dụng.".to_string())?;
    match Entry::new(KEYRING_SERVICE, username).and_then(|entry| entry.get_password()) {
        Ok(secret) => {
            drop(secret);
            Ok(true)
        }
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(_) => Err("Không thể truy cập kho credential của hệ điều hành.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn settings_round_trip_contains_no_credential_fields() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILENAME);
        let settings = AppSettings::default();
        write_settings(&path, &settings).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.to_ascii_lowercase().contains("api-key"));
        assert!(!raw.to_ascii_lowercase().contains("credential"));
        assert_eq!(read_settings(&path).unwrap(), settings);
    }

    #[test]
    fn public_settings_response_reports_credential_presence_without_serializing_secrets() {
        let response = AppSettingsResponse {
            settings: AppSettings::default(),
            gemini_credential_configured: true,
            google_cloud_credential_configured: false,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("geminiCredentialConfigured"));
        assert!(!json.to_ascii_lowercase().contains("api-key"));
        assert!(!json.to_ascii_lowercase().contains("credential\""));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn validates_provider_parameters_and_batch_limits() {
        let mut settings = TranslationSettings::default();
        assert!(validate_settings(&settings).is_ok());
        settings.batch_size = 0;
        assert!(validate_settings(&settings).is_err());
        settings = TranslationSettings::default();
        settings.gemini.model_id = "bad/model".to_string();
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn task_three_settings_gain_default_tts_settings_without_changing_provider_credentials() {
        let old = serde_json::json!({
            "schemaVersion": 1,
            "translation": TranslationSettings::default()
        });
        let decoded: AppSettings = serde_json::from_value(old).unwrap();
        assert_eq!(decoded.tts.provider, "vieneu");
        assert_eq!(decoded.tts.model_id, "v3-turbo");
        assert_eq!(decoded.tts.default_voice, "Ngọc Huyền");
        assert!(decoded.tts.voice_profiles.is_empty());
    }

    #[test]
    fn corrupt_global_settings_are_backed_up_before_defaults_are_returned() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILENAME);
        let original = b"{ broken settings";
        fs::write(&path, original).unwrap();
        let settings = read_settings(&path).unwrap();
        assert_eq!(settings, AppSettings::default());
        assert_eq!(fs::read(&path).unwrap(), original);
        let backup = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|entry| entry.extension().and_then(|value| value.to_str()) == Some("bak"))
            .expect("corrupt settings backup");
        assert_eq!(fs::read(backup).unwrap(), original);
    }
}
