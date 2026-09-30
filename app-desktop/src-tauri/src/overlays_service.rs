use crate::project_service::{BlurRegion, Project, ProjectLogo, ProjectStore, SubtitleSettings};
use crate::video_service::resolve_project_file;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command;
use uuid::Uuid;

const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 50_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualSettings {
    pub subtitle: SubtitleSettings,
    pub blur_regions: Vec<BlurRegion>,
    pub logos: Vec<ProjectLogo>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoMutationResult {
    pub project: Project,
    pub logo: Option<ProjectLogo>,
    pub cleanup_warning: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

pub fn save_visual_settings(
    store: &ProjectStore,
    project_id: &str,
    settings: VisualSettings,
) -> Result<Project, String> {
    validate_subtitle_settings(&settings.subtitle)?;
    validate_blur_regions(&settings.blur_regions)?;

    let mut project = store.get(project_id)?;
    validate_logos(store, project_id, &settings.logos)?;
    project.subtitle = settings.subtitle;
    project.blur_regions = settings.blur_regions;
    project.logos = settings.logos;
    store.update_project(&project)
}

pub fn update_subtitle_settings(
    store: &ProjectStore,
    project_id: &str,
    subtitle: SubtitleSettings,
) -> Result<Project, String> {
    validate_subtitle_settings(&subtitle)?;
    let mut project = store.get(project_id)?;
    project.subtitle = subtitle;
    store.update_project(&project)
}

pub fn import_logo(
    store: &ProjectStore,
    project_id: &str,
    source_path: &str,
) -> Result<LogoMutationResult, String> {
    let mut project = store.get(project_id)?;
    let (asset, original_name, image_width, image_height) =
        stage_image(store, project_id, source_path)?;
    let relative_path = format!(
        "media/overlays/{}",
        asset.file_name().unwrap().to_string_lossy()
    );
    let (x, y, width, height) = default_logo_rect(
        image_width,
        image_height,
        project.source.width.unwrap_or(16),
        project.source.height.unwrap_or(9),
    );
    let mut logo = ProjectLogo {
        id: Uuid::new_v4().to_string(),
        relative_path,
        original_name,
        x,
        y,
        width,
        height,
        enabled: true,
        opacity: 1.0,
        maintain_aspect_ratio: true,
    };
    project.logos.push(logo.clone());
    if let Err(error) = store.update_project(&project) {
        let _ = fs::remove_file(&asset);
        return Err(error);
    }
    // Return the persisted project to make the timestamp and schema version authoritative.
    project = store.get(project_id)?;
    logo = project
        .logos
        .iter()
        .find(|candidate| candidate.id == logo.id)
        .cloned()
        .ok_or_else(|| "Logo đã được lưu nhưng không thể đọc lại metadata.".to_string())?;
    Ok(LogoMutationResult {
        project,
        logo: Some(logo),
        cleanup_warning: None,
    })
}

pub fn replace_logo(
    store: &ProjectStore,
    project_id: &str,
    logo_id: &str,
    source_path: &str,
) -> Result<LogoMutationResult, String> {
    let mut project = store.get(project_id)?;
    let index = project
        .logos
        .iter()
        .position(|logo| logo.id == logo_id)
        .ok_or_else(|| "Không tìm thấy logo cần thay thế.".to_string())?;
    let old_relative = project.logos[index].relative_path.clone();
    let (asset, original_name, _, _) = stage_image(store, project_id, source_path)?;
    let relative_path = format!(
        "media/overlays/{}",
        asset.file_name().unwrap().to_string_lossy()
    );
    project.logos[index].relative_path = relative_path;
    project.logos[index].original_name = original_name;
    if let Err(error) = store.update_project(&project) {
        let _ = fs::remove_file(&asset);
        return Err(error);
    }
    let project = store.get(project_id)?;
    let logo = project
        .logos
        .iter()
        .find(|logo| logo.id == logo_id)
        .cloned();
    let cleanup_warning = cleanup_unreferenced_asset(store, project_id, &project, &old_relative);
    Ok(LogoMutationResult {
        project,
        logo,
        cleanup_warning,
    })
}

pub fn remove_logo(
    store: &ProjectStore,
    project_id: &str,
    logo_id: &str,
) -> Result<LogoMutationResult, String> {
    let mut project = store.get(project_id)?;
    let old_relative = project
        .logos
        .iter()
        .find(|logo| logo.id == logo_id)
        .map(|logo| logo.relative_path.clone())
        .ok_or_else(|| "Không tìm thấy logo cần xóa.".to_string())?;
    project.logos.retain(|logo| logo.id != logo_id);
    let project = store.update_project(&project)?;
    let cleanup_warning = cleanup_unreferenced_asset(store, project_id, &project, &old_relative);
    Ok(LogoMutationResult {
        project,
        logo: None,
        cleanup_warning,
    })
}

pub fn logo_asset_path(
    store: &ProjectStore,
    project_id: &str,
    logo_id: &str,
) -> Result<PathBuf, String> {
    let project = store.get(project_id)?;
    let logo = project
        .logos
        .iter()
        .find(|logo| logo.id == logo_id)
        .ok_or_else(|| "Không tìm thấy logo trong dự án.".to_string())?;
    resolve_project_file(store, project_id, &logo.relative_path, "media/overlays")
}

fn validate_subtitle_settings(settings: &SubtitleSettings) -> Result<(), String> {
    if let Some(region) = &settings.region {
        if ![region.x, region.y, region.width, region.height]
            .into_iter()
            .all(f64::is_finite)
            || region.x < 0.0
            || region.y < 0.0
            || region.width <= 0.0
            || region.height <= 0.0
            || region.x + region.width > 1.0 + 1e-9
            || region.y + region.height > 1.0 + 1e-9
        {
            return Err("Vùng phụ đề phải nằm hoàn toàn trong khung video.".into());
        }
    }
    if !(8..=240).contains(&settings.font_size) {
        return Err("Cỡ chữ phụ đề phải nằm trong khoảng 8–240 px theo khung video.".into());
    }
    if !(100..=900).contains(&settings.font_weight) {
        return Err("Độ đậm chữ phải nằm trong khoảng 100–900.".into());
    }
    if settings.outline_width > 20 || settings.bottom_margin > 240 {
        return Err("Độ dày viền hoặc lề dưới phụ đề vượt giới hạn.".into());
    }
    if !valid_color(&settings.text_color)
        || !valid_color(&settings.outline_color)
        || !valid_color(&settings.background_color)
    {
        return Err("Màu phụ đề phải là mã hex dạng #RRGGBB.".into());
    }
    if settings.font_family.trim().is_empty()
        || settings.font_family.chars().count() > 96
        || settings.font_family.chars().any(char::is_control)
    {
        return Err("Tên font phụ đề không hợp lệ.".into());
    }
    if !matches!(settings.alignment.as_str(), "left" | "center" | "right") {
        return Err("Căn lề phụ đề không hợp lệ.".into());
    }
    if !(10..=100).contains(&settings.max_width) || !(0.8..=3.0).contains(&settings.line_height) {
        return Err("Độ rộng hoặc khoảng dòng phụ đề nằm ngoài giới hạn.".into());
    }
    if settings.vertical_position > 100
        || !settings.background_opacity.is_finite()
        || !(0.0..=1.0).contains(&settings.background_opacity)
    {
        return Err("Vị trí hoặc độ trong suốt phụ đề không hợp lệ.".into());
    }
    if settings.padding_x > 30
        || settings.padding_y > 40
        || !(8..=240).contains(&settings.min_font_size)
        || settings.min_font_size > settings.font_size
        || !matches!(
            settings.vertical_alignment.as_str(),
            "top" | "middle" | "bottom"
        )
    {
        return Err("Cỡ chữ tối thiểu, padding hoặc căn dọc phụ đề không hợp lệ.".into());
    }
    Ok(())
}

fn validate_blur_regions(regions: &[BlurRegion]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    if regions
        .iter()
        .filter(|region| region.role == crate::project_service::BlurRegionRole::Subtitle)
        .count()
        > 1
    {
        return Err("Chỉ được chọn một vùng blur làm vùng phụ đề.".into());
    }
    if regions
        .iter()
        .filter(|region| region.role == crate::project_service::BlurRegionRole::Subtitle)
        .count()
        > 1
    {
        return Err("Chỉ được chọn một vùng blur làm vùng phụ đề.".into());
    }
    for region in regions {
        validate_id(&region.id)?;
        if !ids.insert(&region.id) {
            return Err("Có vùng blur bị trùng mã định danh.".into());
        }
        validate_rect(region.x, region.y, region.width, region.height)?;
        if !region.strength.is_finite() || !(1.0..=100.0).contains(&region.strength) {
            return Err("Độ mạnh blur phải nằm trong khoảng 1–100.".into());
        }
        if region.edge_blend > 50 || region.pixel_noise > 100 || ![1, 2].contains(&region.text_removal_scan_fps) {
            return Err(
                "Hòa mềm viền phải nằm trong khoảng 0–50%, nhiễu hạt trong khoảng 0–100% và tốc độ quét chữ phải là 1 hoặc 2 khung/giây.".into(),
            );
        }
        if region.end_ms.is_some_and(|end| end <= region.start_ms) {
            return Err("Thời điểm kết thúc vùng blur phải sau thời điểm bắt đầu.".into());
        }
    }
    Ok(())
}

fn validate_logos(
    store: &ProjectStore,
    project_id: &str,
    logos: &[ProjectLogo],
) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for logo in logos {
        validate_id(&logo.id)?;
        if !ids.insert(&logo.id) {
            return Err("Có logo bị trùng mã định danh.".into());
        }
        validate_rect(logo.x, logo.y, logo.width, logo.height)?;
        if !logo.opacity.is_finite() || !(0.0..=1.0).contains(&logo.opacity) {
            return Err("Độ mờ logo phải nằm trong khoảng 0–100%.".into());
        }
        if logo.original_name.chars().any(char::is_control)
            || logo.original_name.chars().count() > 255
        {
            return Err("Tên tệp logo không hợp lệ.".into());
        }
        resolve_project_file(store, project_id, &logo.relative_path, "media/overlays")?;
    }
    Ok(())
}

fn validate_rect(x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    if ![x, y, width, height].into_iter().all(f64::is_finite)
        || x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > 1.0 + 1e-9
        || y + height > 1.0 + 1e-9
    {
        return Err("Kích thước hoặc vị trí overlay nằm ngoài khung video.".into());
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value).map_err(|_| "Mã overlay không hợp lệ.".to_string())?;
    if parsed.to_string() != value {
        return Err("Mã overlay không hợp lệ.".into());
    }
    Ok(())
}

fn valid_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

fn stage_image(
    store: &ProjectStore,
    project_id: &str,
    source_path: &str,
) -> Result<(PathBuf, String, u32, u32), String> {
    let source = Path::new(source_path);
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Không thể đọc ảnh logo đã chọn: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err("Logo phải là một tệp ảnh hợp lệ.".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err("Ảnh logo vượt giới hạn dung lượng 25 MB.".into());
    }
    let original_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Tên ảnh logo không hợp lệ.".to_string())?
        .to_string();
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let signature = detect_image_format(source)?;
    if !matches!(
        (signature, extension.as_str()),
        (ImageFormat::Png, "png")
            | (ImageFormat::Jpeg, "jpg" | "jpeg")
            | (ImageFormat::Webp, "webp")
    ) {
        return Err(
            "Phần mở rộng không khớp nội dung ảnh. Chọn PNG, JPG, JPEG hoặc WEBP hợp lệ.".into(),
        );
    }
    let (width, height) = decode_image(source)?;
    let project_root = store.project_directory(project_id)?;
    let overlay_dir = project_root.join("media").join("overlays");
    fs::create_dir_all(&overlay_dir)
        .map_err(|error| format!("Không thể tạo thư mục logo trong dự án: {error}"))?;
    let dir_meta = fs::symlink_metadata(&overlay_dir)
        .map_err(|error| format!("Không thể xác minh thư mục logo: {error}"))?;
    if dir_meta.file_type().is_symlink() || !dir_meta.is_dir() {
        return Err("Thư mục logo không hợp lệ hoặc là liên kết ngoài.".into());
    }
    let asset_id = Uuid::new_v4().to_string();
    let destination = overlay_dir.join(format!("{asset_id}.{}", signature.extension()));
    let staging = overlay_dir.join(format!(".{asset_id}.importing"));
    fs::copy(source, &staging)
        .map_err(|error| format!("Không thể sao chép ảnh logo vào dự án: {error}"))?;
    if let Err(error) = fs::rename(&staging, &destination) {
        let _ = fs::remove_file(&staging);
        return Err(format!("Không thể hoàn tất sao chép ảnh logo: {error}"));
    }
    Ok((destination, original_name, width, height))
}

fn detect_image_format(path: &Path) -> Result<ImageFormat, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("Không thể kiểm tra nội dung ảnh: {error}"))?;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(ImageFormat::Jpeg)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok(ImageFormat::Webp)
    } else {
        Err("Định dạng ảnh không được hỗ trợ. Dùng PNG, JPG, JPEG hoặc WEBP.".into())
    }
}

