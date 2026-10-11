# Kế hoạch timeline nhiều lớp trong phần Dịch

Trạng thái: T01–T11 hoàn tất triển khai và kiểm tra cục bộ. Cập nhật: 2026-10-12. CI GitHub chạy khi push commit.

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

**Hoàn tất ngày 2026-10-11.** Chọn clip để sửa/nhận dạng/dịch/TTS đúng nội dung; ngôn ngữ, prompt/context, giọng và OCR riêng; scope một/các video chọn/tất cả, tiến độ/hủy riêng. Sidecar/cache/khóa theo ID nội dung, merge snapshot có kiểm tra revision và baseline segment. Mô tả API và kiểm tra tại [VIDEO_CONTENT_PROCESSING.md](VIDEO_CONTENT_PROCESSING.md).

Chuyển API đọc/ghi, ASR/OCR, dịch, TTS, sự kiện tiến độ, khóa tác vụ và cache từ chỉ `projectId` sang định danh nội dung video. Bảng bên phải dùng clip đang chọn để tìm đúng nguồn/nội dung. Có phạm vi chạy một video, các video được chọn hoặc toàn bộ; báo lỗi và hủy riêng.

Kết quả tác vụ phải gắn với ID và revision khi bắt đầu. Đổi clip đang chọn hoặc đổi thứ tự trong lúc chạy không được ghi kết quả sang video khác. Sửa lời dịch chỉ đánh dấu audio liên quan cần tạo lại; di chuyển clip không làm cache mất hiệu lực.

Hoàn tất khi: A/B có ngôn ngữ, transcript, bản dịch và giọng riêng; đổi thứ tự hoặc cắt clip vẫn giữ đúng dữ liệu. Phụ thuộc T01–T02.

Đã kiểm tra: build frontend; 45 test frontend; 210 test Rust qua, 23 test tùy chọn bỏ qua. Test WAV thật kiểm tra TTS riêng theo nguồn và từ chối giọng đã đổi; test UI kiểm tra lưu trước đổi video, kết quả A khi đang chọn B, shared clip, ngôn ngữ/TTS riêng, scope deduplicate, hủy riêng, preview nguồn/OCR B và panel hẹp. Preview nguồn bổ sung phục vụ xử lý từng video; phát nối tiếp/composite và export toàn timeline vẫn thuộc T08–T09.

## T04 — Giao diện timeline và điều hướng thời gian

Timeline ở dưới preview/bảng chỉnh sửa, có kéo đổi chiều cao, thước thời gian, playhead, tên track, thumbnail, waveform, chọn clip, zoom quanh con trỏ/playhead, cuộn ngang/dọc và fit toàn bộ dự án. Thanh công cụ dùng cùng phong cách ứng dụng hiện tại, với phím tắt và nhãn truy cập.

Phân biệt clip được chọn với clip đang phát. Track có khóa, bật/tắt hiển thị, mute/solo phù hợp loại track. Khu vực cuộn chỉ dựng các clip/thumbnail đang nhìn thấy để giữ hiệu năng.

Hoàn tất khi: hiển thị và seek được timeline hai video, một ảnh và một audio; thao tác tốt ở các mức zoom và chiều rộng cửa sổ. Phụ thuộc T01–T02.

Đã triển khai: timeline dưới preview/panel, resize chiều cao, ruler/playhead, thumbnail/waveform theo phần nguồn, chọn clip độc lập với thời gian đang xem, zoom quanh con trỏ/playhead, fit và cuộn ngang/dọc. Track có khóa, bật/tắt và mute/solo cho visual/audio; các thay đổi được lưu bằng transaction trên project mới nhất. Clip/track ngoài viewport không dựng DOM; cache dùng chung theo nguồn, tối đa hai decoder, hủy yêu cầu còn chờ khi ra ngoài viewport.

Đã kiểm tra: build frontend, 50 test frontend, 212 test Rust qua và 23 test tùy chọn bỏ qua. UI kiểm tra A/B + ảnh + audio, ánh xạ seek sang B khi editor vẫn là A, lựa chọn ảnh giữ nguyên editor video, trạng thái track, keyboard/zoom/resize, dự án 500 clip và timeline hẹp. Preview điều hướng dùng nguồn trực tiếp và tiếng gốc/audio riêng, không tạo video tạm. Transforms, subtitle/TTS trên timeline, đồng bộ/native preview hoàn chỉnh và export timeline tiếp tục ở T06–T09; preview nguồn cũ vẫn truy cập qua “Xem video đang chọn”. Chi tiết: `TIMELINE_NAVIGATION.md`.

## T05 — Bộ thao tác dựng và lịch sử chỉnh sửa

