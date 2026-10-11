# Timeline: điều hướng và track (T04)

Timeline nằm dưới preview và bảng chỉnh sửa của mục Dịch. Kéo thanh phân cách ngang hoặc dùng Arrow Up/Down khi focus thanh này để đổi chiều cao. Giới hạn theo chiều cao cửa sổ để vẫn nhìn thấy preview/panel.

## Điều hướng

- Click/kéo thước hoặc vùng trống trên track để seek theo thời gian dự án.
- Click clip để chọn; double click để về đầu clip. Chọn video lưu nội dung đang sửa trước khi chuyển editor sang nguồn đó. Chọn ảnh/audio chỉ đổi lựa chọn timeline, giữ nguyên editor video.
- Clip đang chọn có viền cyan; clip tại playhead có dấu ▶ và mép dưới vàng. Seek không thay đổi clip đang chỉnh hay transcript của editor.
- Các nút +/− zoom quanh playhead nếu còn trong viewport. Ctrl/Command + cuộn zoom quanh con trỏ. Shift + cuộn để cuộn ngang; scrollbar hỗ trợ cuộn ngang/dọc.
- Nút Fit hoặc F vừa toàn dự án. Khi đang Fit, đổi kích thước cửa sổ/thời lượng tự cập nhật mức zoom.
- Khi focus timeline: Arrow Left/Right seek 1 giây; Shift tăng lên 10 giây; Home/End về đầu/cuối; Space phát/tạm dừng; +/− zoom. Không chặn phím trong input hoặc thao tác Space của button.

Thời gian dùng microsecond nguyên và khoảng nửa mở của T01. Ví dụ A dài 10 giây, B theo sau: seek 12 giây phát nguồn B ở 2 giây mà editor có thể vẫn giữ A. Tại điểm cuối toàn dự án không phát clip vừa kết thúc. Khoảng trống không giữ frame/tiếng nguồn cũ.

## Trạng thái track

Khóa phục vụ thao tác dựng ở T05; vẫn cho phép chọn và seek track bị khóa. Bật/tắt loại track khỏi điều hướng preview. Mute/solo áp dụng cho tiếng gốc của video và audio, không đổi khả năng nhìn thấy lớp hình ảnh. Solo của track bị tắt hoặc track phụ đề không chặn âm thanh track khác. Track bị mute vẫn im lặng khi solo.

`set_timeline_track_control` nhận project ID, track ID, một control `locked | enabled | muted | solo` và giá trị boolean. Backend chỉnh document mới nhất trong khóa transaction, giữ nguyên nội dung/revision từng video và clip. Subtitle track từ chối control âm thanh. Frontend chờ lưu transcript/visual trước, chỉ nhận kết quả còn đúng project và không ghi đè project mới hơn.

## Preview và hiệu năng

T04 thêm preview điều hướng nguồn trực tiếp video/ảnh/audio và đồng hồ timeline cơ bản. Nút “Xem video đang chọn” quay về preview xử lý nguồn của T03, bao gồm transcript/TTS/OCR và đường native hiện có. Seek từ editor hoặc chọn vùng OCR/overlay tự quay về preview nguồn. Preview timeline không xuất video tạm.

Transforms/crop, subtitle/TTS liên kết, trộn audio đầy đủ, đồng bộ decoder/native và export toàn timeline thuộc T06–T09. Chưa thay export cũ bằng export timeline.

Track/clip ngoài viewport không dựng DOM. Clip rất dài chỉ dựng phần nhìn thấy, waveform lấy mẫu theo source in/out và tốc độ; thumbnail dùng cache ảnh nguồn. Yêu cầu cache trùng nguồn/checksum được dùng chung, tối đa hai decoder hoạt động; yêu cầu chưa chạy được hủy khi clip ra ngoài viewport. Cache bộ nhớ giữ tối đa 64 kết quả không được sử dụng; file cache bền vững tiếp tục dùng kho nguồn T02. Canvas bị giới hạn 4 triệu pixel, ruler chỉ dựng tick của vùng nhìn thấy.

`get_media_asset_path` phân giải asset ID từ manifest, xác minh owned path/checksum và cho phép đúng file qua asset protocol. Đường dẫn đang tải được bỏ qua nếu clip/nguồn cũ đã ra khỏi preview. Video/audio bị pause và frame loop được dừng khi đóng preview.
