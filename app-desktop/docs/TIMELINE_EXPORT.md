# Export timeline nhiều nguồn (T09)

Hoàn tất ngày 2026-10-12. Hộp Export mặc định xuất timeline đã dựng; chế độ
“Video nguồn” giữ đường export nguồn hiện có. Preview không tạo MP4: chỉ thao tác
Export mới chạy renderer và lưu video hoàn chỉnh.

## Sử dụng

- Xuất toàn bộ timeline hoặc nhập điểm bắt đầu/kết thúc tính bằng giây. “Lấy vùng
  các clip đã chọn” lấy khoảng từ đầu clip sớm nhất đến cuối clip muộn nhất,
  bao gồm khoảng trống giữa chúng và các lớp khác giao với vùng đó.
- Chọn H.264/HEVC, encoder, chất lượng, bitrate, độ phân giải và FPS. Mặc định
  dùng khung hình/FPS của dự án; giữ FPS phân số như 30000/1001 và 60000/1001.
- Độ phân giải đầu ra phải chẵn, mỗi chiều 2–8192 pixel; FPS 1–120. Vùng xuất
  phải nằm trong timeline và dài ít nhất một frame đầu ra. Đổi tỷ lệ độ phân
  giải đầu ra kéo giãn toàn cảnh; chỉnh canvas trong timeline để bố cục lại.
- Âm thanh dùng các thuộc tính clip/track đã dựng: âm lượng, mute/solo, fade,
  giọng đọc và ducking. Các tùy chọn âm thanh toàn nguồn chỉ hiện ở chế độ
  “Video nguồn”.
- Có trạng thái chuẩn bị snapshot, tiến độ trộn tiếng/dựng/encode/xác minh,
  hủy khi job đã được đăng ký, lịch sử và mở thư mục đầu ra. Thiết lập bị khóa
  trong lúc chuẩn bị/chạy để tránh gửi nhiều lần hoặc sửa nhầm cấu hình job.

## API và snapshot

`ExportOptions.timeline` là tùy chọn mới, dùng các command validate/start/cancel
và sự kiện/history export hiện có. Khi không có trường này, export nguồn và
batch tiếp tục dùng renderer cũ.

```json
{
  "timeline": {
    "width": 1920,
    "height": 1080,
    "frameRate": { "numerator": 30000, "denominator": 1001 },
    "startUs": null,
    "endUs": null
  }
}
```

Hai điểm `null` nghĩa là toàn bộ dự án; vùng xuất cần cả hai timestamp nguyên
microsecond tuyệt đối. Timeline bắt đầu từ 0 trong file đầu ra. Không áp giới
hạn 2 phút của tính năng Tạo video AI.

`timeline_export::build` kiểm tra document và tài nguyên đang cần trong vùng
xuất, chụp transcript/TTS theo ID nội dung, áp dụng cùng quy tắc thời gian/lớp/
caption/audio của `time.ts`, `scene-plan.ts` và `audio-layout.ts`. Thời điểm nguồn
được tính từ in/out và tốc độ phân số, interval nửa mở `[start, end)`.

`capture` giữ document/sidecar trong bộ nhớ và tạo thư mục tạm dưới
`cache/render`. Media thuộc kho nguồn được kiểm tra kích thước/checksum và giữ
bằng hard link, hoặc sao chép nếu filesystem không hỗ trợ. Các WAV TTS có thể
được tạo lại nên luôn sao chép. Renderer chỉ đọc snapshot, không đọc lại project
hay transcript. Đổi bố cục/nội dung trong lúc chuẩn bị làm yêu cầu bị từ chối;
sau khi snapshot hoàn tất, chỉnh project, relink hoặc tạo lại TTS không đổi job.

Nguồn hình/tiếng đang dùng bị thiếu/hỏng sẽ báo tên nguồn và chặn export. Nguồn
không dùng trong vùng không chặn. Giọng mất/lỗi/hết hiệu lực được cảnh báo và bỏ
qua như preview; không tự tạo lại giọng hoặc lấy giọng của video khác.

## Hình ảnh và phụ đề

FFmpeg ghép canvas có màu nền, các video/ảnh/chữ/phụ đề của track đang bật theo
thứ tự lớp. Giữ trim/rate, khoảng trống, crop, contain/cover/stretch, vị trí tâm,
scale, xoay, lật và opacity. Nguồn khác codec/kích thước/FPS được chuẩn hóa trước
khi ghép; toàn cảnh được scale sang độ phân giải đầu ra.