Thêm, di chuyển, đổi track/thứ tự, kéo hai mép trim, split tại playhead, nhân bản, copy/paste, chọn nhiều, nhóm/liên kết, xóa thường và xóa rồi dồn khoảng trống. Snap với mép clip/playhead và có bật/tắt. Clip cùng track không chồng lấn vô tình; lớp chồng nhau dùng track riêng, chuyển cảnh dùng quy tắc riêng.

Triển khai theo command/transaction để kéo một lần là một bước undo. Undo/redo bao gồm thay đổi của các clip liên kết. Autosave sau thao tác hợp lệ; chặn trim vượt nguồn, thời lượng âm và thao tác trên track khóa.

Hoàn tất khi: cut/reorder/ripple/undo/redo cho nhiều clip giữ nguyên ánh xạ thời gian và các nội dung liên kết. Phụ thuộc T01, T04.

Đã triển khai: kéo clip giữa track và kéo hai mép trim với ghost, hít mép/playhead có bật/tắt và Alt bỏ hít, chọn nhiều, cut tại playhead, đổi thứ tự clip/track, nhân bản, copy/paste, nhóm/liên kết, thêm track/clip, xóa thường và ripple toàn bộ track. Có hộp nhập thời gian/track đích cho thao tác không dùng kéo. Mỗi gesture hợp lệ là một transaction autosave và một bước undo; lịch sử giữ 50 bước theo dự án, không hoàn tác dữ liệu transcript/dịch/TTS mới. Backend so sánh bố cục, kiểm tra khóa, nguồn, tham chiếu và chồng lấn trên document mới nhất. Xóa clip vẫn giữ nguồn/content trong kho; binding clip null không tái tạo clip đã xóa.

Đã kiểm tra: build frontend, 62 test frontend, 215 test Rust qua và 23 test tùy chọn bỏ qua; test Rust bao gồm CAS, overlap/bounds/locks, xóa binding, thêm đầu tiên rồi undo/redo và giữ cập nhật nội dung độc lập. UI kiểm tra gesture là một bước undo, trim/cut lớp phụ thuộc, reorder, paste lên track trống, nhóm/link, xóa/ripple, undo giữ kết quả AI, khóa, hàng rào lưu, AI cập nhật giữa lúc kéo và thêm media đúng giờ dự án khi xem nguồn B. Kiểm tra hồi quy T04 qua cả điều hướng, track controls, resize và virtualization. Chi tiết: `TIMELINE_EDITING.md`.

## T06 — Nhiều lớp và thuộc tính hình ảnh

Track video, ảnh, chữ/logo và phụ đề; video có thể nằm trên video khác để làm hình trong hình. Kéo lớp chọn thời điểm bắt đầu/kết thúc. Điều chỉnh vị trí, kích thước, xoay, lật, opacity, crop và fit/fill; phân biệt tỷ lệ khung hình dự án với crop của từng clip.

Khung hình dự án hỗ trợ 16:9, 9:16, 1:1 và kích thước tùy chỉnh. Vùng OCR/xóa chữ vẫn gắn với tọa độ nguồn, được biến đổi đúng khi clip bị crop/scale. Caption liên kết video có vị trí/style có thể chỉnh.

Hoàn tất khi: ảnh xuất hiện đúng khoảng thời gian trên video; thay đổi thứ tự lớp và tỷ lệ cho kết quả đúng trong preview. Phụ thuộc T04–T05 và preview T08 được phát triển cùng.

Đã triển khai: bảng Properties cho transform/crop từng clip và khung hình 16:9, 9:16, 1:1/tùy chỉnh; nhập giá trị để xem thử, Áp dụng lưu một bước undo và Hủy xem thử. Preview trình duyệt ghép video/ảnh/chữ/phụ đề theo order track, hỗ trợ picture-in-picture, vị trí tâm, kích thước, xoay, lật, opacity và fit/fill/stretch. Thêm chữ và phụ đề liên kết bằng toolbar; ảnh/logo dùng kho nguồn. Typography là thuộc tính riêng của clip, không sửa transcript/bản dịch dùng chung. Vùng OCR/blur/AI Remove hiển thị hướng dẫn trong tọa độ nguồn, cùng đi qua crop/transform của video; chức năng xử lý vùng vẫn ở editor nguồn.

