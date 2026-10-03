# Task 4 — GPU visual renderer

Task 5 bổ sung đường Windows GPU surfaces; xem [Native GPU media pipeline](NATIVE_GPU_MEDIA_PIPELINE.md).
Trong Auto, đầu vào được hỗ trợ và hardware encoder có thể dùng đường Tier B thay cho staging mô tả bên dưới.

## Kết quả và phạm vi

Bộ render Rust/wgpu đã được nối vào **Export chính** trong `run_export_inner`.
Ở chế độ mặc định Auto, video SDR/CFR tương thích dùng wgpu cho hiệu ứng;
FFmpeg tiếp tục decode, xử lý audio, encode H.264/HEVC và mux MP4.
Không thêm model, Python renderer, CUDA pack hay SDK shader.

Task 1–3 và dữ liệu project vẫn giữ nguyên. Không có schema migration.
Tên `strength_px` của spec cũ thực tế chứa mức điều khiển 1–100; không đổi
field để tránh phá compatibility. Quy đổi sigma được định nghĩa rõ bên dưới.

## Dependency / thiết bị

- wgpu **27.0.1** (pin), pollster **0.4.0**, cosmic-text **0.18.2** (pin).
- bytemuck dùng uniforms; image decode PNG/JPEG/WebP.
- DX12 được thử trước, Vulkan/Metal sau. Adapter HighPerformance được yêu cầu
  qua wgpu; integrated GPU được chấp nhận, CPU software adapter bị bỏ qua.
- Adapter, backend, device type, driver, texture format và texture limit
  được ghi log một lần khi tạo renderer.
- WGSL compile qua wgpu, không yêu cầu người dùng tải compiler/runtime khác.
- Shader validation, uncaptured errors và device loss được chuyển thành lỗi;
  Auto có thể thử lại bằng legacy.

