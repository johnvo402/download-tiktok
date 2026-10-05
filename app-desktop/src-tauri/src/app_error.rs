//! Structured errors for support and new IPC. Existing string commands retain
//! compatibility while their job records expose an error code.
use serde::Serialize;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorContext {
    pub feature: Option<String>,
    pub job_id: Option<String>,
    pub project_id: Option<String>,
    pub backend: Option<String>,
    pub stage: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub technical_details: String,
    pub recoverable: bool,
    pub context: ErrorContext,
}

impl AppError {
    pub fn classify(details: &str, context: ErrorContext) -> Self {
        let lower = details.to_lowercase();
        let (code, message) = if lower.contains("no space left")
            || lower.contains("disk full")
            || lower.contains("os error 112")
            || lower.contains("không đủ dung lượng")
        {
            (
                "DISK_FULL",
                "Ổ đĩa không đủ dung lượng. Hãy giải phóng dung lượng hoặc chọn thư mục khác.",
            )
        } else if lower.contains("permission denied")
            || lower.contains("access is denied")
            || lower.contains("os error 5")
        {
            (
                "ACCESS_DENIED",
                "Không có quyền truy cập dữ liệu. Hãy kiểm tra quyền của thư mục rồi thử lại.",
            )
        } else if lower.contains("mới hơn") {
            (
                "NEWER_SCHEMA",
                "Dữ liệu được tạo bằng phiên bản mới hơn. Hãy cập nhật JC AI.",
            )
        } else if lower.contains("cancel") || lower.contains("đã hủy") {
            ("CANCELLED", "Tác vụ đã được hủy.")
        } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("không tiến thêm") || lower.contains("vượt thời gian") {
            (
                "PROCESS_TIMEOUT",
                "Bộ xử lý không phản hồi kịp. Hãy thử lại hoặc kiểm tra kết nối.",
            )
        } else if lower.contains("worker")
            && (lower.contains("ended") || lower.contains("kết thúc") || lower.contains("crash"))
        {
            (
                "WORKER_CRASHED",
                "Bộ xử lý đã dừng trước khi hoàn tất. Dữ liệu trước đó được giữ nguyên.",
            )
        } else if lower.contains("cuda") || lower.contains("nvenc") || lower.contains("device lost")
        {
            (
                "GPU_UNAVAILABLE",
                "Thiết bị tăng tốc không sẵn sàng. Hãy kiểm tra thiết bị hoặc chọn CPU.",
            )
        } else if lower.contains("json")
            && (lower.contains("invalid")
                || lower.contains("không hợp lệ")
                || lower.contains("bị lỗi"))
        {
            (
                "DATA_CORRUPT",
                "Tệp dữ liệu không hợp lệ. Hãy kiểm tra bản sao lưu hoặc liên hệ hỗ trợ.",
            )
        } else if lower.contains("ffmpeg") {
            (
                "FFMPEG_FAILED",
                "Không xử lý được video. Hãy kiểm tra tệp nguồn và cấu hình xuất.",
            )
        } else {
            (
                "JOB_FAILED",
                "Không hoàn tất được tác vụ. Dữ liệu trước đó được giữ nguyên.",
            )
        };
        let message = if code == "JOB_FAILED" {
            crate::log_service::redact(details)
        } else {
            message.into()
        };
        Self {
            code: code.into(),
            message,
            technical_details: crate::log_service::redact(details),
            recoverable: code != "NEWER_SCHEMA",
            context,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_full_and_permission_have_different_actionable_codes() {
        assert_eq!(
            AppError::classify("No space left on device", ErrorContext::default()).code,
            "DISK_FULL"
        );
        assert_eq!(
            AppError::classify("Access is denied (os error 5)", ErrorContext::default()).code,
            "ACCESS_DENIED"
        );
    }
    #[test]
    fn secrets_are_not_exposed_in_technical_details() {
        let error =
            AppError::classify("Authorization: Bearer supersecret", ErrorContext::default());
        assert!(!error.technical_details.contains("supersecret"));
    }
}