Đã kiểm tra: build frontend, 67 test frontend, 217 test Rust qua và 23 test tùy chọn bỏ qua. UI với video/ảnh thật kiểm tra PiP, thứ tự track, ảnh/chữ theo khoảng thời gian, caption theo nguồn A, crop/fill/lật/OCR, draft không ghi dữ liệu, khung hình/transform undo/redo, giữ cập nhật AI, khóa và inspector 360px. Hồi quy UI T05 qua. Đồng bộ audio/native preview, hiệu ứng xử lý vùng trên compositor và export toàn timeline tiếp tục ở T07–T09. Chi tiết: [TIMELINE_VISUALS.md](TIMELINE_VISUALS.md).

## T07 — Phụ đề và audio đi theo clip

**Hoàn tất ngày 2026-10-11.** Preview ánh xạ phụ đề/tiếng gốc/TTS theo khoảng nguồn và tốc độ của từng clip; trộn nhạc qua Web Audio với âm lượng, mute/solo, fade và ducking. Có lớp Giọng, tách audio gốc và tách liên kết tường minh; thuộc tính và tốc độ cố định có autosave/undo. Cảnh báo trim giữa lời/giọng vượt mép kèm chỉnh điểm cắt, tốc độ giọng hoặc tạo lại chỉ các segment chọn; giữ nguyên file nguồn và timestamp transcript.

Ánh xạ subtitle, tiếng gốc và TTS từ nguồn vào từng clip, bao gồm cắt đầu/cuối, split, nhân bản, đổi tốc độ cố định và khoảng trống. Trộn thêm nhạc/audio riêng; volume, mute/solo, fade và ducking. Có thể tách liên kết audio một cách tường minh.

Mặc định giữ nguyên audio nguồn/TTS đã tạo và giới hạn phần phát theo clip. Nếu trim cắt giữa lời hoặc TTS vượt khoảng cho phép, hiển thị cảnh báo cùng lựa chọn điều chỉnh điểm cắt, tốc độ hoặc tạo lại giọng cho phần được chọn; không âm thầm xóa lời hay tạo lại toàn bộ. Caption giao mép clip được giới hạn khoảng hiển thị, không sửa timestamp gốc.

Hoàn tất khi: A/B đổi vị trí, trim hoặc split thì tiếng và chữ vẫn đúng; một nguồn xuất hiện hai lần phát đúng ở cả hai vị trí. Phụ thuộc T03, T05.

Đã kiểm tra: build frontend, 75 test frontend và 221 test Rust qua, 23 test tùy chọn bỏ qua. Test Rust với WAV thật kiểm tra ánh xạ độc lập A/B, reorder/trim/split/lưu–mở giữ nguyên byte WAV và thời gian nguồn, file hỏng/mất/outdated, chặn audio từ video khác, audio optional/CAS/undo/validation. UI với video có tiếng/WAV thật qua B/A/repeated A, offset trim/split, retime liên kết, PCM Web Audio, mute/solo/fade/ducking, cảnh báo/TTS đúng phạm vi, extraction/detach/undo và inspector 360px; hồi quy UI T05/T06 qua. Clock/prefetch/native preview và export toàn timeline tiếp tục T08–T09. Chi tiết: [TIMELINE_AUDIO.md](TIMELINE_AUDIO.md).

## T08 — Preview nhiều nguồn và đồng bộ phát

**Hoàn tất ngày 2026-10-11.** Một đồng hồ timeline dùng mốc thời gian tuyệt đối, dừng chờ tất cả nguồn đang cần và đi qua từng biên clip/audio. Seek có revision riêng, ngắt tiếng và ẩn frame cũ ngay, chỉ nhận frame của seek mới; kéo playhead giữ ý định phát và tiếp tục khi thả. Có bước tiến/lùi theo FPS đầu ra, prefetch 1.5 giây với giới hạn player, giải phóng media/gain/context khi rời clip hoặc đóng preview. Scene plan chung mô tả lớp, thời gian nguồn, caption và audio để T09 xây export từ snapshot.

Một đồng hồ timeline điều khiển video, audio, subtitle và overlay. Chuyển nguồn ở biên clip, prefetch clip sắp phát, xử lý seek liên tục, frame stepping, khoảng trống và video chồng lớp. Dùng cùng mô tả cảnh với export, quản lý cache và giải phóng decoder/frame/audio cũ khi nguồn đổi.

Tiếp tục hỗ trợ đường preview native/GPU và phương án dự phòng phù hợp. Hủy seek cũ, tránh hiện frame hoặc tiếng của clip trước sau khi người dùng kéo playhead sang nguồn khác.

Hoàn tất khi: phát và seek qua A/B ổn định, không lẫn frame/tiếng/phụ đề; chỉnh ảnh/chữ không tạo video đầu ra. Phụ thuộc T01–T04, tích hợp dần T05–T07.

