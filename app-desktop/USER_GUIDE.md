# JC AI — Hướng dẫn người dùng

## Cài đặt và dữ liệu

Cài file `JC-AI-*-windows-x64-setup.exe` dưới tài khoản Windows hiện tại. Ứng dụng đóng gói FFmpeg/FFprobe và AI Worker; không cần Node.js, Rust, Python hoặc FFmpeg riêng.

Ở bước **Nơi lưu dữ liệu ứng dụng**, chọn thư mục có quyền ghi và đủ dung lượng,
ví dụ `D:\JC AI\Data`. Mặc định khi cài mới là `Documents\JC AI`. Dự án,
settings, logs, models, runtime AI, cache và hồ sơ WebView đều nằm trong thư mục
này. Thư mục cài file ứng dụng được chọn ở bước riêng; nơi lưu dữ liệu phải nằm
ngoài thư mục cài ứng dụng. Export vẫn dùng nơi xuất do bạn chọn.

Installer nhớ lựa chọn khi nâng cấp/cài lại. Với bản cũ, mặc định chọn thư mục
AppData đang có dữ liệu để giữ truy cập project. Nếu chọn thư mục khác, dữ liệu
cũ được giữ nguyên và không tự di chuyển; chọn lại thư mục cũ để dùng tiếp.
Đóng app trước khi sao chép dữ liệu sang ổ khác. Gỡ ứng dụng mặc định giữ dữ liệu.
Nếu ổ dữ liệu không có hoặc không ghi được, app báo lỗi để bạn khôi phục ổ hoặc
chạy lại installer chọn nơi lưu phù hợp.

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

## Tạo video AI từ ý tưởng

1. Chọn **Tạo video AI** ở Project Manager, hoặc mở **Tạo video AI** trong dự án.
2. Nhập chủ đề, chọn Tiếng Việt / English và thời lượng từ **2 phút**. Codex đưa đúng **5 ý tưởng**, đánh sao một ý tưởng được đề xuất, kèm mô tả, câu mở đầu và lý do chọn.
3. Chọn **Thực hiện ý tưởng** để chuyển sang màn hình biên tập trong cùng cửa sổ. Sửa tiêu đề, mô tả, ngôn ngữ, phong cách ảnh và giọng đọc.
4. Chọn **Lập dàn ý**, sửa tên / nội dung chương rồi **Duyệt dàn ý**. Chọn **Viết các chương còn thiếu**; mỗi chương được lưu ngay sau khi viết xong. Để bỏ một chương, chọn chương đó, bấm **Xóa chương** trong **Dàn ý & kịch bản** rồi xác nhận. Các cảnh của chương được bỏ khỏi kịch bản; cần duyệt lại dàn ý sau khi xóa.
5. Chọn từng cảnh, sửa lời kể và prompt ảnh. Chọn **Sao chép prompt**, dán vào Codex desktop để tạo ảnh, lưu ảnh rồi chọn **Nhập ảnh từ Codex**. Hỗ trợ PNG, JPG và WebP; ảnh nhập được sao chép vào dự án.
6. Tạo giọng cho một cảnh có ảnh, nghe / xem thử rồi **Duyệt cảnh mẫu**. Chọn **Tạo / tiếp tục giọng đọc còn thiếu** để xử lý các cảnh còn lại. Cần cài bộ xử lý và tải model VieNeu-TTS trong **Cài đặt → Voice / TTS**.
7. Preview phát ảnh và audio trực tiếp, có thể phát một chương hoặc toàn video và tua theo thời gian. Kéo phụ đề, logo, text để đổi vị trí hoặc dùng thanh chỉnh trong bảng bên phải. Có thể sửa chữ, thời điểm phụ đề, màu, cỡ chữ, thời gian xuất hiện logo / text và bật zoom nhẹ.
8. Sửa một cảnh và tạo lại riêng giọng / ảnh của cảnh đó nếu cần. Có thể khôi phục tối đa 5 phiên bản ảnh / giọng trước. Đổi lời kể, ngôn ngữ hoặc giọng làm audio cũ hết hiệu lực; cần tạo lại trước khi xuất.
9. **Lưu video** chỉ bật khi mọi chương có cảnh, mọi cảnh có ảnh / giọng phù hợp và tổng thời lượng lời kể thực tế đạt mục tiêu đã chọn. Nếu chưa đủ mục tiêu, thêm chương / cảnh hoặc bổ sung lời kể rồi tạo giọng lại. Ứng dụng không kéo dài audio bằng khoảng lặng để đủ thời lượng.

Preview không tạo MP4. Chỉ **Lưu video** mới xuất video 1080p, H.264 + AAC vào thư mục `exports` của dự án, kèm bản nháp JSON đúng phiên bản dùng để xuất. Mỗi lần xuất tạo tệp mới. Bản nháp tự lưu, có bản dự phòng và mở lại ở màn hình Tạo video AI. Bấm **Dừng** rồi tiếp tục để xử lý phần còn thiếu.

Codex CLI cần được cài / đăng nhập và cấu hình đường dẫn, model, timeout tại **Cài đặt → Translation → Codex CLI**. Tạo ý tưởng / kịch bản dùng tài khoản Codex hiện tại. Phiên bản này nhập ảnh được tạo trong Codex desktop; chưa tự gọi công cụ tạo ảnh qua CLI. Thời điểm phụ đề được tính theo từng đoạn lời kể do TTS tạo, không căn chỉnh từng từ. Với nội dung lịch sử, kiểm tra các dữ kiện được đánh dấu chưa chắc chắn trước khi xuất.

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