fn decode_image(path: &Path) -> Result<(u32, u32), String> {
    let output = crate::runtime_paths::ffprobe_command()?
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Không thể khởi chạy FFprobe để kiểm tra logo: {error}"))?;
    if !output.status.success() {
        return Err(
            "FFprobe không thể đọc ảnh logo. Hãy chọn một ảnh PNG, JPG hoặc WEBP hợp lệ.".into(),
        );
    }
    let parsed: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "FFprobe trả về metadata ảnh không hợp lệ.".to_string())?;
    let stream = parsed["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .ok_or_else(|| "Tệp được chọn không có dữ liệu ảnh.".to_string())?;
    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err("Kích thước ảnh logo không hợp lệ hoặc vượt quá 16.384 px.".into());
    }
    let decode = crate::runtime_paths::ffmpeg_command()?
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "null", "-"])
        .output()
        .map_err(|error| format!("Không thể khởi chạy FFmpeg để kiểm tra ảnh: {error}"))?;
    if !decode.status.success() {
        return Err("Ảnh logo bị hỏng hoặc không thể giải mã.".into());
    }
    Ok((width, height))
}

fn default_logo_rect(
    image_width: u32,
    image_height: u32,
    video_width: u32,
    video_height: u32,
) -> (f64, f64, f64, f64) {
    let width = 0.16;
    let image_aspect = image_width.max(1) as f64 / image_height.max(1) as f64;
    let video_aspect = video_width.max(1) as f64 / video_height.max(1) as f64;
    let height = (width * video_aspect / image_aspect).clamp(0.04, 0.3);
    (0.78, 0.06, width.min(1.0 - 0.78), height.min(1.0 - 0.06))
}

