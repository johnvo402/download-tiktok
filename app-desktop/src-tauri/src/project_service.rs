use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::NamedTempFile;
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 8;
const LEGACY_TTS_VOICE: &str = "Ngọc Lan";
const DEFAULT_TTS_VOICE: &str = "Ngọc Huyền";
const MAX_NAME_CHARS: usize = 100;
const PROJECT_DIRECTORIES: &[&str] = &[
    "media/source",
    "media/overlays",
    "subtitles",
    "audio/extracted",
    "audio/tts",
    "cache",
    "cache/preview",
    "exports",
];
const PROJECT_LOCATIONS_FILENAME: &str = "project-locations.json";
static PROJECT_LOCATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub source: ProjectSource,
    #[serde(default)]
    pub transcript: Option<TranscriptMetadata>,
    pub translation: TranslationSettings,
    #[serde(default)]
    pub subtitle: SubtitleSettings,
    pub dubbing: DubbingSettings,
    #[serde(default)]
    pub blur_regions: Vec<BlurRegion>,
    #[serde(default)]
    pub logos: Vec<ProjectLogo>,
    #[serde(default)]
    pub transcript_settings: TranscriptSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptSettings {
    pub mode: TranscriptMode,
    /// Frames sampled per second for on-screen subtitle OCR (1, 2, or 4).
    pub ocr_fps: f64,
    /// Normalized [0,1] rectangle in source video coordinates.
    pub ocr_region: Option<OcrRegion>,
}

