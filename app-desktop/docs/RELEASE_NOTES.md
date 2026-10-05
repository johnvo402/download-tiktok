# JC AI cho Windows

## 0.1.14-beta.1 — phục hồi dữ liệu và tác vụ

- Giữ ba bản sao dữ liệu gần nhất, phục hồi bản chép lời bị hỏng và thông báo khi đã phục hồi.
- Lưu thay đổi trước khi đóng ứng dụng; nếu chưa lưu được, giữ cửa sổ và nội dung đang sửa.
- Tác vụ dở dang được đánh dấu sau khi mở lại; chỉ chạy lại khi bạn chọn thử lại.
- Kiểm tra video xuất trước khi hoàn tất, giữ nguyên tệp đã xuất và video nguồn.
- Có thể tìm lại video nguồn bị mất nếu tệp được chọn giống bản gốc.
- Thêm kiểm tra và sửa chữa các thành phần xử lý tùy chọn, cùng báo cáo chẩn đoán đã lọc thông tin riêng tư.

Đây là bản beta thử nghiệm, chưa xác nhận đầy đủ trên NVIDIA/AMD và máy Windows mới. Bộ cài thử nghiệm chưa ký số. Ứng dụng chưa tự cập nhật.

## Bắt đầu

1. Tải tệp `JC-AI-...-windows-x64-setup.exe` và mở để cài đặt.
2. Mở JC AI và tạo hoặc mở dự án.
3. Vào **Cài đặt → Nhận dạng lời thoại** hoặc **Giọng đọc** để tải mô hình bạn muốn dùng. Ứng dụng hiển thị dung lượng trước khi tải.

Ứng dụng đã kèm các thành phần cần cho tính năng cơ bản. Không cần cài thêm công cụ xử lý video. Nếu máy thiếu thành phần hiển thị video của Windows, bộ cài sẽ tải khi cần.

Trong mỗi mục nhận dạng, đọc chữ và giọng đọc, **Thiết bị** chỉ có CPU và NVIDIA. Chọn NVIDIA sẽ mở xác nhận, rồi cài bộ xử lý CUDA cùng model đang chọn trong một lượt và tự lưu lựa chọn khi hoàn tất. WhisperX và tách thoại gốc có nút cài ngay trong thẻ tính năng; WhisperX cần chọn ngôn ngữ để lấy đúng model căn thời gian. Thành phần chung và mô hình đã có được dùng lại; chỉ phần còn thiếu mới tải xuống.

Dự án và mô hình được lưu trong thư mục dữ liệu JC AI, tách khỏi thư mục cài đặt. Có thể xem dung lượng và mở các thư mục này tại **Cài đặt → Nâng cao**.

Ứng dụng hiện chưa tự cập nhật; hãy tải và cài phiên bản mới khi có bản phát hành.
