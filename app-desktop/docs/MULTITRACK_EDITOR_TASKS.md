# Kế hoạch timeline nhiều lớp trong phần Dịch

Trạng thái: T01–T02 hoàn tất; T03–T11 chưa triển khai. Cập nhật: 2026-10-11.

## Mục tiêu và bố cục

Trong màn hình Dịch, bổ sung timeline phía dưới khu vực preview và bảng chỉnh sửa. Timeline có thước thời gian, playhead, thumbnail video, waveform, nhiều track, zoom và thanh cuộn. Người dùng thêm nhiều video, ảnh, chữ và audio; kéo clip đến thời điểm mong muốn, cắt, kéo mép chỉnh thời lượng, đổi thứ tự, xóa và chỉnh khung hình.

Phía trên vẫn có preview và bảng thuộc tính/transcript. Kho nguồn chứa các video, ảnh và audio đã nhập. Chọn clip video nào thì bảng Dịch hiện dữ liệu của video đó; phát qua video khác không tự đổi đối tượng đang chỉnh sửa. Có thể thu gọn hoặc đổi chiều cao timeline.

## Các nguyên tắc dữ liệu

- Mỗi video nguồn có ID, metadata, transcript, bản dịch, cấu hình giọng đọc, audio TTS và trạng thái tác vụ riêng.
- Clip là một lần sử dụng nguồn trên timeline, có ID riêng, track, vị trí bắt đầu, điểm vào/ra nguồn, tốc độ và biến đổi hình ảnh. Cắt hoặc nhân bản clip không nhân bản tệp video.
- Transcript lưu theo thời gian video nguồn. Timeline tính vị trí hiển thị từ điểm cắt, tốc độ và vị trí clip. Di chuyển clip không ghi lại timestamp nguồn, không phải nhận dạng hay tạo giọng lại.
- Ở tốc độ cố định: thời gian nguồn = điểm vào nguồn + (thời gian timeline - vị trí clip) × tốc độ. Dùng miền thời gian nguyên và quy tắc làm tròn thống nhất giữa TypeScript/Rust; hỗ trợ nguồn VFR và FPS đầu ra khác FPS nguồn.
- Các đoạn cắt từ cùng một nguồn mặc định dùng chung transcript/bản dịch nguồn. Nếu cần sửa độc lập một bản sao, cung cấp thao tác tạo bản nội dung riêng; không âm thầm sửa tất cả bản sao.
- Phụ đề và lồng tiếng liên kết với clip video mặc định. Ảnh/chữ/nhạc chèn thêm mặc định nằm ở thời gian timeline; có thể nhóm/liên kết để kéo theo video.
- Mọi chỉnh sửa cắt ghép đều không phá hủy nguồn. Xóa clip khỏi timeline không xóa tệp nguồn hoặc kết quả AI còn được sử dụng.
- Preview phát từ các nguồn và trạng thái timeline; không xuất MP4 hoàn chỉnh mỗi lần chỉnh sửa. Proxy tương thích codec, thumbnail và waveform có thể được tạo và cache riêng.
- Preview và export dùng chung quy tắc thời gian, lớp, biến đổi hình ảnh và trộn âm thanh.
- Thời lượng timeline được suy ra từ các clip; không áp giới hạn tối thiểu 2 phút của tính năng Tạo video AI cho phần Dịch.

## Căn cứ từ code hiện tại

- Trước T01, `src/features/projects/types.ts` và `src-tauri/src/project_service.rs`: Project chỉ có một `source`, schema 11. T01 thêm `timeline` ở schema 12 và giữ `source` làm dữ liệu tương thích cho các chức năng hiện tại.
- `src/app/project-state.ts`: một `currentTranscript` và một đồng hồ phát của dự án.
- `src-tauri/src/transcript_service.rs`: transcript hiện lưu ở `subtitles/transcript.json` theo project.
- `src/features/workspace/translation/TranslationView.tsx`: player lấy thời lượng và nguồn từ `project.source`.
- `src-tauri/src/preview/scene.rs`, `src-tauri/src/render_spec.rs`, `src-tauri/src/render_service.rs`: preview/export hiện xây dựng cảnh từ một nguồn.
- `src-tauri/src/audio/timeline.rs`: đã có xử lý audio TTS theo segment, có thể tái sử dụng sau khi bổ sung ánh xạ theo clip.

