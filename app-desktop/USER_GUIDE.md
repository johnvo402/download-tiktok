# JC AI — Hướng dẫn người dùng

## Cài đặt và dữ liệu

Cài file `JC-AI-*-windows-x64-setup.exe` dưới tài khoản Windows hiện tại. Ứng dụng đóng gói FFmpeg/FFprobe và AI Worker; không cần Node.js, Rust, Python hoặc FFmpeg riêng. Dự án, settings, logs, models và exports lưu ngoài thư mục cài ứng dụng trong AppData. Gỡ ứng dụng mặc định giữ nguyên dữ liệu.

## Bắt đầu dự án

1. Chọn **Tạo dự án mới**, nhập tên và tạo project.
2. Mở **Dịch**, chọn Browse Files hoặc kéo video vào preview.
3. Trong Media, chọn text subtitle track để extract; nếu video không có subtitle phù hợp, vào **Cài đặt → Speech Recognition**, tải model faster-whisper rồi tạo transcript.
4. Chọn provider trong **Cài đặt → Translation**, lưu credential bằng OS credential manager và chạy Test Connection.
5. Dịch segment, sửa Original/Translation và đợi autosave.
6. Trong **Voice / TTS**, tải VieNeu model nếu cần và tạo audio cho segment đã dịch.
7. Tạo vùng blur che hard subtitle, đặt role **Subtitle Blur**, bật translated subtitle và thêm logo nếu cần.
8. Mở Export, kiểm tra warnings, chọn thư mục đích và export.

Translated subtitle trong video cuối luôn nằm trong vùng blur có role Subtitle Blur. Nếu subtitle tắt, blur vẫn có thể được render nhưng không có text dịch.

## Models, storage và offline

Model weights không đi kèm installer. Mỗi download yêu cầu xác nhận; model cache dùng chung giữa các projects. Settings → Speech Recognition / Voice TTS cho phép gỡ model đã cài. Không thể gỡ model khi job tương ứng chạy; gỡ model không xóa project hoặc audio đã tạo.

Settings → Advanced → Storage hiển thị Models, Projects, Project cache, Temporary files và Logs. Dọn Temporary không xóa media, transcript, TTS output hoặc export.

Sau khi models cần thiết được cài, subtitle extraction, local ASR, VieNeu inference và FFmpeg export có thể chạy offline. Gemini/Google Cloud yêu cầu internet; Codex CLI cần cài và đăng nhập theo cấu hình người dùng.

## Chẩn đoán nhanh

- Thiếu FFmpeg/worker: Settings → Advanced → Runtime Health; repair hoặc reinstall ứng dụng.
- Model tải lỗi: kiểm tra Internet/dung lượng rồi retry từ Settings.
- CUDA unavailable: chọn Auto/CPU. Bản release là CPU-first.
- API key lỗi: cập nhật credential và chạy Test Connection.
- Subtitle không vừa blur: chọn đúng Subtitle Blur, tăng kích thước vùng hoặc sửa bản dịch/cỡ chữ.
- Project/settings lỗi: giữ backup migration và không sửa khi ứng dụng đang chạy.
- Để liên hệ hỗ trợ, tạo Settings → Advanced → Export diagnostics; kiểm tra bundle trước khi chia sẻ.