Đã kiểm tra: build frontend; 85 test frontend; 223 test Rust qua, 24 test tùy chọn bỏ qua trong suite thường. Test GPU tùy chọn mới chạy riêng thành công trên Intel Iris Xe với nguồn A/B thật, surface ẩn, seek/cancel và fallback decoder CPU. UI media thật qua clock chung, prefetch im lặng, biên A/B, clip lặp, khoảng trống, ảnh, hai video chồng lớp, seek liên tục/chờ nguồn, frame stepping, giải phóng player và lỗi/thử lại; hồi quy T05–T07 qua. Protocol native kiểm tra mở theo clip A/B, bỏ phiên mở/seek cũ, lấy mẫu clock timeline và chuyển compositor khi có caption/dialog. Native tăng tốc cảnh một video nguyên khung; cảnh nhiều lớp dùng compositor trình duyệt. Chi tiết: [TIMELINE_PREVIEW.md](TIMELINE_PREVIEW.md).

## T09 — Export toàn bộ timeline

**Hoàn tất ngày 2026-10-12.** Export mặc định timeline, có toàn bộ/vùng thời gian/các clip chọn, độ phân giải, FPS phân số, H.264/HEVC và encoder. Plan chụp document/transcript/TTS cùng media trước khi đăng ký job; chỉnh dự án/relink/tạo lại giọng không đổi snapshot đang chạy. FFmpeg ghép nhiều nguồn/lớp/transform/caption/khoảng trống; audio trộn theo khối nhỏ với trim/rate/voice speed/mute/solo/fade/ducking. Có validation nguồn thiếu, tiến độ/hủy, kiểm tra file cuối và commit không ghi đè. Chi tiết và phạm vi xử lý vùng nguồn tại [TIMELINE_EXPORT.md](TIMELINE_EXPORT.md).

Xây kế hoạch render từ snapshot dự án: nhiều nguồn, in/out, tốc độ, thứ tự lớp, khoảng trống, canvas, caption và audio mix. Chuẩn hóa nguồn khác codec/FPS/kích thước; xuất toàn bộ hoặc vùng được chọn; cấu hình codec, độ phân giải và FPS. Hỗ trợ tiến độ, hủy, kiểm tra file cuối và lỗi tài nguyên thiếu.

Hoàn tất khi: file xuất khớp các mốc hình/tiếng/phụ đề trong preview; hủy không để file cuối hỏng; thay đổi project trong lúc export không làm lệch snapshot. Phụ thuộc T06–T08.

Đã kiểm tra: build frontend; 87 test frontend; 227 test Rust qua, 26 test tùy chọn bỏ qua trong suite thường. Hai test media export chạy riêng với H.264/HEVC, nhiều FPS/kích thước, trim/rate/ảnh xoay/caption/tiếng đánh dấu/khoảng trống và xuất vùng HEVC 29.97 FPS; xác minh pixel/PCM, snapshot sau sửa/xóa nguồn/tạo lại WAV, voice speed/duck/fade, hủy và nguồn thiếu. UI kiểm tra default timeline, vùng/resolution/FPS/payload, tiến độ/hủy/khóa thiết lập, lỗi vùng và chế độ export nguồn cũ. Compositor giữ phạm vi preview T06/T08: OCR/blur/STTN là hướng dẫn nguồn; xử lý vùng thực hiện qua export nguồn trước khi dựng. Đo dự án dài và phục hồi sau app crash tiếp tục ở T11.

## T10 — Công cụ dựng bổ sung

**Hoàn tất ngày 2026-10-12.** Chuyển cảnh hòa tan/quét ngang có overlap chủ đích và crossfade tiếng, fade hình, keyframe vị trí/scale/opacity theo clock nguồn; UI bản nháp/Áp dụng, dấu trên clip và menu chuột phải/Shift+F10/K. Tốc độ cố định/fade tiếng từ T07 tiếp tục hoạt động. TS/Rust validation chặn nguồn vượt giới hạn, tham chiếu sai/chồng ba clip/track khóa; preview và export áp dụng cùng clock, kể cả xuất vùng. Hiệu ứng lưu theo clip trong transaction có undo/redo, không sửa transcript/TTS nguồn. Chi tiết quy tắc chỉnh sửa tại [TIMELINE_EFFECTS.md](TIMELINE_EFFECTS.md).

Chuyển cảnh cơ bản, fade hình/tiếng, tốc độ phát cố định, keyframe vị trí/scale/opacity và thao tác bằng phím tắt/context menu. Quy định rõ thời lượng chuyển cảnh và phần nguồn có sẵn; kiểm thử retime với phụ đề/TTS. Kiến trúc dành chỗ cho speed ramp nhưng cần task riêng nếu đưa vào phạm vi, do ánh xạ thời gian phi tuyến.