## T01 — Dữ liệu timeline và chuyển đổi dự án cũ

**Hoàn tất ngày 2026-10-11.** Project schema 12, timeline schema 1; thời gian nguyên microsecond, FPS/tốc độ dạng phân số; đồng bộ tạm với chức năng một video hiện tại. Mô tả cấu trúc và quy tắc tại [TIMELINE_SCHEMA.md](TIMELINE_SCHEMA.md).

Định nghĩa MediaAsset, VideoContent, Track, Clip, TimelineSettings và ID liên kết. MediaAsset nhận diện tệp; VideoContent chứa transcript/bản dịch/TTS; Clip tham chiếu hai phần này. Thống nhất thời gian nguồn và timeline, interval nửa mở, FPS, tốc độ, thứ tự lớp, phiên bản dữ liệu và validation.

Chuyển dự án cũ thành một video/clip bắt đầu tại 0, giữ lại transcript, TTS, vùng OCR/blur, logo và chữ. Lưu nguyên tử, sao lưu trước migration và phục hồi nếu gián đoạn. Luồng dự án trống/Tạo video AI vẫn phải hoạt động.

Hoàn tất khi: mở lại dự án cũ cho kết quả tương đương; lưu/mở dữ liệu mới không mất ID, thời gian hay asset.

Đã kiểm tra: build frontend; 39 test frontend; 5 test công cụ CI; 200 test Rust qua, 23 test phần cứng/benchmark tùy chọn được bỏ qua. Có kiểm thử migration giữ transcript/TTS/OCR/blur/logo/chữ và tệp gốc, ID ổn định, nhiều nguồn/lớp, liên kết giọng/phụ đề, sửa mâu thuẫn, dữ liệu lỗi, phục hồi và từ chối schema mới hơn. Giao diện timeline và preview nhiều nguồn thuộc các task sau.

## T02 — Kho nguồn và nhập nhiều media

**Hoàn tất ngày 2026-10-11.** Tab Media có kho nguồn, nhập nhiều tệp/kéo thả, tìm kiếm/lọc, trạng thái nguồn, thumbnail và waveform theo nhu cầu; thêm nhiều clip từ cùng nguồn; tìm lại nguồn bằng checksum. Mô tả API và lưu trữ tại [MEDIA_LIBRARY.md](MEDIA_LIBRARY.md).

Nhập nhiều video, ảnh, audio bằng hộp chọn hoặc kéo thả; đọc metadata, tạo thumbnail/waveform theo nhu cầu; phát hiện nguồn trùng, nguồn lỗi hoặc thiếu. Đặt lại đường dẫn nguồn bằng kiểm tra định danh, tránh gắn transcript của video khác. Cache và file nằm trong cấu trúc theo asset.

Hoàn tất khi: nhập A và B, thêm nhiều lần lên timeline, mở lại project và liên kết lại nguồn bị di chuyển thành công. Phụ thuộc T01.

Đã kiểm tra bằng media thật: nhập hỗn hợp và lỗi từng tệp; không sao chép nguồn trùng; A/B/A lưu–mở giữ ID/nội dung; ảnh/audio vào track phù hợp; tìm lại tệp bị thiếu hoặc hỏng vẫn đọc được transcript/TTS; thumbnail/waveform được tái sử dụng và tạo lại khi cache lỗi. Kiểm thử trình duyệt cho tìm kiếm Unicode, lọc, thêm clip, sóng âm, relink, chỉ thử lại tệp lỗi và panel hẹp. Build frontend thành công; 41 test frontend và 203 test Rust qua, 23 test tùy chọn bỏ qua.

## T03 — Transcript, dịch và lồng tiếng theo từng video

Chuyển API đọc/ghi, ASR/OCR, dịch, TTS, sự kiện tiến độ, khóa tác vụ và cache từ chỉ `projectId` sang định danh nội dung video. Bảng bên phải dùng clip đang chọn để tìm đúng nguồn/nội dung. Có phạm vi chạy một video, các video được chọn hoặc toàn bộ; báo lỗi và hủy riêng.

Kết quả tác vụ phải gắn với ID và revision khi bắt đầu. Đổi clip đang chọn hoặc đổi thứ tự trong lúc chạy không được ghi kết quả sang video khác. Sửa lời dịch chỉ đánh dấu audio liên quan cần tạo lại; di chuyển clip không làm cache mất hiệu lực.

