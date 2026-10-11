# Phụ đề và âm thanh theo clip (T07)

Trong phần Dịch, phụ đề, tiếng gốc và giọng đọc được ánh xạ từ thời gian nguồn sang từng lần dùng video trên timeline. Di chuyển, đổi thứ tự, split và nhân bản không sửa timestamp transcript hoặc tạo lại giọng. Caption chỉ hiện phần giao với khoảng nguồn của clip, dùng interval nửa mở để không xuất hiện thêm ở mép cuối.

## Sử dụng

- Chọn video, thêm **+ Phụ đề** / **+ Giọng** trên toolbar. Giọng ở lớp riêng dùng các WAV đã tạo cho video đó. Nếu chưa có lớp Giọng riêng, video bật lồng tiếng phát giọng trực tiếp từ cấu hình nguồn.
- Chọn clip rồi mở **Âm thanh** trong Properties: âm lượng 0–200%, mute, fade vào/ra, ducking và tốc độ giọng 0.5–2×. Nhập giá trị rồi **Áp dụng âm thanh**; **Hủy chỉnh âm thanh** khôi phục giá trị đã lưu. Nhạc/audio riêng được nhập từ kho Media và dùng cùng các điều khiển.
- Track mute/solo áp dụng cho tiếng gốc, nhạc và giọng; track bị khóa không cho sửa. Ducking giảm tiếng nền theo khoảng speech thực của những giọng đang nghe được, với attack/release và mức giảm tùy chỉnh. Voice track không tự duck chính nó.
- **Tách audio gốc** tạo một clip `sourceAudio` trên audio track, tham chiếu video gốc, và mute tiếng gốc trên clip video để tránh phát hai lần. Clip audio vẫn đi theo video khi dựng. Chọn lớp audio rồi **Tách liên kết audio** để di chuyển/cắt độc lập. Đây là thao tác tường minh, có undo; không tạo file audio mới.
- **Áp dụng tốc độ clip** đổi tốc độ cố định của clip và các lớp liên kết/nhóm, giữ khoảng nguồn. Khi khoảng kéo dài chồng lấn clip khác hoặc lớp nằm trên track khóa, transaction bị từ chối. Tốc độ giọng chỉ thay đổi cách phát WAV; tốc độ clip áp dụng đồng thời hình/phụ đề/tiếng.
- Trim giữa lời, giọng vượt mép hoặc audio cần tạo lại hiển thị cảnh báo trong Properties và liên kết cảnh báo ở preview. Có thể nhập điểm cắt khác, đặt tốc độ giọng phù hợp rồi Áp dụng, hoặc chọn từng đoạn có cảnh báo và **Tạo lại N đoạn này**. Yêu cầu TTS gửi đúng `videoContentId`, `scope: selected` và các ID đoạn chọn; có hủy tác vụ. Giọng mới được dùng chung bởi các bản sao cùng nội dung, được ghi rõ trên giao diện. Không tự xóa lời hay tạo lại toàn bộ video.

Tạo lớp Giọng tắt giọng tự động trên instance video; lớp riêng luôn được ưu tiên để tránh phát trùng, kể cả khi mute/tắt track giọng. Sau khi xóa/tách lớp Giọng, có thể bật lại giọng tự động bằng checkbox trên video khi cần. Các thuộc tính âm thanh, tốc độ và liên kết tham gia lịch sử layout; undo không hoàn tác kết quả transcript/dịch/TTS mới.

## Dữ liệu và preview

`Clip.audio` là tùy chọn, gồm `volume`, `muted`, `fadeInUs`, `fadeOutUs`, `voiceEnabled`, `voiceSpeed`, `duckingEnabled`, `duckingAmount`, `duckAttackUs`, `duckReleaseUs`. Clip cũ không có trường này lấy volume/mute/ducking từ cấu hình nguồn; fade bằng 0, tốc độ giọng bằng 1. Fade dài hơn clip được giới hạn khi tính gain, nên trim vẫn giữ cấu hình đã nhập. Validation TypeScript/Rust thống nhất và chạy trước khi lưu. `sourceAudio` chỉ nằm trên audio track và phải khớp asset/content video.

API `get_timeline_source_media(projectId, videoContentId)` đọc transcript và kiểm tra các WAV sẵn có của nguồn, trả đường dẫn, thời gian file, khoảng speech thực và cảnh báo theo segment. Không chạy TTS. Giữ tương thích WAV cũ qua cache ingest hiện có; không ghi lại tệp gốc. Chỉ cho phép audio đúng chủ sở hữu nội dung, có ngoại lệ legacy cho video được binding. Một file lỗi không chặn các đoạn hợp lệ khác.

Frontend chia sẻ manifest theo project/content/revision, trạng thái transcript và các job kết thúc; kết quả request cũ không ghi đè request mới. Draft lời dịch lỗi thời hoặc xóa segment ngừng phát giọng cũ ngay; chỉnh timestamp giữ file giọng và đổi vị trí theo thời gian nguồn. Caption cũng hiển thị draft đúng video. Manifest của nguồn đang dùng không bị đẩy khỏi cache bởi các nguồn khác trong cùng dự án.

`audio-layout.ts` cung cấp kế hoạch interval, offset, tốc độ và gain thuần để tiếp tục dùng trong T08–T09. Preview hiện phát media trực tiếp qua HTML audio và Web Audio: mỗi region có gain, trộn chung qua compressor giới hạn tín hiệu lớn (threshold −1dB, knee 0, ratio 20, attack 3ms, release 100ms). Video compositor luôn mute để không phát tiếng gốc hai lần. WAV được giới hạn tại mép clip, dùng source offset khi trim/split; không tạo MP4 hoặc file mix cho preview. Pause/seek/đóng preview ngừng và giải phóng audio nodes/context.

T08 đã bổ sung clock chung, prefetch, seek có revision và native/GPU preview; xem [TIMELINE_PREVIEW.md](TIMELINE_PREVIEW.md). T09 tiếp tục export toàn timeline theo cùng quy tắc. Preview nguồn cũ vẫn dùng cho xử lý riêng từng video; export cũ chưa trở thành export nhiều clip trong T07–T08.

## Kiểm tra

- Build frontend; 75 test frontend qua, gồm retime/cut/mép interval, B/A và repeated source, WAV offset, draft/outdated speech, fade/ducking/mute/solo, extraction/detach và validation/khóa/atomic failure.
- 221 test Rust qua, 23 test tùy chọn bỏ qua: WAV thật riêng A/B sau reorder/trim/split/lưu–mở, giữ nguyên byte WAV và transcript nguồn, file hỏng/mất/outdated, chặn audio thuộc video khác, audio optional roundtrip/CAS/undo và từ chối cấu hình sai trước khi ghi.
- UI trình duyệt dùng video có tiếng và WAV thật: phát qua Web Audio với PCM tới gain nodes; giọng/phụ đề B/A và bản sao A; trim/split/rate, cảnh báo, TTS đúng phạm vi, mute/solo/ducking, volume/fade, extraction/detach/undo và inspector 360px. Kiểm tra hồi quy UI T05/T06 qua. Đây là kiểm tra browser compositor; ngưỡng đồng bộ native/export thuộc T08–T11.
