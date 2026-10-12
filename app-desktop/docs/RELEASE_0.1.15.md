# JC AI 0.1.15

Rà soát ngày 2026-10-12 trên Windows. Bản phát hành gồm T01–T11 của editor
timeline và đặt mục **Tạo video AI** bên dưới **Dịch** trong sidebar.

## Nội dung chính

- Nhiều nguồn/track, timeline có thước thời gian, lớp hình/chữ/phụ đề/audio,
  cắt/trim/di chuyển/nhóm, undo/redo và điều khiển track.
- Transcript, dịch, nhận dạng và giọng lưu theo từng video nguồn, giữ danh tính
  khi đổi thứ tự hoặc cắt clip; dự án cũ được chuyển đổi và có bản dự phòng.
- Canvas/crop/transform, fade, keyframe, hòa tan/quét ngang. Preview phát từ
  nguồn theo một đồng hồ; export snapshot toàn bộ timeline hoặc vùng đã chọn.
- Chỉ mục interval và cache có giới hạn cho dự án dài; export chia vùng trên
  lưới frame chung, master audio liên tục; cleanup và retry sau gián đoạn.

## Sửa trong lượt rà soát phát hành

1. Native preview bỏ lỗi IPC muộn của decoder đã đóng. Khi đổi A sang B trong
   lúc seek A chưa hoàn tất, lỗi A không đóng B; seek B tiếp tục được gửi.
   Có hai test frontend kiểm tra lỗi cũ và fallback khi lỗi thuộc phiên hiện tại.
2. Fade vào/ra chồng nhau dùng ramp nhỏ hơn cả trong preview lẫn export.
   Test media bắt lỗi midpoint đỏ 62 thay vì khoảng 127, và qua sau sửa;
   test frontend xác nhận độ mờ 0,25/0,5/0,25 trên clip hai giây.
3. Đồng bộ 0.1.15 trong npm, lockfiles, Cargo và Tauri; giữ version worker độc
   lập. Bổ sung hướng dẫn thao tác và phạm vi xử lý nguồn/timeline.

## Gate cục bộ

- Build frontend và native (`cargo build --locked --no-default-features`);
  103 test TypeScript/23 file; 5 test công cụ release Node.
- 234 test Rust thường; 30 test model/GPU/benchmark tùy chọn.
- 5 media acceptance FFmpeg/FFprobe, gồm VFR/Unicode/multi-audio, batch/range,
  cancel, motion/chuyển cảnh và fade chồng nhau.
- 86 test worker Python, 10 tùy chọn; 4 test cache công cụ.
- UI Chromium: 30/60/90 phút, seek/phát A/B, pixel/caption/default audio,
  virtualization, nguồn thiếu/retry, manifest muộn, close cleanup và panel hẹp.
- Hai integration test audio cũ; merge copy/transcode Intel/CPU H.264/HEVC.
- Native hidden GPU A/B seek/cancel/fallback trên Intel Iris Xe; UI hồi quy
  T02–T10 và native protocol qua với media/DOM thật, IPC giả lập. Fixture T02/T04
  cục bộ được cập nhật theo API transaction/manifest và frame step hiện tại.
- Clippy all-targets qua, còn cảnh báo style trong code hiện tại.

Chi tiết fixture và benchmark dài ở [TIMELINE_STABILITY.md](TIMELINE_STABILITY.md).
Test model/hardware tùy chọn không đồng nghĩa đã kiểm tra mọi GPU hoặc máy sạch.
GitHub Actions chạy khi push; gate cục bộ không thay kết quả CI/bundle installer.