Hoàn tất khi: A/B có ngôn ngữ, transcript, bản dịch và giọng riêng; đổi thứ tự hoặc cắt clip vẫn giữ đúng dữ liệu. Phụ thuộc T01–T02.

## T04 — Giao diện timeline và điều hướng thời gian

Timeline ở dưới preview/bảng chỉnh sửa, có kéo đổi chiều cao, thước thời gian, playhead, tên track, thumbnail, waveform, chọn clip, zoom quanh con trỏ/playhead, cuộn ngang/dọc và fit toàn bộ dự án. Thanh công cụ dùng cùng phong cách ứng dụng hiện tại, với phím tắt và nhãn truy cập.

Phân biệt clip được chọn với clip đang phát. Track có khóa, bật/tắt hiển thị, mute/solo phù hợp loại track. Khu vực cuộn chỉ dựng các clip/thumbnail đang nhìn thấy để giữ hiệu năng.

Hoàn tất khi: hiển thị và seek được timeline hai video, một ảnh và một audio; thao tác tốt ở các mức zoom và chiều rộng cửa sổ. Phụ thuộc T01–T02.

## T05 — Bộ thao tác dựng và lịch sử chỉnh sửa

Thêm, di chuyển, đổi track/thứ tự, kéo hai mép trim, split tại playhead, nhân bản, copy/paste, chọn nhiều, nhóm/liên kết, xóa thường và xóa rồi dồn khoảng trống. Snap với mép clip/playhead và có bật/tắt. Clip cùng track không chồng lấn vô tình; lớp chồng nhau dùng track riêng, chuyển cảnh dùng quy tắc riêng.

Triển khai theo command/transaction để kéo một lần là một bước undo. Undo/redo bao gồm thay đổi của các clip liên kết. Autosave sau thao tác hợp lệ; chặn trim vượt nguồn, thời lượng âm và thao tác trên track khóa.

Hoàn tất khi: cut/reorder/ripple/undo/redo cho nhiều clip giữ nguyên ánh xạ thời gian và các nội dung liên kết. Phụ thuộc T01, T04.

## T06 — Nhiều lớp và thuộc tính hình ảnh

Track video, ảnh, chữ/logo và phụ đề; video có thể nằm trên video khác để làm hình trong hình. Kéo lớp chọn thời điểm bắt đầu/kết thúc. Điều chỉnh vị trí, kích thước, xoay, lật, opacity, crop và fit/fill; phân biệt tỷ lệ khung hình dự án với crop của từng clip.

Khung hình dự án hỗ trợ 16:9, 9:16, 1:1 và kích thước tùy chỉnh. Vùng OCR/xóa chữ vẫn gắn với tọa độ nguồn, được biến đổi đúng khi clip bị crop/scale. Caption liên kết video có vị trí/style có thể chỉnh.

Hoàn tất khi: ảnh xuất hiện đúng khoảng thời gian trên video; thay đổi thứ tự lớp và tỷ lệ cho kết quả đúng trong preview. Phụ thuộc T04–T05 và preview T08 được phát triển cùng.

## T07 — Phụ đề và audio đi theo clip

Ánh xạ subtitle, tiếng gốc và TTS từ nguồn vào từng clip, bao gồm cắt đầu/cuối, split, nhân bản, đổi tốc độ cố định và khoảng trống. Trộn thêm nhạc/audio riêng; volume, mute/solo, fade và ducking. Có thể tách liên kết audio một cách tường minh.

Mặc định giữ nguyên audio nguồn/TTS đã tạo và giới hạn phần phát theo clip. Nếu trim cắt giữa lời hoặc TTS vượt khoảng cho phép, hiển thị cảnh báo cùng lựa chọn điều chỉnh điểm cắt, tốc độ hoặc tạo lại giọng cho phần được chọn; không âm thầm xóa lời hay tạo lại toàn bộ. Caption giao mép clip được giới hạn khoảng hiển thị, không sửa timestamp gốc.

Hoàn tất khi: A/B đổi vị trí, trim hoặc split thì tiếng và chữ vẫn đúng; một nguồn xuất hiện hai lần phát đúng ở cả hai vị trí. Phụ thuộc T03, T05.

## T08 — Preview nhiều nguồn và đồng bộ phát

