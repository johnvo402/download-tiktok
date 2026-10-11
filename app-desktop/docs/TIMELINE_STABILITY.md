# T11 — Kiểm thử tích hợp và ổn định timeline

Hoàn tất triển khai và gate cục bộ ngày 2026-10-12. GitHub Actions chạy sau khi commit được push; không coi kết quả cục bộ là một lần chạy CI trên GitHub.

## Thay đổi từ lượt kiểm tra tổng thể

- Preview/editor tạo chỉ mục interval khi document hoặc audio manifest thay đổi. Seek/frame tick tìm interval đang giao và mốc kế tiếp bằng cây/binary search, giữ khoảng nửa mở và lớp kéo dài qua nhiều clip. Viewport lọc clip một lần cho các track đang dựng; speculative player vẫn giới hạn sáu visual/tám audio, không bỏ lớp đang hoạt động.
- Thumbnail/waveform bỏ cache và yêu cầu còn chờ khi rời dự án. Tối đa hai yêu cầu backend đã chạy được phép hoàn tất; kết quả không được giữ lại. Manifest loại entry của dự án không còn consumer, chặn response cũ bằng identity của request cả khi đóng/mở lại cùng ID/revision. Decoder, frame callback, Web Audio node/context và RAF vẫn được dispose theo T08. Đóng/đổi dự án giải phóng undo/clipboard; đổi tab trong cùng dự án giữ lịch sử.
- Timeline có hơn 32 visual clip trong vùng export được encode theo các vùng tối đa khoảng 30 giây, tự chia nhỏ để mỗi FFmpeg mở tối đa 32 input visual. Biên vùng nằm trên một lưới frame đầu ra chung; không cộng dồn thời lượng làm tròn từng clip. Video encode một lần, ghép packet để giữ chất lượng; master audio trộn liên tục và AAC encode một lần để tránh seam giữa các vùng. Cấu hình codec/encoder/quality được giữ. Tiến độ và hủy hoạt động trong từng vùng; kiểm tra cuối/commit không ghi đè giữ nguyên T09.
- Export VFR giữ preroll từ keyframe và PTS có dấu sau seek, rồi resample CFR bằng `fps:start_time=0:round=up`. Không reset frame đầu được decode thành thời điểm 0 trước khi chuẩn hóa FPS: điểm cắt nằm giữa hai frame phải giữ frame trước đó đến PTS kế tiếp. Kiểm thử pixel bắt được lỗi frame tương lai xuất hiện sớm và đã qua sau sửa. Video có nhiều audio stream được probe một lần/asset để export/waveform lấy stream mặc định như Chromium/WebView; fallback stream đầu nếu không có cờ default. Voice WAV và các decoder cũ giữ cách ánh xạ riêng.
- Snapshot timeline gắn với journal UUID của job; manifest ghi thư mục snapshot và file output tạm. Khi restart, job đang chạy thành `interrupted`, giữ payload để người dùng thử lại. Startup recovery dọn snapshot/partial trong project của job gián đoạn, giữ media/TTS/final output và thư mục output ngoài project. Retry chụp document hiện tại, không hứa tiếp tục encode từ frame đã chạy trước crash.

## Bằng chứng

| Kiểm tra | Kết quả |
|---|---|
| Build production | Qua |
| Frontend | 100 test qua / 22 file |
| Rust suite thường | 234 qua, 30 test media/GPU/benchmark tùy chọn |
| Công cụ release/CI | 5 test Node qua |
| Worker Python | 86 qua, 10 test phần cứng/model tùy chọn; 4 test cache công cụ qua |
| Media acceptance | 5 test FFmpeg/FFprobe qua: T09/T10, Unicode/VFR/hai audio stream, batching/cancellation |
| Native GPU | Hidden surface A/B, seek/cancel và fallback CPU được chạy riêng |
| UI hồi quy | T05–T10 qua với media thật; AI update/selection, undo, ảnh/chữ/crop/canvas, voice/fade/duck, transport, export, keyframe/chuyển cảnh |
| UI T11 | Dự án 30/60/90 phút, rapid seek, phát qua A/B, pixel/caption, nguồn thiếu/thử lại, response muộn, đóng/mở và panel hẹp |
| Phục hồi | Timeline 900 clip/90 phút cùng motion/canvas/settings nguồn phục hồi từ backup; journal export gián đoạn giữ payload retry và không xóa nguồn/final |

### Đo UI trên máy dev

