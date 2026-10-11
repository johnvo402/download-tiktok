# Hình ảnh nhiều lớp (T06)

Chọn clip trên timeline và bấm **Hình ảnh / khung hình**, hoặc mở tab **Properties**. Bảng chỉnh clip đầu tiên trong lựa chọn. Vị trí X/Y là tâm clip, tính theo phần trăm canvas; kích thước X/Y là scale tương đối với canvas. Có xoay, lật ngang/dọc, opacity, Fit/Fill/Kéo giãn và crop X/Y/rộng/cao theo phần trăm nguồn. Track khóa chỉ cho xem thuộc tính.

Nhập giá trị hợp lệ hiển thị ngay bản xem thử trên timeline preview, dừng phát và không ghi project. **Áp dụng clip** / **Áp dụng khung hình** lưu qua transaction và tạo một bước undo; **Hủy xem thử** trả về dữ liệu đã lưu. Đổi clip hoặc rời Properties bỏ bản xem thử chưa áp dụng. Không tạo MP4 tạm để xem thay đổi.

Khung hình dự án có preset 1920×1080, 1080×1920, 1080×1080, kích thước tùy chỉnh (1–16384 pixel mỗi chiều) và màu nền. Thay đổi khung hình không sửa điểm vào/ra, thời lượng hay timestamp nguồn. Khung hình đã tùy chỉnh trước khi thêm video đầu tiên được giữ. Lịch sử T05 nay bao gồm `TimelineSettings` cùng clips/tracks/binding; assets và dữ liệu nội dung video vẫn nằm ngoài undo/redo.

Thêm video/ảnh từ kho nguồn; các lớp chồng thời gian dùng track riêng. Giảm kích thước video và chỉnh X/Y để làm picture-in-picture. Logo mới dùng ảnh PNG/WebP có alpha, được trim/move/xóa như clip ảnh. **+ Chữ** tạo clip 5 giây tại playhead, sau đó chỉnh nội dung/font/cỡ/độ đậm/màu/nền/căn lề. **+ Phụ đề** tạo clip liên kết video được chọn, sao chép source window/rate và lấy transcript của đúng video. Typography và vị trí caption chỉ áp dụng cho instance đó; nội dung transcript/bản dịch vẫn dùng chung nguồn. Caption chưa có override kế thừa typography từ video.

Compositor trình duyệt ghép các track bật từ order nhỏ đến lớn, nguồn nằm dưới và lớp cao hơn phủ lên trên. Dùng interval nửa mở nên ảnh/chữ biến mất đúng tại cuối clip. Có thể click hình/chữ đang hiện để chọn clip, hoặc chọn qua timeline bằng bàn phím. Kích thước font tính theo pixel khung hình đầu ra rồi thu theo kích thước preview.

Crop được áp dụng trên mặt phẳng đầy đủ của nguồn trước fit vào canvas; mọi tọa độ OCR/blur/xóa chữ vẫn chuẩn hóa theo nguồn gốc. Tùy chọn **Vùng OCR/xóa chữ** hiển thị đường hướng dẫn qua cùng crop, scale, rotate và flip; không viết lại vùng khi đổi canvas hoặc transform. Chỉnh vùng/AI Remove và các overlay nguồn cũ vẫn dùng **Xem video đang chọn** cùng các công cụ nguồn hiện có. Những overlay nguồn cũ không tự chuyển thành clip mới hoặc nhân bản tài nguyên.

`TimelineClip.style` là trường tùy chọn, chỉ dùng cho `text`/`subtitles`: font family/size/weight, text/background colors, background opacity và alignment. Document cũ thiếu trường này vẫn đọc được; copy/split/undo/redo giữ style. Backend xác thực kiểu chữ, crop/scale, kích thước canvas và track khóa trước lưu; lệnh chỉ thay layout trên project mới nhất để giữ nội dung AI vừa hoàn tất.

T06 dùng video/image DOM để xem ghép lớp. Audio TTS/mix, bộ giải mã/native/GPU và xử lý blur/STTN trên compositor tiếp tục ở T07–T08; export nhiều lớp theo snapshot ở T09. Vùng hướng dẫn là vị trí xử lý, không phải kết quả AI Remove.

Kiểm tra: 67 test frontend; 217 test Rust qua, 23 test tùy chọn bỏ qua; build production; UI với A xanh/B đỏ, ảnh, chữ và caption A, khung hình dọc, crop, xoay, lật, thứ tự lớp, undo/redo, cập nhật AI, khóa, panel 360px và hồi quy T05.