impl Default for TranscriptSettings {
    fn default() -> Self {
        Self { mode: TranscriptMode::SpeechRecognition, ocr_fps: 2.0, ocr_region: None }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptMode {
    #[default]
    SpeechRecognition,
    Ocr,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSource {
    pub video_path: Option<String>,
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    pub video_codec: Option<String>,
    #[serde(default)]
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub checksum_sha256: Option<String>,
    #[serde(default)]
    pub audio_streams: Vec<AudioStreamMetadata>,
    #[serde(default)]
    pub subtitle_tracks: Vec<SubtitleTrackMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamMetadata {
    pub stream_index: u32,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub channels: Option<u16>,
    pub sample_rate: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrackMetadata {
    pub stream_index: u32,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub text_supported: bool,
    pub image_based: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptMetadata {
    pub status: TranscriptStatus,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<TranscriptSourceType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_track: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub segment_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptStatus {
    NotStarted,
    Running,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptSourceType {
    Subtitle,
    Asr,
    Ocr,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    pub source: String,
    pub translated: String,
    pub translation_status: TranslationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<TtsSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranslationStatus {
    Empty,
    Machine,
    Edited,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TtsSegment {
    pub audio_relative_path: String,
    pub fingerprint: String,
    pub duration_ms: u64,
    pub status: TtsStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TtsStatus {
    Ready,
    Stale,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlurRegion {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub enabled: bool,
    pub strength: f64,
    #[serde(default)]
    pub role: BlurRegionRole,
    #[serde(default)]
    pub edge_blend: u8,
    #[serde(default)]
    pub pixel_noise: u8,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BlurRegionRole {
    #[default]
    Generic,
    Subtitle,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectLogo {
    pub id: String,
    pub relative_path: String,
    pub original_name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub enabled: bool,
    pub opacity: f64,
    pub maintain_aspect_ratio: bool,
}

impl Default for ProjectLogo {
    fn default() -> Self {
        Self {
            id: String::new(),
            relative_path: String::new(),
            original_name: String::new(),
            x: 0.78,
            y: 0.05,
            width: 0.16,
            height: 0.12,
            enabled: true,
            opacity: 1.0,
            maintain_aspect_ratio: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSettings {
    pub source_language: String,
    pub target_language: String,
    #[serde(default)]
    pub custom_prompt: String,
    pub segments: Vec<TranslationSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleSettings {
    pub enabled: bool,
    #[serde(default = "default_subtitle_font_size", alias = "font_size")]
    pub font_size: u16,
    #[serde(default = "default_subtitle_text_color", alias = "text_color")]
    pub text_color: String,
    #[serde(default = "default_subtitle_outline")]
    pub outline: bool,
    #[serde(default = "default_subtitle_bottom_margin", alias = "bottom_margin")]
    pub bottom_margin: u16,
    pub font_family: String,
    pub font_weight: u16,
    pub outline_color: String,
    pub outline_width: u16,
    pub background_enabled: bool,
    pub background_color: String,
    pub background_opacity: f64,
    pub alignment: String,
    pub max_width: u8,
    pub line_height: f64,
    pub vertical_position: u8,
    #[serde(default = "default_subtitle_padding_x")]
    pub padding_x: u8,
    #[serde(default = "default_subtitle_padding_y")]
    pub padding_y: u8,
    #[serde(default = "default_subtitle_auto_fit")]
    pub auto_fit: bool,
    #[serde(default = "default_subtitle_min_font_size")]
    pub min_font_size: u16,
    #[serde(default = "default_subtitle_vertical_alignment")]
    pub vertical_alignment: String,
}

fn default_subtitle_font_size() -> u16 {
    30
}
fn default_subtitle_text_color() -> String {
    "#FFFFFF".to_string()
}
fn default_subtitle_outline() -> bool {
    true
}
fn default_subtitle_bottom_margin() -> u16 {
    48
}
fn default_subtitle_padding_x() -> u8 {
    4
}
fn default_subtitle_padding_y() -> u8 {
    10
}
fn default_subtitle_auto_fit() -> bool {
    true
}
fn default_subtitle_min_font_size() -> u16 {
    20
}
fn default_subtitle_vertical_alignment() -> String {
    "middle".to_string()
}

impl Default for SubtitleSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            font_size: default_subtitle_font_size(),
            text_color: default_subtitle_text_color(),
            outline: default_subtitle_outline(),
            bottom_margin: default_subtitle_bottom_margin(),
            font_family: "Arial".to_string(),
            font_weight: 700,
            outline_color: "#000000".to_string(),
            outline_width: 3,
            background_enabled: false,
            background_color: "#000000".to_string(),
            background_opacity: 0.48,
            alignment: "center".to_string(),
            max_width: 86,
            line_height: 1.2,
            vertical_position: 88,
            padding_x: default_subtitle_padding_x(),
            padding_y: default_subtitle_padding_y(),
            auto_fit: default_subtitle_auto_fit(),
            min_font_size: default_subtitle_min_font_size(),
            vertical_alignment: default_subtitle_vertical_alignment(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DubbingSettings {
    pub enabled: bool,
    pub provider: String,
    pub model_id: String,
    pub voice: Option<String>,
    pub voice_profile_id: Option<String>,
    pub speed: Option<f64>,
    pub fit_mode: Option<String>,
    pub original_volume: f64,
    pub dubbed_volume: f64,
    pub mute_original: bool,
    pub ducking_enabled: bool,
    pub ducking_amount: f64,
    #[serde(default)]
    pub dialogue_reduction_enabled: bool,
    #[serde(default = "default_dialogue_reduction_amount")]
    pub dialogue_reduction_amount: f64,
}

fn default_dialogue_reduction_amount() -> f64 { 0.65 }

impl Default for DubbingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "vieneu".to_string(),
            model_id: "v3-turbo".to_string(),
            voice: None,
            voice_profile_id: None,
            speed: None,
            fit_mode: None,
            original_volume: 1.0,
            dubbed_volume: 1.0,
            mute_original: false,
            ducking_enabled: false,
            ducking_amount: 0.75,
            dialogue_reduction_enabled: false,
            dialogue_reduction_amount: default_dialogue_reduction_amount(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListResponse {
    pub projects: Vec<Project>,
    pub warnings: Vec<String>,
}

pub struct ProjectStore {
    projects_root: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectLocationRegistry {
    custom_roots: HashMap<String, PathBuf>,
}

impl ProjectStore {
    pub fn new(projects_root: PathBuf) -> Self {
        Self { projects_root }
    }

    pub fn create(&self, name: &str) -> Result<Project, String> {
        self.create_at(name, None)
    }

    pub fn create_at(&self, name: &str, selected_root: Option<&Path>) -> Result<Project, String> {
        let name = validate_project_name(name)?;
        self.ensure_root()?;

        let default_root = self
            .projects_root
            .canonicalize()
            .map_err(|error| format!("Không thể xác minh thư mục lưu dự án mặc định: {error}"))?;
        let storage_root = match selected_root {
            Some(path) => canonical_storage_root(path)?,
            None => default_root.clone(),
        };
        let custom_storage = storage_root != default_root;

        let id = Uuid::new_v4().to_string();
        let destination = storage_root.join(&id);
        let staging = storage_root.join(format!(".creating-{id}"));
        if destination.exists() || staging.exists() {
            return Err("Không thể khởi tạo thư mục dự án mới. Hãy thử lại.".to_string());
        }

        fs::create_dir(&staging)
            .map_err(|error| format!("Không thể tạo thư mục dự án: {error}"))?;
        let now = now_iso();
        let project = Project {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            name,
            created_at: now.clone(),
            updated_at: now,
            source: ProjectSource {
                video_path: None,
                original_name: None,
                duration: None,
                width: None,
                height: None,
                fps: None,
                video_codec: None,
                audio_codec: None,
                file_size: None,
                checksum_sha256: None,
                audio_streams: Vec::new(),
                subtitle_tracks: Vec::new(),
            },
            transcript: None,
            translation: TranslationSettings {
                source_language: "auto".to_string(),
                target_language: "vi".to_string(),
                custom_prompt: String::new(),
                segments: Vec::new(),
            },
            subtitle: SubtitleSettings::default(),
            dubbing: DubbingSettings::default(),
            blur_regions: Vec::new(),
            logos: Vec::new(),
            transcript_settings: TranscriptSettings::default(),
        };

        let result: Result<Project, String> = (|| {
            for directory in PROJECT_DIRECTORIES {
                fs::create_dir_all(staging.join(directory))
                    .map_err(|error| format!("Không thể tạo thư mục dự án {directory}: {error}"))?;
            }
            write_project_file(&staging.join("project.json"), &project)?;
            fs::rename(&staging, &destination)
                .map_err(|error| format!("Không thể hoàn tất tạo dự án: {error}"))?;
            Ok(project)
        })();

        if result.is_err() && staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        let project = result?;
        if custom_storage {
            if let Err(error) = self.register_custom_root(&id, &storage_root) {
                if destination.exists()
                    && verify_direct_child(&storage_root, &destination, &id).is_ok()
                    && verify_tree_has_no_reparse_points(&destination).is_ok()
                {
                    let _ = fs::remove_dir_all(&destination);
                }
                return Err(error);
            }
        }
        Ok(project)
    }

    pub fn list(&self) -> Result<ProjectListResponse, String> {
        self.ensure_root()?;
        let entries = fs::read_dir(&self.projects_root)
            .map_err(|error| format!("Không thể đọc danh sách dự án: {error}"))?;
        let mut projects = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();

        for entry in entries {
            let entry = entry.map_err(|error| format!("Không thể đọc thư mục dự án: {error}"))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = match Uuid::parse_str(&name) {
                Ok(id) if id.to_string() == name => id.to_string(),
                _ => continue,
            };

            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    warnings.push(format!("Không thể kiểm tra dự án {id}: {error}"));
                    continue;
                }
            };
            if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
                warnings.push(format!("Đã bỏ qua dự án {id} vì đường dẫn không an toàn."));
                continue;
            }

            match read_project(&path, &id) {
                Ok(project) => {
                    seen.insert(project.id.clone());
                    projects.push(project);
                }
                Err(error) => warnings.push(format!("Dự án {id} không thể mở: {error}")),
            }
        }

        match self.read_location_registry() {
            Ok(registry) => {
                for (id, root) in registry.custom_roots {
                    let Ok(canonical_id) = canonical_project_id(&id) else {
                        warnings
                            .push("Đã bỏ qua một đường dẫn dự án có mã không hợp lệ.".to_string());
                        continue;
                    };
                    if canonical_id != id || seen.contains(&id) {
                        if seen.contains(&id) {
                            warnings.push(format!("Mã dự án {id} xuất hiện ở nhiều thư mục."));
                        }
                        continue;
                    }
                    match safe_project_path(&root, &id) {
                        Ok(path) => match read_project(&path, &id) {
                            Ok(project) => {
                                seen.insert(project.id.clone());
                                projects.push(project);
                            }
                            Err(error) => {
                                warnings.push(format!("Dự án {id} không thể mở: {error}"))
                            }
                        },
                        Err(error) => warnings.push(format!("Dự án {id} không thể mở: {error}")),
                    }
                }
            }
            Err(error) => warnings.push(format!(
                "Không thể đọc danh sách thư mục dự án tùy chọn: {error}"
            )),
        }

        projects.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(ProjectListResponse { projects, warnings })
    }

    pub fn get(&self, id: &str) -> Result<Project, String> {
        let id = canonical_project_id(id)?;
        self.ensure_root()?;
        let path = self.project_path(&id)?;
        read_project(&path, &id)
    }

    pub fn project_directory(&self, id: &str) -> Result<PathBuf, String> {
        let id = canonical_project_id(id)?;
        self.project_path(&id)
    }

    pub fn update_project(&self, project: &Project) -> Result<Project, String> {
        let id = canonical_project_id(&project.id)?;
        let path = self.project_path(&id)?;
        let existing = read_project(&path, &id)?;
        if existing.id != project.id {
            return Err("Mã dự án không hợp lệ.".to_string());
        }
        validate_project_name(&project.name)?;
        let mut updated = project.clone();
        updated.schema_version = SCHEMA_VERSION;
        updated.updated_at = now_iso_after(&existing.updated_at);
        write_project_file(&path.join("project.json"), &updated)?;
        Ok(updated)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Project, String> {
        let id = canonical_project_id(id)?;
        let name = validate_project_name(name)?;
        let path = self.project_path(&id)?;
        let mut project = read_project(&path, &id)?;
        project.name = name;
        project.updated_at = now_iso_after(&project.updated_at);
        write_project_file(&path.join("project.json"), &project)?;
        Ok(project)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let id = canonical_project_id(id)?;
        let custom_root = self.registered_root(&id)?;
        let root = custom_root.as_deref().unwrap_or(&self.projects_root);
        let path = self.project_path(&id)?;
        // Refuse to delete projects whose metadata is missing, corrupt, or mismatched.
        let _project = read_project(&path, &id)?;
        verify_tree_has_no_reparse_points(&path)?;

        let quarantine = root.join(format!(".deleting-{id}"));
        if quarantine.exists() {
            return Err(
                "Đang tồn tại dữ liệu xóa dở cho dự án này. Hãy kiểm tra thư mục dữ liệu ứng dụng."
                    .to_string(),
            );
        }
        fs::rename(&path, &quarantine)
            .map_err(|error| format!("Không thể chuyển dự án vào vùng xóa an toàn: {error}"))?;

        if custom_root.is_some() {
            if let Err(error) = self.remove_custom_root(&id) {
                let _ = fs::rename(&quarantine, &path);
                return Err(error);
            }
        }

        let delete_result = verify_direct_child(root, &quarantine, &format!(".deleting-{id}"))
            .and_then(|_| verify_tree_has_no_reparse_points(&quarantine))
            .and_then(|_| {
                fs::remove_dir_all(&quarantine)
                    .map_err(|error| format!("Không thể xóa toàn bộ dữ liệu dự án: {error}"))
            });

        if let Err(error) = delete_result {
            if quarantine.exists() && !path.exists() {
                let _ = fs::rename(&quarantine, &path);
                if let Some(root) = custom_root.as_deref() {
                    let _ = self.register_custom_root(&id, root);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn ensure_root(&self) -> Result<(), String> {
        fs::create_dir_all(&self.projects_root)
            .map_err(|error| format!("Không thể tạo thư mục lưu dự án: {error}"))?;
        let metadata = fs::symlink_metadata(&self.projects_root)
            .map_err(|error| format!("Không thể xác minh thư mục lưu dự án: {error}"))?;
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err(
                "Thư mục lưu dự án không hợp lệ hoặc là symlink/reparse point.".to_string(),
            );
        }
        Ok(())
    }

    fn project_path(&self, id: &str) -> Result<PathBuf, String> {
        self.ensure_root()?;
        let root = self
            .registered_root(id)?
            .unwrap_or_else(|| self.projects_root.clone());
        safe_project_path(&root, id)
    }

    fn location_registry_path(&self) -> PathBuf {
        self.projects_root
            .parent()
            .unwrap_or(&self.projects_root)
            .join(PROJECT_LOCATIONS_FILENAME)
    }

    fn read_location_registry(&self) -> Result<ProjectLocationRegistry, String> {
        let path = self.location_registry_path();
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("File đăng ký thư mục project không hợp lệ: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ProjectLocationRegistry::default())
            }
            Err(error) => Err(format!("Không thể đọc đăng ký thư mục project: {error}")),
        }
    }

    fn write_location_registry(&self, registry: &ProjectLocationRegistry) -> Result<(), String> {
        let path = self.location_registry_path();
        let parent = path
            .parent()
            .ok_or_else(|| "Thư mục đăng ký project không hợp lệ.".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Không thể tạo thư mục đăng ký project: {error}"))?;
        let bytes = serde_json::to_vec_pretty(registry)
            .map_err(|error| format!("Không thể tạo danh sách thư mục project: {error}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("Không thể tạo file đăng ký project tạm: {error}"))?;
        temporary
            .write_all(&bytes)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|error| format!("Không thể ghi đăng ký thư mục project: {error}"))?;
        temporary
            .persist(&path)
            .map_err(|error| format!("Không thể lưu đăng ký thư mục project: {}", error.error))?;
        Ok(())
    }

    fn registered_root(&self, id: &str) -> Result<Option<PathBuf>, String> {
        if fs::symlink_metadata(self.projects_root.join(id)).is_ok() {
            return Ok(None);
        }
        let registry = self.read_location_registry()?;
        let Some(root) = registry.custom_roots.get(id) else {
            return Ok(None);
        };
        let metadata = fs::symlink_metadata(root)
            .map_err(|_| "Không tìm thấy thư mục lưu dự án đã chọn.".to_string())?;
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err("Thư mục lưu dự án đã chọn không an toàn hoặc không khả dụng.".to_string());
        }
        let canonical = root
            .canonicalize()
            .map_err(|error| format!("Không thể xác minh thư mục lưu dự án đã chọn: {error}"))?;
        let default = self.projects_root.canonicalize().unwrap_or_default();
        if canonical == default {
            return Ok(None);
        }
        Ok(Some(canonical))
    }

    fn register_custom_root(&self, id: &str, root: &Path) -> Result<(), String> {
        let _guard = PROJECT_LOCATION_LOCK
            .lock()
            .map_err(|_| "Không thể khóa danh sách thư mục project.".to_string())?;
        let mut registry = self.read_location_registry()?;
        registry
            .custom_roots
            .insert(id.to_string(), root.to_path_buf());
        self.write_location_registry(&registry)
    }

    fn remove_custom_root(&self, id: &str) -> Result<(), String> {
        let _guard = PROJECT_LOCATION_LOCK
            .lock()
            .map_err(|_| "Không thể khóa danh sách thư mục project.".to_string())?;
        let mut registry = self.read_location_registry()?;
        registry.custom_roots.remove(id);
        self.write_location_registry(&registry)
    }
}

fn canonical_storage_root(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục lưu dự án đã chọn: {error}"))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| format!("Không thể truy cập thư mục lưu dự án đã chọn: {error}"))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err("Vui lòng chọn một thư mục lưu project hợp lệ.".to_string());
    }
    Ok(canonical)
}

fn safe_project_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| "Không tìm thấy thư mục lưu dự án đã chọn.".to_string())?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err("Thư mục lưu dự án không an toàn hoặc không khả dụng.".to_string());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục dự án: {error}"))?;
    let candidate = canonical_root.join(id);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|_| "Không tìm thấy dự án này trong thư mục đã đăng ký.".to_string())?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err("Đường dẫn dự án không hợp lệ hoặc không an toàn.".to_string());
    }
    verify_direct_child(&canonical_root, &candidate, id)?;
    Ok(candidate)
}

pub fn validate_project_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Tên dự án không được để trống.".to_string());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "Tên dự án không được dài quá {MAX_NAME_CHARS} ký tự."
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("Tên dự án không được chứa ký tự điều khiển.".to_string());
    }
    Ok(name.to_string())
}

pub fn validate_translation_custom_prompt(prompt: &str) -> Result<(), String> {
    if prompt.chars().count() > 8_000
        || prompt
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(
            "Hướng dẫn dịch tối đa 8.000 ký tự và không được chứa ký tự điều khiển.".to_string(),
        );
    }
    Ok(())
}

fn canonical_project_id(id: &str) -> Result<String, String> {
    Uuid::parse_str(id)
        .map(|uuid| uuid.to_string())
        .map_err(|_| "Mã dự án không hợp lệ.".to_string())
}

fn read_project(project_dir: &Path, expected_id: &str) -> Result<Project, String> {
    let metadata = project_dir.join("project.json");
    let file_metadata = fs::symlink_metadata(&metadata)
        .map_err(|_| "Không tìm thấy file project.json hoặc không thể đọc file.".to_string())?;
    if is_reparse_or_symlink(&file_metadata) || !file_metadata.is_file() {
        return Err("File project.json không hợp lệ hoặc là symlink/reparse point.".to_string());
    }
    let bytes = fs::read(&metadata)
        .map_err(|_| "Không tìm thấy file project.json hoặc không thể đọc file.".to_string())?;
    let mut project: Project = serde_json::from_slice(&bytes)
        .map_err(|error| format!("File project.json bị lỗi hoặc không hợp lệ: {error}"))?;
    if project.id != expected_id {
        return Err("Mã dự án trong project.json không khớp với thư mục.".to_string());
    }
    if project.schema_version > SCHEMA_VERSION || project.schema_version == 0 {
        return Err(format!(
            "Phiên bản dữ liệu {} chưa được hỗ trợ (ứng dụng hỗ trợ {}).",
            project.schema_version, SCHEMA_VERSION
        ));
    }
    validate_project_name(&project.name)?;
    if project.schema_version < SCHEMA_VERSION {
        let old_version = project.schema_version;
        backup_project_before_migration(&metadata, old_version, &bytes)?;
        if old_version < 5
            && project.dubbing.voice_profile_id.is_none()
            && project.dubbing.voice.as_deref() == Some(LEGACY_TTS_VOICE)
        {
            project.dubbing.voice = Some(DEFAULT_TTS_VOICE.to_string());
        }
        project.schema_version = SCHEMA_VERSION;
        write_project_file(&metadata, &project)?;
    }
    Ok(project)
}

fn backup_project_before_migration(
    path: &Path,
    version: u32,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Thư mục dự án không hợp lệ.".to_string())?;
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S-%f");
    let backup = parent.join(format!("project.json.pre-v{version}-{timestamp}.bak"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(|error| {
            format!("Không thể sao lưu dữ liệu trước khi nâng cấp project: {error}")
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Không thể hoàn tất bản sao lưu project: {error}"))?;
    Ok(backup)
}

fn write_project_file(path: &Path, project: &Project) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(project)
        .map_err(|error| format!("Không thể chuyển dữ liệu dự án thành JSON: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "Đường dẫn project.json không hợp lệ.".to_string())?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Không thể ghi dữ liệu dự án tạm thời: {error}"))?;
    temporary
        .write_all(&bytes)
        .map_err(|error| format!("Không thể ghi project.json: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Không thể lưu project.json: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Không thể cập nhật project.json: {}", error.error))?;
    Ok(())
}

fn verify_direct_child(root: &Path, candidate: &Path, expected_name: &str) -> Result<(), String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Không thể xác minh thư mục gốc dự án: {error}"))?;
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|_| "Không tìm thấy dự án này.".to_string())?;
    let parent = canonical_candidate
        .parent()
        .ok_or_else(|| "Đường dẫn dự án không an toàn.".to_string())?;
    let leaf = canonical_candidate
        .file_name()
        .and_then(|name| name.to_str());
    if parent != canonical_root || leaf != Some(expected_name) {
        return Err("Từ chối thao tác ngoài thư mục dự án được quản lý.".to_string());
    }
    Ok(())
}

fn verify_tree_has_no_reparse_points(path: &Path) -> Result<(), String> {
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("Không thể kiểm tra dữ liệu dự án: {error}"))?;
        if is_reparse_or_symlink(&metadata) {
            return Err("Dự án chứa symlink hoặc reparse point; đã từ chối xóa để bảo vệ dữ liệu ngoài dự án.".to_string());
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&current)
                .map_err(|error| format!("Không thể kiểm tra nội dung dự án: {error}"))?
            {
                let entry =
                    entry.map_err(|error| format!("Không thể kiểm tra nội dung dự án: {error}"))?;
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn is_reparse_or_symlink(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn now_iso_after(previous: &str) -> String {
    let now = Utc::now();
    let updated = chrono::DateTime::parse_from_rfc3339(previous)
        .map(|date| date.with_timezone(&Utc))
        .map(|previous| {
            let next_millisecond = previous + chrono::Duration::milliseconds(1);
            if now < next_millisecond {
                next_millisecond
            } else {
                now.clone()
            }
        })
        .unwrap_or(now);
    updated.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validates_project_names() {
        assert_eq!(validate_project_name("  Premiere  ").unwrap(), "Premiere");
        assert!(validate_project_name("  \t").is_err());
        assert!(validate_project_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert!(validate_project_name("hello\nworld").is_err());
    }

    #[test]
    fn creates_reads_renames_and_deletes_project_data() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let mut project = store.create("  My film  ").unwrap();
        assert_eq!(project.name, "My film");
        assert!(Uuid::parse_str(&project.id).is_ok());
        for directory in PROJECT_DIRECTORIES {
            assert!(temp
                .path()
                .join("projects")
                .join(&project.id)
                .join(directory)
                .is_dir());
        }
        let metadata_path = temp
            .path()
            .join("projects")
            .join(&project.id)
            .join("project.json");
        assert!(metadata_path.is_file());
        assert_eq!(store.get(&project.id).unwrap(), project);

        let original_id = project.id.clone();
        let old_updated = project.updated_at.clone();
        project = store.rename(&project.id, "Renamed").unwrap();
        assert_eq!(project.id, original_id);
        assert_eq!(project.name, "Renamed");
        assert!(project.updated_at > old_updated);
        assert_eq!(store.list().unwrap().projects, vec![project.clone()]);

        store.delete(&project.id).unwrap();
        assert!(!temp.path().join("projects").join(&project.id).exists());
        assert!(store.get(&project.id).is_err());
    }

    #[test]
    fn rejects_path_traversal_and_preserves_outside_files() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("projects");
        let outside = temp.path().join("important.txt");
        fs::write(&outside, "keep").unwrap();
        let store = ProjectStore::new(root);
        assert!(store.delete("../important.txt").is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
    }

    #[test]
    fn reports_corrupt_json_and_missing_projects() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Broken after disk edit").unwrap();
        let project_file = temp
            .path()
            .join("projects")
            .join(&project.id)
            .join("project.json");
        fs::write(project_file, "{not valid json").unwrap();
        assert!(store.get(&project.id).unwrap_err().contains("project.json"));
        let list = store.list().unwrap();
        assert!(list.projects.is_empty());
        assert_eq!(list.warnings.len(), 1);
        assert!(store
            .get("00000000-0000-4000-8000-000000000000")
            .unwrap_err()
            .contains("Không tìm thấy"));
    }

    #[test]
    fn upgrades_task_one_project_schema_without_losing_existing_fields() {
        let temp = tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Legacy project").unwrap();
        let path = temp
            .path()
            .join("projects")
            .join(&project.id)
            .join("project.json");
        let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        old["schemaVersion"] = serde_json::json!(1);
        old["source"] = serde_json::json!({ "videoPath": null });
        old.as_object_mut().unwrap().remove("transcript");
        for field in [
            "fontFamily",
            "fontWeight",
            "outlineColor",
            "outlineWidth",
            "backgroundEnabled",
            "backgroundColor",
            "backgroundOpacity",
            "alignment",
            "maxWidth",
            "lineHeight",
            "verticalPosition",
        ] {
            old["subtitle"].as_object_mut().unwrap().remove(field);
        }
        fs::write(&path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();

        let upgraded = store.get(&project.id).unwrap();
        assert_eq!(upgraded.schema_version, 8);
        assert_eq!(upgraded.id, project.id);
        assert_eq!(upgraded.name, "Legacy project");
        assert_eq!(upgraded.source.video_path, None);
        assert!(upgraded.transcript.is_none());
        assert_eq!(upgraded.subtitle.font_family, "Arial");
        assert_eq!(upgraded.subtitle.alignment, "center");
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], 8);
        assert_eq!(
            fs::read_dir(path.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str())
                        == Some("bak")
                )
                .count(),
            1
        );
    }

    #[test]
    fn migrates_old_blur_region_to_generic_without_auto_selecting_it_for_subtitles() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Legacy blur").unwrap();
        let file = store
            .project_directory(&project.id)
            .unwrap()
            .join("project.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        json["schemaVersion"] = serde_json::json!(3);
        json["blurRegions"] = serde_json::json!([{"id":"region-1","x":0.1,"y":0.7,"width":0.8,"height":0.2,"startMs":0,"endMs":null,"enabled":true,"strength":20.0}]);
        fs::write(&file, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        let loaded = store.get(&project.id).unwrap();
        assert_eq!(loaded.blur_regions[0].role, BlurRegionRole::Generic);
        assert_eq!(loaded.blur_regions[0].edge_blend, 0);
        assert_eq!(loaded.blur_regions[0].pixel_noise, 0);
        assert_eq!(loaded.schema_version, 8);
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], 8);
        assert_eq!(persisted["blurRegions"][0]["role"], "generic");
    }

    #[test]
    fn older_projects_receive_safe_dubbing_defaults() {
        let previous = serde_json::json!({
            "enabled": true,
            "originalVolume": 0.25,
            "dubbedVolume": 1.0,
            "duckingEnabled": true,
            "duckingAmount": 0.7
        });
        let migrated: DubbingSettings = serde_json::from_value(previous).unwrap();
        assert!(migrated.enabled);
        assert_eq!(migrated.provider, "vieneu");
        assert_eq!(migrated.model_id, "v3-turbo");
        assert_eq!(migrated.voice_profile_id, None);
        assert_eq!(migrated.fit_mode, None);
        assert!(!migrated.mute_original);
        assert_eq!(migrated.original_volume, 0.25);
    }

    #[test]
    fn refuses_to_delete_projects_with_symlinks_to_external_data() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("projects");
        let store = ProjectStore::new(root.clone());
        let project = store.create("Linked data").unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let outside_file = outside.join("keep.txt");
        fs::write(&outside_file, "preserve").unwrap();
        let link = root.join(&project.id).join("media").join("outside-link");

        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&outside, &link);

        if link_result.is_err() {
            // Windows may disable symlink creation without developer mode or elevated rights.
            return;
        }
        assert!(store.delete(&project.id).is_err());
        assert_eq!(fs::read_to_string(outside_file).unwrap(), "preserve");
    }
}