fn cleanup_unreferenced_asset(
    store: &ProjectStore,
    project_id: &str,
    project: &Project,
    relative_path: &str,
) -> Option<String> {
    if project
        .logos
        .iter()
        .any(|logo| logo.relative_path == relative_path)
    {
        return None;
    }
    let path = match resolve_project_file(store, project_id, relative_path, "media/overlays") {
        Ok(path) => path,
        Err(error) => {
            return Some(format!(
                "Metadata đã cập nhật nhưng không thể xác minh ảnh logo cũ: {error}"
            ))
        }
    };
    fs::remove_file(path)
        .err()
        .map(|error| format!("Metadata đã cập nhật nhưng chưa thể dọn ảnh logo cũ: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_service::SubtitleSettings;

    #[test]
    fn accepts_project_bounded_rectangles_and_rejects_path_overflow() {
        assert!(validate_rect(0.0, 0.0, 1.0, 1.0).is_ok());
        assert!(validate_rect(0.8, 0.1, 0.3, 0.2).is_err());
        assert!(validate_rect(f64::NAN, 0.0, 0.2, 0.2).is_err());
    }

    #[test]
    fn rejects_invalid_subtitle_style_values() {
        let mut style = SubtitleSettings::default();
        assert!(validate_subtitle_settings(&style).is_ok());
        style.background_opacity = 1.1;
        assert!(validate_subtitle_settings(&style).is_err());
        style = SubtitleSettings::default();
        style.alignment = "justify".into();
        assert!(validate_subtitle_settings(&style).is_err());
    }

    #[test]
    fn recognizes_real_image_signatures_instead_of_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let fake_png = temp.path().join("fake.png");
        fs::write(&fake_png, b"not an image").unwrap();
        assert!(detect_image_format(&fake_png).is_err());
        let webp = temp.path().join("image.bin");
        fs::write(&webp, b"RIFF0000WEBP").unwrap();
        assert_eq!(detect_image_format(&webp).unwrap(), ImageFormat::Webp);
    }

    #[test]
    fn logo_asset_validation_refuses_paths_outside_the_managed_overlay_directory() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Overlay path test").unwrap();
        let root = store.project_directory(&project.id).unwrap();
        let asset = root.join("media").join("overlays").join("brand.png");
        fs::write(&asset, b"placeholder bytes").unwrap();
        let mut logo = ProjectLogo {
            id: Uuid::new_v4().to_string(),
            relative_path: "media/overlays/brand.png".into(),
            original_name: "brand.png".into(),
            ..ProjectLogo::default()
        };
        assert!(validate_logos(&store, &project.id, &[logo.clone()]).is_ok());
        logo.relative_path = "media/overlays/../../outside.png".into();
        assert!(validate_logos(&store, &project.id, &[logo]).is_err());
    }

    #[test]
    fn visual_settings_round_trip_through_the_project_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Overlay persistence").unwrap();
        let mut style = SubtitleSettings::default();
        style.font_size = 54;
        style.background_enabled = true;
        style.background_opacity = 0.72;
        let region = BlurRegion {
            id: Uuid::new_v4().to_string(),
            x: 0.15,
            y: 0.2,
            width: 0.3,
            height: 0.25,
            start_ms: 0,
            end_ms: None,
            enabled: true,
            strength: 28.0,
            effect: Default::default(),
            role: crate::project_service::BlurRegionRole::Generic,
            edge_blend: 0,
            pixel_noise: 0,
            text_removal_scan_fps: 2,
            text_removal_mode: crate::project_service::TextRemovalMode::OcrGuided,
        };
        let saved = save_visual_settings(
            &store,
            &project.id,
            VisualSettings {
                subtitle: style,
                blur_regions: vec![region],
                logos: vec![],
            },
        )
        .unwrap();
        let loaded = store.get(&project.id).unwrap();
        assert_eq!(saved.updated_at, loaded.updated_at);
        assert_eq!(loaded.subtitle.font_size, 54);
        assert_eq!(loaded.subtitle.background_opacity, 0.72);
        assert_eq!(loaded.blur_regions[0].x, 0.15);
    }

    #[test]
    fn imports_and_removes_a_real_logo_inside_an_isolated_project() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.png");
        let generated = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=32x32",
                "-frames:v",
                "1",
            ])
            .arg(&source)
            .status();
        if !generated.is_ok_and(|status| status.success()) {
            // FFmpeg integration coverage is optional on development machines without FFmpeg.
            return;
        }

        let store = ProjectStore::new(temp.path().join("projects"));
        let project = store.create("Logo import integration").unwrap();
        let imported = import_logo(&store, &project.id, source.to_str().unwrap()).unwrap();
        let logo = imported.logo.unwrap();
        assert_eq!(logo.original_name, "source.png");
        assert!(logo.relative_path.starts_with("media/overlays/"));
        let asset = logo_asset_path(&store, &project.id, &logo.id).unwrap();
        assert!(asset.is_file());

        let replacement_source = temp.path().join("replacement.jpg");
        let replacement_generated = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=48x32",
                "-frames:v",
                "1",
            ])
            .arg(&replacement_source)
            .status();
        if !replacement_generated.is_ok_and(|status| status.success()) {
            return;
        }
        let replaced = replace_logo(
            &store,
            &project.id,
            &logo.id,
            replacement_source.to_str().unwrap(),
        )
        .unwrap();
        let replacement = replaced.logo.unwrap();
        assert_eq!(replacement.id, logo.id);
        assert_eq!(replacement.original_name, "replacement.jpg");
        assert_ne!(replacement.relative_path, logo.relative_path);
        assert!(!asset.exists());
        let replacement_asset = logo_asset_path(&store, &project.id, &logo.id).unwrap();
        assert!(replacement_asset.is_file());

        let removed = remove_logo(&store, &project.id, &logo.id).unwrap();
        assert!(removed.logo.is_none());
        assert!(removed.project.logos.is_empty());
        assert!(!replacement_asset.exists());
    }
}