Chữ Unicode được rasterize bằng font hệ thống, có wrapping, màu, nền, alignment
và style riêng của clip. Caption lấy bản dịch, fallback nội dung nguồn; đoạn
giao nhau được nối như preview và giới hạn theo in/out/rate/vùng xuất. PNG/concat
dùng timebase microsecond; caption trống có frame trong suốt để không giữ chữ cũ.
Rasterizer hệ thống và trình duyệt có thể khác nhẹ ở nét chữ/metrics font.

OCR/blur/AI Remove vẫn là hướng dẫn trong compositor timeline theo T06/T08.
Export timeline không chạy STTN/blur hoặc các logo/chữ global của editor nguồn.
Để xử lý vùng nguồn, xuất bằng “Video nguồn” rồi nhập kết quả vào kho; ảnh/logo/
chữ cần nằm trên timeline để xuất cùng các lớp. Compositor dùng CPU FFmpeg;
encoder phần cứng được chọn qua hệ thống capability/fallback hiện có.

## Âm thanh và tài nguyên

Trộn stereo 48 kHz theo từng khối 1024 sample. Decoder chỉ tồn tại khi region
đang cần, đọc qua queue hữu hạn và đóng sau mép region; clip nằm xa không tạo
buffer im lặng dài trong RAM. Master WAV tạm chứa khoảng trống thật trên đĩa.
At tempo giữ cao độ khi đổi tốc độ clip/giọng; mono được nhân sang hai kênh
với gain tương đương Web Audio. Fade theo toàn clip vẫn giữ đúng khi xuất vùng.

Ducking dùng các khoảng speech của giọng nghe được, kể cả attack/release từ
đoạn ngay ngoài vùng xuất. Speech chồng nhau được hợp nhất; đoạn giọng liên kết
tường minh thay thế giọng tự động của video theo quy tắc preview. Volume được
cộng trực tiếp và chỉ clamp peak vượt full scale. Đầu ra có AAC khi vùng có
audio nghe được; vùng chỉ có khoảng trống/hình không tạo stream tiếng giả.

Kiểm tra dung lượng trước sao chép/trộn/render. Master WAV có giới hạn 4 GiB;
timeline có tiếng dài hơn giới hạn này cần xuất thành các vùng ngắn hơn. Canvas
hoặc footprint lớp vượt 32 triệu pixel bị từ chối để tránh allocation quá lớn.

## Hủy và xuất bản file

Child FFmpeg nằm trong process containment; stdout/stderr có bộ đệm hữu hạn.
Hủy đóng decoder, kill/wait child, thu hồi reader và giải phóng file tạm. Job
không tiến triển trong 10 phút báo lỗi. Auto chỉ thử encoder dự phòng khi lỗi
khởi tạo encoder xảy ra trước frame đầu tiên.

Renderer ghi `.partial.mp4` trước, xác minh codec, độ phân giải, thời lượng,
stream tiếng và FPS bằng ffprobe; sai lệch thời lượng video tối đa một frame
đầu ra cộng 2 ms. Sau `sync_all`, commit nguyên tử theo trạng thái job và
`persist_noclobber` sang tên cuối duy nhất. Hủy trước commit hoặc lỗi không
để file cuối hỏng và không ghi đè video có sẵn. Snapshot/master/partial được
thu hồi khi kết thúc bình thường; kiểm tra phục hồi sau app crash và đo dự án
30–90 phút thuộc lượt ổn định T11.

## Kiểm tra

- `npm run build`; `npm test`: 87 test frontend qua.
- `cargo test --lib`: 227 test Rust qua, 26 test media/phần cứng/benchmark tùy
  chọn bỏ qua trong suite thường.
- `cargo test --lib timeline_export -- --ignored --nocapture`: hai test media
  thật qua với FFmpeg/ffprobe trên PATH. H.264 24 FPS và HEVC 30 FPS khác kích
  thước; trim/rate/layer/caption/gap; xuất HEVC 640×360 ở 30000/1001 theo vùng;
  lấy mẫu pixel/PCM/tần số để kiểm tra hình/tiếng. Snapshot vẫn đúng sau xóa
  media gốc, đổi vị trí clip và ghi lại WAV; hủy thu hồi partial; nguồn thiếu
  bị chặn; voice speed giữ cao độ; duck/fade và vùng im lặng được kiểm tra.
- Kiểm tra UI bằng Playwright với mock command/events: mặc định timeline,
  cấu hình vùng/độ phân giải/FPS, payload, tiến độ/hủy/khóa thiết lập, vùng lỗi
  chặn nút xuất, chuyển sang export nguồn và giữ cảnh báo subtitle region.

Các module chính: `timeline_export.rs` (plan/snapshot), `graph.rs` (hình/chữ),
`audio.rs` (streaming mix), `runner.rs` (FFmpeg/verify/commit),
`TimelineExportSettings.tsx` và `ExportDialog.tsx` (UI).
