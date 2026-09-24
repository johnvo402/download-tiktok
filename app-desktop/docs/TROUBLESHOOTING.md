# Troubleshooting

| Vấn đề | Việc cần thử |
|---|---|
| Thiếu FFmpeg/FFprobe hoặc AI Worker | Settings → Advanced → Runtime Health; repair/reinstall. Installer đã đóng gói runtime. |
| Model download lỗi | Kiểm tra kết nối, proxy và dung lượng; thử tải lại trong Settings. Download dở không được đánh dấu đã cài. |
| CUDA unavailable | Chọn Auto hoặc CPU. Core release CPU-first, không cần cài CUDA toolkit. |
| Hết VRAM hoặc CPU chậm | Chọn model Whisper nhỏ hơn hoặc dùng CPU; đóng ứng dụng nặng khác. |
| API key/quota lỗi | Cập nhật credential trong Translation settings, chạy Test Connection và kiểm tra quota/provider status. |
| FFmpeg export thất bại | Kiểm tra quyền ghi/dung lượng, input streams và validation; nguồn video vẫn được giữ nguyên. |
| Phụ đề không vừa blur | Chọn đúng Subtitle Blur; tăng vùng/padding hoặc chỉnh bản dịch/font. Export validator liệt kê các segment tràn. |
| Chưa chọn Subtitle Blur | Trong Properties, đặt role vùng che thành Subtitle Blur. Generic blur không nhận translated text. |
| Corrupt project/settings | Settings được backup trước reset; project được backup trước migration. Khôi phục backup khi app đã đóng. |
| SmartScreen cảnh báo | Installer chưa ký. Chỉ chạy từ nguồn tin cậy và đối chiếu SHA-256. |

Export diagnostics từ Settings → Advanced khi cần báo lỗi. Gói đã lọc và loại media/transcript/credentials, nhưng hãy tự xem lại trước khi gửi.