Chromium headless, video H.264 320×180 có âm thanh, hai nguồn lặp lại với rate 2/3, mỗi video có caption/audio liên kết. Mỗi lần đo seek chờ toàn bộ participant sẵn sàng; heap lấy sau GC. Không tính đây là benchmark decoder 4K hoặc bộ nhớ toàn process/GPU. Fixture còn giữ document và module JavaScript khi đóng để tiếp tục kiểm tra.

| Timeline | Clip | Mở + ready | 1.000 scene query | Seek nguồn/đầu-cuối | JS heap mở / đóng |
|---|---:|---:|---:|---:|---:|
| 30 phút | 900 | 158 ms | 14,8 ms | 83–108 ms | 16,3 / 15,5 MiB |
| 60 phút | 1.800 | 300 ms | 24,1 ms | 83–116 ms | 18,1 / 15,7 MiB |
| 90 phút | 2.700 | 368 ms | 33,0 ms | 83–108 ms | 23,2 / 19,7 MiB |

Viewport zoom 10 lần dựng dưới 100 clip, active + speculative giữ ≤7 video/≤9 audio trong fixture; đóng dự án không còn DOM editor/player. Sau ba lần mở/đóng, heap tăng dưới gate 16 MiB; response manifest từ lần mở trước không thay nội dung của lần mở mới. Unit clock kiểm tra 324.000 tick/90 phút và 161.838 frame step ở 30000/1001 không cộng dồn sai số.

### Export dài

Benchmark riêng xuất đủ **90 phút / 900 video clip** ở **64×36, 2 FPS**, kiểm đếm đúng **10.800 frame**, sample pixel đầu/30/60/90 phút và giữ byte nguồn. Lượt đo trước khi bổ sung tiến độ trong vùng: 39,68 giây; sampled working set của process Rust 17,3 → 19,1 MiB, không bao gồm FFmpeg child. Đây là kiểm tra độ dài/bounds/timestamp, không phải tốc độ export 1080p.

Media acceptance ngắn kiểm tra batching ở **320×180, 30000/1001**, 65 clip thành đúng **195 frame**; sample hình/PCM qua biên các vùng và hủy. Test VFR đặt clip ở cuối timeline 90 phút, xuất vùng và cắt tại khoảng không có frame, kiểm tra hình/tiếng theo source timestamp. Test browser bắt PCM 880Hz của stream thứ hai được đánh dấu default; test export kiểm tra đúng stream index 2/880Hz và loại 440Hz của stream đầu. Lựa chọn stream thủ công chưa có UI.

## Chạy lại và CI

```powershell
npm run build
npm test
npm run test:ci
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib timeline_export::tests::media_ -- --ignored --nocapture
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib benchmark_ninety_minute_export -- --ignored --nocapture
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib timeline_hidden_gpu_sources_and_seek_generations -- --ignored --nocapture
$env:PYTHONPATH = Join-Path $PWD 'python'
python -m unittest discover -s python/tests -v
python -m unittest discover -s scripts/tests -p 'test_*.py' -v
```

UI fixture đã commit, không cần model hoặc app-data thật:

```powershell
npm install --no-save --package-lock=false playwright@1.62.1
node node_modules/playwright/cli.js install chromium
npm run test:timeline-ui
```

Có thể dùng `PLAYWRIGHT_MODULE` trỏ tới runtime desktop đã có; `PLAYWRIGHT_CHANNEL=chrome` dùng Chrome cài trên máy. `FFMPEG_PATH` đổi binary nếu không dùng FFmpeg bundled; `TIMELINE_TEST_PORT` đổi cổng riêng (mặc định 1422). Script tự tạo media trong `.release-cache/timeline-stability-ui`, mở/đóng Vite/browser riêng, lưu `report.json` và ảnh. Browser test dùng Tauri boundary giả, media/DOM/Web Audio thật; lưu/phục hồi/encode và GPU được kiểm tra riêng bằng Rust. CI Windows chạy suite thường, năm media acceptance, UI T11 và upload report; benchmark dài/GPU vẫn opt-in để runner không phụ thuộc GPU.

## Giới hạn xác định

Preview không dựng MP4 sau chỉnh sửa. Active lớp không bị cắt bớt; export từ chối trên 32 visual input giao cùng một frame để tránh làm cạn tài nguyên. Batching cần thêm chỗ đĩa cho video trung gian, có preflight và cleanup khi kết thúc/hủy; output tạm ở thư mục ngoài project vẫn được giữ sau app crash theo chính sách recovery hiện tại. Video bị gián đoạn phải thử lại; checkpoint AI/STTN tiếp tục dùng cơ chế sẵn có. T11 không thay scope xử lý vùng nguồn T09 hoặc thêm speed ramp.