Một đồng hồ timeline điều khiển video, audio, subtitle và overlay. Chuyển nguồn ở biên clip, prefetch clip sắp phát, xử lý seek liên tục, frame stepping, khoảng trống và video chồng lớp. Dùng cùng mô tả cảnh với export, quản lý cache và giải phóng decoder/frame/audio cũ khi nguồn đổi.

Tiếp tục hỗ trợ đường preview native/GPU và phương án dự phòng phù hợp. Hủy seek cũ, tránh hiện frame hoặc tiếng của clip trước sau khi người dùng kéo playhead sang nguồn khác.

Hoàn tất khi: phát và seek qua A/B ổn định, không lẫn frame/tiếng/phụ đề; chỉnh ảnh/chữ không tạo video đầu ra. Phụ thuộc T01–T04, tích hợp dần T05–T07.

## T09 — Export toàn bộ timeline

Xây kế hoạch render từ snapshot dự án: nhiều nguồn, in/out, tốc độ, thứ tự lớp, khoảng trống, canvas, caption và audio mix. Chuẩn hóa nguồn khác codec/FPS/kích thước; xuất toàn bộ hoặc vùng được chọn; cấu hình codec, độ phân giải và FPS. Hỗ trợ tiến độ, hủy, kiểm tra file cuối và lỗi tài nguyên thiếu.

Hoàn tất khi: file xuất khớp các mốc hình/tiếng/phụ đề trong preview; hủy không để file cuối hỏng; thay đổi project trong lúc export không làm lệch snapshot. Phụ thuộc T06–T08.

## T10 — Công cụ dựng bổ sung

Chuyển cảnh cơ bản, fade hình/tiếng, tốc độ phát cố định, keyframe vị trí/scale/opacity và thao tác bằng phím tắt/context menu. Quy định rõ thời lượng chuyển cảnh và phần nguồn có sẵn; kiểm thử retime với phụ đề/TTS. Kiến trúc dành chỗ cho speed ramp nhưng cần task riêng nếu đưa vào phạm vi, do ánh xạ thời gian phi tuyến.

Hoàn tất khi: mỗi hiệu ứng triển khai có cùng kết quả preview/export, có undo/redo và được lưu trong project. Phụ thuộc T05–T09.

## T11 — Kiểm thử tích hợp và ổn định

Kiểm thử xuyên suốt migration, lưu/mở, kéo/cắt/undo, đổi lựa chọn khi AI đang chạy, hủy/resume tác vụ, thiếu nguồn, Unicode path, nguồn VFR, nhiều FPS và nhiều audio stream. Dùng media ngắn có hình/tiếng đánh dấu để đo sai lệch preview/export; ngưỡng hình theo một frame đầu ra, tránh cộng dồn sai số trên timeline dài.

Đo độ phản hồi và bộ nhớ khi mở dự án 30–90 phút với nhiều clip/lớp; giải phóng cache, decoder và tác vụ khi đóng dự án. Kiểm tra tính năng Dịch cũ, batch và Tạo video AI không bị ảnh hưởng.

Hoàn tất khi: kịch bản chấp nhận bên dưới chạy qua, test frontend/Rust/Python liên quan và CI thành công. Task này được làm xuyên suốt, với một lượt kiểm tra tổng thể sau T09–T10.

## Các mốc triển khai

1. Nền tảng chạy được: T01–T04 và phần preview cơ bản của T08. Nhập A/B, chọn từng clip, nhận dạng/dịch/TTS riêng, phát nối tiếp và đổi thứ tự không lẫn nội dung.
2. Dựng và xuất được: T05–T09. Cắt, trim, nhiều lớp, tỷ lệ, audio liên kết, undo/redo và export khớp preview.
3. Hoàn thiện công cụ: T10 và hoàn tất T11. Keyframe/chuyển cảnh, tối ưu dự án dài, kiểm tra hồi quy và phát hành.

## Kịch bản chấp nhận chung

Nhập video A và B; tạo transcript, dịch và giọng riêng; đưa B lên trước A; cắt A thành hai đoạn và kéo đoạn sau sang vị trí khác. Thêm ảnh chỉ hiện trong giây 10–20, chữ ở track trên, nhạc ở track dưới; đổi canvas sang 9:16 và crop từng clip. Phát/seek để kiểm tra chữ và tiếng đi theo video, undo/redo, lưu rồi mở lại; cuối cùng xuất MP4 và đối chiếu với preview. Toàn bộ thao tác giữ nguyên tệp nguồn.
