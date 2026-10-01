# Hardware Acceleration v1

## Phạm vi và cách dùng

Export Video và Gộp Video dùng chung lựa chọn **Tự động, CPU, NVIDIA NVENC, Intel Quick Sync, AMD AMF**, cho H.264 và HEVC. Tự động là mặc định của các lần xuất mới, kể cả chế độ dịch hàng loạt.

Chọn codec trước, rồi chọn bộ mã hóa. Màn hình hiển thị backend mà Auto chọn, lý do các backend khác không khả dụng và nút Kiểm tra lại. Chọn thủ công không bị đổi âm thầm khi đổi codec. Nếu phần cứng không hỗ trợ lựa chọn đó, backend trả lỗi rõ ràng.

Gộp video vẫn ưu tiên stream copy khi stream, codec và khung hình phù hợp, không có logo/chữ/hiệu ứng cần render. Trong trường hợp này lựa chọn encoder không ép chuyển mã và không yêu cầu GPU có mặt.

## Kiến trúc

- `src-tauri/src/hardware_acceleration.rs`: `VideoCodec`, `VideoEncoder`, mapping tám encoder, probe thật, capability cache, Auto resolver, retry policy và cấu hình chất lượng/bitrate riêng cho từng backend.
- `src-tauri/src/render_spec.rs`: cấu hình dùng enum chung. Trong `render_service::prepare`, backend thực tế được resolve trước khi specification được chuyển tới FFmpeg builder.
- `src-tauri/src/render_service.rs`: validation trả `resolvedEncoder`/`resolvedEncoderLabel`; encode và retry Auto dùng chung filtergraph, audio và AI patches đã chuẩn bị.
- `src-tauri/src/video_tools_service.rs`: quyết định stream copy trước khi resolve encoder; command builder dùng chung quality policy, vẫn giữ chất lượng CPU/NVENC trước đây của Merge.
- `src/app/services/video-encoding.ts`: types/capability presentation chung cho frontend; có unit tests.
- `src/shared/components/VideoEncoderSelect.tsx`: dropdown, trạng thái, refresh và chẩn đoán ngắn cho cả Export/Merge.
- `src/app/services/render-service.ts`, `video-tools-service.ts`, `src/features/workspace/ExportDialog.tsx`, `ImportPlaceholder.tsx`: nối IPC và UI.
- `src/features/projects/batch-workflow.ts`, `src/features/workspace/batch/batch-video-runner.ts`: mặc định Auto cho batch mới.
- `src/styles.css`: style control dùng chung.
- `src-tauri/src/commands.rs`, `lib.rs`: đăng ký module và cập nhật types command.
- `scripts/build-ffmpeg.ps1`: kiểm tra đủ tám encoder, AAC, ASS, boxblur và gblur; cập nhật `ffmpeg-build.json` được tạo trong resources. Không kiểm tra driver trên máy build.

Command `get_export_capabilities(refresh?: boolean)` trả danh sách `encoders` generic và `recommendedEncoders` theo codec. Các command Export/Merge hiện có vẫn giữ tên. Lịch sử cũ với `cpu`/`nvidia` vẫn deserialize; trường encoder bị thiếu có mặc định Auto, không cần sửa file project.

## Probe, cache và Auto

Mỗi cặp codec/backend được encode thử một frame màu 256×256, tới null output. Probe chạy bằng process arguments, không shell, không hiện console, không tạo file. Mỗi process có timeout 5 giây và được kill/wait nếu quá hạn. Stderr được đọc song song và giới hạn phần lưu; lỗi kỹ thuật chỉ ghi log nội bộ.

Một memory cache dùng chung được bảo vệ bằng mutex và gắn với đường dẫn FFmpeg. Các lệnh IPC kiểm tra chạy trong `spawn_blocking`; mở dialog và validation không probe lặp lại. Refresh chủ động kiểm tra lại sau thay đổi driver.

Policy mặc định: NVIDIA → Intel → AMD → CPU, chỉ xét backend đã encode thành công **codec đang chọn**. Hàm `resolve_with_priority` tách riêng policy, có thể thay bằng preference/benchmark sau này. Auto có thể thử backend khác nếu lỗi khởi tạo encoder trước khi có frame đầu ra; không retry lỗi filter, ổ đĩa hay encode đã chạy. Retry có giới hạn theo số backend khả dụng. Lựa chọn thủ công không fallback.

## Quality và container

- CPU: preset/CRF cũ; custom bitrate dùng constrained VBR.
- NVENC: preset p2/p5/p6, VBR/CQ cũ; custom bitrate dùng VBR.
- QSV: preset riêng, ICQ qua global quality; custom bitrate dùng VBR. Chỉ H.264 truyền `look_ahead`; HEVC không nhận option đó. Tham chiếu [FFmpeg QSV](https://ffmpeg.org/ffmpeg-codecs.html#QSV-Encoders).
- AMF: quality speed/balanced/quality, CQP với qp_i/qp_p; custom bitrate dùng vbr_peak. Tham chiếu [FFmpeg AMF implementation](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/amfenc_h264.c).

Output vẫn MP4 8-bit 4:2:0 (FFprobe `yuv420p`), AAC, `+faststart`; HEVC dùng tag `hvc1`. QSV có thể tự chuyển input sang NV12 nội bộ. Không bật hardware decoding hoặc thay đổi video filters/AI runtimes trong task này.

## Kiểm thử

```powershell
npm run build
npm test
$env:LOCALIZATION_FFMPEG_DIR = (Resolve-Path 'src-tauri/resources/ffmpeg').Path
$env:PATH = $env:LOCALIZATION_FFMPEG_DIR + ';' + $env:PATH
cargo test --manifest-path src-tauri/Cargo.toml
```

Integration tests phần cứng được tách riêng để không ép máy CI có GPU. Tất cả dùng temp directory, không sửa dữ liệu project thật:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml runtime_encodes_available_backends_to_valid_mp4 -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml real_merge_copy_and_transcode_keep_codec_audio_and_duration -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml real_export_with_effects_on_available_backends -- --ignored --nocapture
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-ffmpeg.ps1
```

Unit tests bao phủ mapping, codec-aware Auto, priority thay thế, fallback CPU, lỗi manual, retry có giới hạn, deserialize legacy, options không lẫn giữa vendor, giữ stream copy khi chọn hardware, và status frontend.

Trên máy kiểm thử: CPU và Intel QSV encode thật H.264/HEVC thành công, gồm Fast/Balanced/High, quality mode và bitrate mode, AAC, tag HEVC, pixel format và đường dẫn tiếng Việt. Đã kiểm tra gộp thật bằng copy/transcode và export thật với blur → subtitle → logo + dubbing audio. NVIDIA/AMD probe không khả dụng trên máy này: mapping/options có unit tests nhưng cần kiểm thử encode trên máy có GPU/driver tương ứng.

## Giới hạn

Chưa benchmark tự chọn GPU nhanh nhất trên máy nhiều GPU. Một frame probe không bảo đảm mọi độ phân giải/tải hệ thống đều dùng được; vì vậy có validation và retry Auto giới hạn khi khởi tạo thất bại. Tăng tốc trong task này dành cho video encoding; filters và các bước AI vẫn chạy theo pipeline hiện có.