Hoàn tất khi: mỗi hiệu ứng triển khai có cùng kết quả preview/export, có undo/redo và được lưu trong project. Phụ thuộc T05–T09.

Đã kiểm tra: build frontend; 93 test frontend; 230 test Rust qua, 27 test tùy chọn bỏ qua. Shared fixture kiểm tra motion/fade; test lưu/mở/undo/redo và validation nguyên tử; ba test media export chạy riêng qua, gồm dissolve/wipe/audio crossfade/motion/fade/xuất vùng cùng hồi quy T09. UI kiểm tra bản nháp, hai keyframe, fade/motion trong preview, lưu/undo/redo, chuyển cảnh cùng phụ đề liên kết, gỡ phục hồi vị trí, menu/bàn phím/focus, khóa và panel hẹp. Speed ramp nằm ngoài phạm vi T10.

## T11 — Kiểm thử tích hợp và ổn định

**Hoàn tất ngày 2026-10-12.** Chỉ mục interval cho preview/editor/audio, dọn cache và undo khi đóng dự án, chặn manifest muộn sau đóng/mở lại; export chia vùng theo một lưới frame và tối đa 32 input hình mỗi FFmpeg, master audio liên tục; sửa điểm cắt VFR giữ frame trước PTS kế tiếp. Snapshot export gắn journal để recovery dọn partial thuộc project và giữ retry payload. Chi tiết gate, benchmark và phạm vi tại [TIMELINE_STABILITY.md](TIMELINE_STABILITY.md).

Kiểm thử xuyên suốt migration, lưu/mở, kéo/cắt/undo, đổi lựa chọn khi AI đang chạy, hủy/resume tác vụ, thiếu nguồn, Unicode path, nguồn VFR, nhiều FPS và nhiều audio stream. Dùng media ngắn có hình/tiếng đánh dấu để đo sai lệch preview/export; ngưỡng hình theo một frame đầu ra, tránh cộng dồn sai số trên timeline dài.

Đo độ phản hồi và bộ nhớ khi mở dự án 30–90 phút với nhiều clip/lớp; giải phóng cache, decoder và tác vụ khi đóng dự án. Kiểm tra tính năng Dịch cũ, batch và Tạo video AI không bị ảnh hưởng.

Hoàn tất khi: kịch bản chấp nhận bên dưới chạy qua, test frontend/Rust/Python liên quan và CI thành công. Task này được làm xuyên suốt, với một lượt kiểm tra tổng thể sau T09–T10.

Đã kiểm tra: build production; 100 test frontend, 234 test Rust (30 tùy chọn), 5 test công cụ Node, 86 test worker Python (10 tùy chọn) và 4 test cache qua. Năm media acceptance FFmpeg/FFprobe và UI T05–T11 qua; native hidden GPU chạy riêng. UI 90 phút/2.700 clip mở khoảng 368ms, query 1.000 scene khoảng 33ms, seek nguồn khoảng 83–108ms trên fixture máy dev. Export đủ 90 phút/900 clip ở 64×36/2FPS đúng 10.800 frame; benchmark không đại diện export 1080p. Test crash/recovery giữ layout/effects/nội dung nguồn và job retry, không xóa media/final. Workflow Windows đã bổ sung media acceptance, UI T11 và artifact số đo; kết quả GitHub cho commit này chỉ có sau push.

## Các mốc triển khai

1. Nền tảng chạy được: T01–T04 và phần preview cơ bản của T08. Nhập A/B, chọn từng clip, nhận dạng/dịch/TTS riêng, phát nối tiếp và đổi thứ tự không lẫn nội dung.
2. Dựng và xuất được: T05–T09. Cắt, trim, nhiều lớp, tỷ lệ, audio liên kết, undo/redo và export khớp preview.
3. Hoàn thiện công cụ: T10 và hoàn tất T11. Keyframe/chuyển cảnh, tối ưu dự án dài, kiểm tra hồi quy và phát hành.

## Kịch bản chấp nhận chung

Nhập video A và B; tạo transcript, dịch và giọng riêng; đưa B lên trước A; cắt A thành hai đoạn và kéo đoạn sau sang vị trí khác. Thêm ảnh chỉ hiện trong giây 10–20, chữ ở track trên, nhạc ở track dưới; đổi canvas sang 9:16 và crop từng clip. Phát/seek để kiểm tra chữ và tiếng đi theo video, undo/redo, lưu rồi mở lại; cuối cùng xuất MP4 và đối chiếu với preview. Toàn bộ thao tác giữ nguyên tệp nguồn.