API tham chiếu: [wgpu 27.0.1](https://docs.rs/wgpu/27.0.1/wgpu/),
[cosmic-text 0.18.2](https://docs.rs/cosmic-text/0.18.2/cosmic_text/).

## Source và kiến trúc

| File/module | Trách nhiệm |
|---|---|
| `src-tauri/src/render/mod.rs` | RenderBackend Auto/Wgpu/Legacy |
| `device.rs` | Chọn adapter, device, diagnostics, lỗi GPU |
| `frame.rs` | Borrowed VideoFrame, PTS, PixelFormat |
| `graph.rs` | RenderScene từ RenderSpecification, thứ tự, timing, quy đổi blur |
| `texture.rs` | Texture allocation/upload |
| `renderer.rs` | Điều phối các pass và readback |
| `passes/blur.rs` | Resource ROI, Gaussian ngang/dọc, feather/noise |
| `passes/logo.rs` | Decode/upload/cache logo và aspect fit |
| `passes/composite.rs` | Bind group, draw/composite bằng straight alpha |
| `passes/text.rs` | Cache texture chữ và auto-fit với font thực |
| `text.rs` | Shape Unicode, font fallback, raster glyph/outline/background |
| `shaders/effects.wgsl` | Pass-through, Gaussian, alpha/feather/noise |
| `stream.rs` | Pipe decode → render → encode, bounded buffers, PTS, cancel |
| `tests.rs` | Pixel correctness, cache invalidation, warm 1080p benchmark |
| `src-tauri/examples/render-frame.rs` | Tool input.png → wgpu → output.png |
| `src-tauri/src/render_service.rs` | Tích hợp main Export, audio/encoder/fallback/commit |
| `src-tauri/src/render_spec.rs` | Bổ sung style cho renderer; obey Apply Logo |
| `src-tauri/src/text_removal_service.rs` | GPU permit cho OCR/STTN |
| `src/shared/visual-effects.json` | Contract blur cho Rust và frontend |
| `src/features/overlays/blur-visual-params.ts` | Quy đổi contract sang CSS preview |
| `OverlayCanvas.tsx` | Dùng cùng sigma/feather trong video-space |
| `Cargo.toml`, `Cargo.lock` | Graphics dependencies |

Luồng:

```text
Project → RenderSpecification → RenderScene
FFmpeg decode → RGBA + integer PTS/time base
             → wgpu:
               base → clean patch → blur → subtitle → text → logo
             → reusable RGBA readback
             → existing FFmpeg audio graph + encoder resolver → MP4
             → FFprobe validation → atomic/no-clobber publish → export history
```

RenderScene chỉ chứa dữ liệu render; backend resolve/validate asset trước khi
truyền vào renderer. Renderer không đọc project hay credentials.
Không thêm Tauri command hay đổi API frontend của Export.

## Blur, feather và parity

- `visual-effects.json` là contract chung: sigma = clamp(strength, 1, 100) × 2
  **pixel video nguồn**.
- Preview scale sigma đúng một lần theo displayed height / source height.
- GPU chỉ crop ROI, downsample theo sigma để kernel nhỏ, Gaussian separable
  25 taps ngang rồi dọc, upsample và composite vào vị trí cũ.
- Feather = min(region width, height) × edgeBlend/50 × 0.1, smoothstep trên GPU.
- strength ≥ 95 bỏ feather để không trộn lại chữ gốc ở viền.
- Noise được hash theo tọa độ, giữ ổn định giữa frame; intensity theo phần trăm.
- Multiple regions chạy crop/effect/composite trên cùng target, không tạo N
  pipeline decode/encode đầy đủ.
- Legacy gblur cũng lấy sigma và feather từ contract.
- CSS blur và FFmpeg Gaussian là approximation; cùng semantic/rect nhưng chưa
  byte-identical. Native preview dùng cùng shader thuộc Task 6.

## Logo và text/subtitle

Logo được decode/upload một lần trên mỗi asset path trong scene, dùng PNG
straight alpha, opacity, aspect ratio và start/end time. Logo nằm trên cùng.

Chữ dùng cosmic-text để shape Unicode với font hệ thống và Swash glyph cache.
Có family, weight, size, color, opacity, horizontal alignment, line spacing.
Font rỗng dùng Segoe UI; fallback glyph theo font hệ thống cho CJK.

Phụ đề dùng cùng RenderSpecification/layout đã resolve, hiển thị theo PTS trong
subtitle region. Không gọi FFmpeg `subtitles=` trên GPU path. Padding, outline,
background, horizontal/vertical alignment và auto-fit được áp dụng. Font thực
được đo/wrap lại; nếu cần giảm từng 1px tới minimum, không truncate.
Nếu minimum vẫn không vừa, báo lỗi/fallback thay vì cắt chữ.
Outline đã triển khai; shadow riêng chưa có (legacy style hiện cũng Shadow=0).

Text raster và texture cache key gồm ID + text + rect + toàn bộ style/minimum.
Thay text nhưng giữ ID không dùng lại texture cũ. Layer cache LRU giới hạn
64 entries / 64 MiB; glyph cache được dọn khi vượt 16,384 entries / 32 MiB.
Static text không được raster/upload lại mỗi frame.

## Timing, màu và giới hạn đầu vào

- Visual events dùng timestamp thực từ decoder, không dùng frameIndex/30.
- Dùng integer PTS × time base; tránh mất độ chính xác từ `pts_time` được
  FFmpeg làm tròn ở video dài.
- Bridge rawvideo v1 yêu cầu CFR. Preflight kiểm tra avg/r frame rate; runtime
  kiểm tra delta PTS. VFR được trả về legacy thay vì âm thầm đổi timing.
- RGBA8Unorm SDR chứa RGB đã encode gamma, compositing cùng convention CSS.
  Không phải linear-light/HDR renderer.
- HDR/PQ/HLG, 10-bit/P010 hoặc display rotation dùng legacy trong Auto;
  forced wgpu báo lý do rõ. NV12/P010 có enum để mở rộng, chưa là input native.
- GPU textures/source/target/ROI/bind groups/pipelines/readback reuse.
- V1 vẫn có upload + readback mỗi frame, chưa zero-copy decode/encoder surface.

## Audio, GPU scheduling, jobs và filesystem

Original audio, TTS delay, volume, ducking, dialogue stem, AAC dùng graph có sẵn.
Thêm silence padding tới source duration để `-shortest` không cắt video khi
chỉ bật dubbing và câu cuối kết thúc sớm. Nội dung/tốc độ TTS không đổi.

Renderer dùng GPU permit chung với AI; khởi tạo **sau** STTN/BandIt preparation.
OCR/STTN worker cũng giữ permit khi có thể dùng GPU; được nhả trước render.
STTN được serialize bảo thủ kể cả khi runtime chọn CPU để tránh tranh thiết bị.

Pipe dùng hai source buffers tái sử dụng + một output buffer, frame queue=1,
PTS queue=8. Không preload video vào RAM/VRAM và không tạo temp PNG giữa decode,
wgpu và encoder. AI patch sequence có sẵn vẫn là PNG của pipeline STTN cũ;
chỉ đọc frame patch đang cần, giữ texture tương ứng và bounded current frame.

Cancel watchdog dừng cả decoder lẫn encoder kể cả lúc pipe bị block.
Process RAII kill/wait khi lỗi. Final mux wait giới hạn 120s, GPU poll 10s.
Output vẫn là partial temporary file, FFprobe validate rồi persist_noclobber.
Source không bị thay đổi. Progress, cancel và export history tiếp tục qua jobs cũ.

## Backend lựa chọn / fallback

Internal configuration:

```powershell
$env:JC_RENDER_BACKEND = "auto"    # mặc định
$env:JC_RENDER_BACKEND = "wgpu"    # lỗi rõ nếu không chạy được
$env:JC_RENDER_BACKEND = "legacy"  # FFmpeg visual renderer
npm run tauri dev
```

Auto không khởi tạo được device / gặp lỗi shader/render / unsupported timing
sẽ báo thông báo tương thích và retry legacy một lần. Encoder Auto tiếp tục
dùng resolver/candidate fallback của Task 1; không viết lại NVENC/QSV/AMF.
GPU renderer và encoder là hai lựa chọn riêng: wgpu + CPU encode là hợp lệ.

## Kiểm thử và benchmark thực tế (2026-10-02)

Hardware: **Intel Iris Xe Graphics, DX12**, có Intel QSV H.264.
Kết quả fixture thật:

| Test | Kết quả |
|---|---|
| RGBA pass-through | Byte-identical |
| Blur ngoài/trong vùng, feather, 2 regions | Pass |
| Alpha logo, logo trên subtitle/blur | Pass |
| Việt/Trung; Unicode Nhật/Hàn raster test | Pass |
| Subtitle timing, sharp glyphs, cache reuse/edit | Pass |
| RGBA clean patch, rect overflow rejection | Pass |
| 1080p Export legacy CPU | Pass |
| 1080p Export wgpu + CPU encoder | Pass |
| 1080p Export wgpu + QSV H.264 | Pass |
| Dubbing + original + ducking | Pass |
| Dubbing only / pad end | Pass |
| Cancel decoder/encoder | Pass, assertion <5s |
| FFprobe output resolution/duration/audio/codec | Pass |

Warm render-only 1920×1080, 60 frames, blur + subtitle + text + alpha logo:
**152–168 fps**, **6.0–6.6 ms/frame**, gồm upload/readback; cache text 1.44 MB.
Không gồm FFmpeg decode/audio/encode hoặc device/font initialization.

Export fixture 1s / 20 frames / 1080p (debug build):
legacy CPU khoảng **0.55–1.17s**;
wgpu + CPU khoảng **0.43–0.67s**;
wgpu + QSV khoảng **0.63–0.83s**.
Fixture rất ngắn, startup ảnh hưởng mạnh; đây không phải cam kết tốc độ video dài
hay bằng chứng QSV luôn nhanh hơn CPU. Không có performance threshold trong CI.

Artifacts/logs local (ignored): `.release-cache/gpu-render-validation/`,
`wgpu-hardware-tests.log`, `wgpu-unit-tests.log`,
`wgpu-frontend-build.log`, `wgpu-frontend-tests.log`.

```powershell
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml

# Opt-in hardware tests; no AI model download
$env:JC_GPU_TEST_ARTIFACTS = Join-Path $PWD ".release-cache/gpu-render-validation"
cargo test --manifest-path src-tauri/Cargo.toml --lib gpu_ -- --ignored --nocapture --test-threads=1

# Single-frame tool
cargo run --manifest-path src-tauri/Cargo.toml --example render-frame -- input.png output.png
```

Kết quả hồi quy: cargo test **112 passed / 7 ignored**, npm test **25 passed**,
scripts CI **4 passed**, Python unittest **57 tests / 5 skipped**, npm production
build thành công. `cargo build --release --manifest-path src-tauri/Cargo.toml`
cũng thành công, không có compiler warning; lượt đầu biên dịch graphics crates
mất 4m36s. Đây là build binary Rust, chưa đóng gói installer mới. Bốn test GPU opt-in đã chạy riêng và đều pass; ba test hardware
legacy còn lại vẫn opt-in theo môi trường. Python tests chạy từ thư mục python:
`python -m unittest discover -s tests`.

GPU tests bỏ qua khi không có adapter tương thích. Các fixture filesystem dùng
tempdir biệt lập, gồm Unicode path; không thao tác dữ liệu project người dùng.

## Chưa kiểm chứng / Task tiếp theo

- NVIDIA NVENC, AMD AMF và Vulkan chưa kiểm thử trên hardware thực trong Task 4.
- Chưa test installer sạch hoặc chạy tay đầy đủ GUI preview/export side-by-side.
  Đã kiểm tra ảnh pixel và main export helper/bridge thực trên Iris Xe.
- Chưa benchmark video 1+ giờ, peak RAM/VRAM đo ngoài hoặc driver device-loss thật.
- Chưa chạy STTN/BandIt model end-to-end trong Task 4; compositor patch/audio
  interface được nối, không thay engine AI.
- Merge Video vẫn dùng pipeline riêng hiện tại.
- Layout CSS/libass/cosmic-text có khác font metrics; Auto legacy vẫn khả dụng.
- Task 5: hardware decode/reduced-copy NV12 và encoder surfaces, giữ PTS VFR.
- Task 6: native preview dùng cùng renderer, parity bằng regression screenshots.
