# Task 8 — Hard-sub Removal / AI Video Inpainting Engine v2

## Trạng thái

Đã triển khai pipeline STTN dùng buffer, sparse OCR, cache theo đoạn, resume, native preview và export. Đã tải model thật, kiểm thử CPU, chạy luồng desktop/worker/FFmpeg thật và build release executable.

**Chưa xác nhận hiệu năng CUDA hoặc chất lượng trên phim thật dài có khuôn mặt/chuyển động phức tạp.** Lần đo CPU 12 frame cho thấy v2 giảm RAM/I/O nhưng chạy đầu chậm hơn legacy; không coi toàn bộ mục tiêu hiệu năng/chất lượng của Task 8 là đã đạt. Installer/PyInstaller worker chưa build lại trong lần này.

Task 7 đã commit `2ef9140`; code Task 8 đang ở working tree trên branch `dev`. Version giữ `0.1.13-preview`.

## Pipeline cũ và mới

Trước: FFmpeg ghi JPEG nguồn → OCR/mask PNG từng frame → STTN đọc ảnh, ghi PNG → FFmpeg đọc lại ảnh. Khi làm lại preview/export phải lặp lại phần lớn công việc.

Hiện tại, đường mặc định:

```text
Source video
  → RapidOCR: mẫu 1 hoặc 2 frame/s trong ROI
  → detection.json: thời gian, polygon, text/confidence
  → decode ROI của từng đoạn + temporal context trong RAM
  → mask ổn định và STTN theo cửa sổ
  → RGBA .pack + index PTS theo đoạn, commit an toàn
  → native preview / GPU compositor export dùng cùng patch
  → subtitle → text/logo → encode/audio engine hiện có
```

Không ghi JPEG/PNG từng frame trên đường xử lý mới. Hàm legacy giữ để benchmark/compatibility, không được gọi bởi Rust production pipeline. CPU compositor tương thích dùng một video FFV1/BGRA `.nut` cho mỗi đoạn từ pack, không tạo chuỗi ảnh. Clip H.264 ngắn vẫn được tạo để hỗ trợ preview tương thích.

## Kiến trúc và file

| Module | Trách nhiệm |
| --- | --- |
| `python/worker/text_removal/frames.py` | PyAV lazy decode, source PTS/VFR, crop ROI, core + halo hữu hạn |
| `detection.py` | OCR thưa, refine ranh giới có/không có chữ, cache validation |
| `masks.py` | Dilation/feather, ổn định polygon, mask context lân cận, scene/window planning |
| `inpainting.py` | `VideoInpainter` / `SttnInpainter`, tensor RGB/mask, encoder feature LRU |
| `patch_cache.py` | Pack zlib/index, checksum cấu trúc, atomic commit, FFV1 fallback |
| `pipeline.py` | Detect result → ROI windows → STTN → checkpoint/resume/report |
| `src-tauri/src/text_removal_cache.rs` | Reader Rust theo PTS, CRC, size/bounds, legacy PNG reader |
| `src-tauri/src/text_removal_service.rs` | Tauri jobs, cache identity, cancellation, safe paths, stale completion |
| `src-tauri/src/preview/clean_patches.rs` | Upload patch theo PTS và scale preview, texture dùng chung theo ROI |
| `src-tauri/src/preview/manager.rs` | Load cache khi mở lại/đổi vùng, telemetry `cleanPatches` |
| `src-tauri/src/render/{renderer,preview_target,stream}.rs` | Composite patches trước subtitle/logo, reader dùng chung với export |
| `src-tauri/src/render_service.rs` | Reuse cache trước khi yêu cầu runtime/model, FFmpeg compatibility |
| `src/features/workspace/translation/{TranslationPanel,TranslationView,native-preview}.tsx/ts` | Detect/Remove/Cancel, status, original full-video native preview |

Các file kết nối: `commands.rs`, `lib.rs`, `src/app/{project-state,services/project-service}.ts`, worker `__main__.py`, packaging spec, STTN optional requirements. Dependency mới: direct Rust `flate2`; thêm pin `av==18.1.0` vào runtime STTN. Không nâng Torch/CUDA hoặc thay các model khác trong task này.

## OCR và thời gian có chữ

Chọn 1 hoặc 2 mẫu/giây; mỗi mẫu decode source PTS thật rồi chỉ gửi ROI cho RapidOCR. Khi có/không có chữ đổi trạng thái, refine tối đa 4 lần; không OCR mọi frame. Padding ranh giới 80 ms, giới hạn trong thời gian vùng được bật. Đoạn cùng text/geometry được gộp, geometry dùng EMA 75% cũ / 25% mới. Dữ liệu này độc lập hoàn toàn với transcript/ASR.

OCR backend và STTN backend báo riêng. OCR tiếp tục dùng adapter CPU/CUDA/OpenVINO có kiểm tra phiên thực tế; production STTN dùng PyTorch CPU/CUDA. `fullRegion` vẫn dùng OCR để tìm thời gian có chữ, chỉ đổi mask trong các khoảng đó.

## ROI, mask và STTN

- Convert vùng normalized → pixel một lần ở Rust. Thêm margin context; giữ tỷ lệ 432:240 nếu kích thước nguồn cho phép. Mask được shift về local crop, phần ngoài mask giữ source.
- OCR-guided: fill polygon, dilation 6 px theo chiều cao tham chiếu 1080, feather 1.5 px phía trong mask. Full-region: fill đúng rectangle user, không fill toàn context crop.
- Các frame lân cận có caption khác cũng được mask; không coi chữ ở đoạn trước/sau là background sạch để làm reference.
- Tensor RGB float32 [-1,1], mask binary [0,1], input 432×240; giữ radius 5, stride 5, tối đa 8 refs. Không giảm resolution/reference count để lấy tốc độ.
- Ưu tiên ref sạch, chọn khoảng cách theo thời gian deterministic. ROI scene cut qua độ lệch RGB trung bình ảnh 64×36 >48; không dùng neighbor/ref qua scene cut phát hiện được. Cảnh rất ngắn được thêm center riêng.
- Encoder features LRU tối đa 24 frame, key gồm frame index/PTS/hash mask. Core tối đa 24 frame, halo 10 mỗi phía; RGB buffer có budget mục tiêu 64 MiB. Đây là budget mềm: ROI cực lớn với minimum core/context có thể vượt 64 MiB. Tensor/activations PyTorch lớn hơn riêng RGB buffer.
- Chỉ xuất core frame có chữ; blend các prediction của cửa sổ chồng nhau, giữ source alpha/mask bên ngoài.

## Cache, resume, an toàn

```text
project/cache/text-removal/v2/
  {detectionKey}/
    detection.json
    {paintingKey}/
      {spanKey}.pack
      {spanKey}.json
      {spanKey}.nut        # chỉ compatibility
      result.json
      report.json
```

Detection key: source checksum, ROI, time bounds, scan FPS, OCR model/device/version. Painting key: STTN model hash, ROI, mask mode, context-mask algorithm revision. Span key còn gồm fingerprint nội dung detection, mask profile và settings STTN. Sửa bản dịch/style không làm chạy OCR/STTN lại; thay source/vùng/cấu hình xử lý làm cache cũ không còn được chọn.

Pack chứa zlib RGBA theo từng frame, index có PTS/end thật và offset/length; không base64 pixels qua IPC. Reader kiểm tra finite time, bounds, header, byte count, zlib CRC, size tối đa và phạm vi file. Detection cache thiếu/sai schema được scan lại.

Ghi `.partial` → flush/fsync → replace pack → completion JSON atomic. Hủy/worker bị kill không xác nhận file đang ghi; đoạn đã commit giữ lại. Chạy lại xác minh từng pack, chỉ xử lý đoạn thiếu/hỏng. UI có trạng thái `not_detected/detected/partial/removed` và số khung/MB đã lưu. Job lock hiện có ngăn writer cùng project. Stale completion kiểm tra source và region identity; không nhận clip từ cấu hình đã thay đổi.

## Preview/export và commands

- `start_text_removal_preview(..., detect_only)` nhận diện riêng hoặc phục hồi; Detect không cần STTN runtime.
- `get_text_removal_cache_status(project_id, region_id)` trả trạng thái/interval/sample/backend/frame/bytes, không trả pixels.
- Các command STTN model/runtime đang có được giữ.
- Worker operations mới: `text_removal_detect_v2`, `text_removal_inpaint_v2`, `text_removal_patch_video` qua JSONL; logs ở stderr.
- Trước chạy job, frontend flush visual settings để backend dùng đúng vùng vừa chỉnh.
- Native preview mở **video nguồn đầy đủ**, overlay patch theo source PTS; không bị giới hạn clip 3 giây. Mở lại project tự resolve cache. Outside span không overlay patch.
- GPU staging export đọc cùng pack/PTS. CPU compatibility dùng FFV1 alpha NUT và seek/offset tương ứng. Subtitle/logo nằm trên lớp phục hồi. Audio/TTS/ASR giữ engine hiện có.
- Cache hoàn tất vẫn dùng được khi model đã gỡ; chỉ yêu cầu model/runtime cho công việc thiếu. Legacy clip/PNG còn đọc được ở các đường tương thích.

## Model và môi trường đã tải

Model thật: 66,252,587 bytes, SHA-256 `25b0c2c30042d82efd1893bd42ec726764262d94115393a1718f8d65d2a7817b` từ URL STTN đã pin trong app. Đã lưu `C:\Users\Admin\AppData\Roaming\com.ai.video-localization\models\text-removal\sttn.pth`.

PyTorch 2.8 CPU cài riêng trong `.release-cache/sttn-qa/venv` để QA; không thay môi trường AI đang dùng. App QA chạy data/runtime riêng trong `.release-cache/preview-validation`. Việc này không đồng nghĩa runtime STTN của bản app cài bình thường đã được cài: người dùng vẫn quản lý runtime bằng Settings của app.

## Benchmark CPU thật

Máy Intel Iris Xe, không NVIDIA; model STTN thật. Clip synthetic 432×240, 12 frame @6fps, background chuyển động có ground truth. Legacy và v2 chạy subprocess riêng, process peak RSS lấy từ chính child model process; số này không phải RSS launcher Windows.

| Metric | Legacy | V2 |
| --- | ---: | ---: |
| JPEG nguồn / PNG mask / PNG output | 12 / 12 / 12 | 0 / 0 / 0 |
| Disk ảnh trung gian / persistent pack | 2,019,439 B | 1,670,151 B |
| Extract ảnh | 0.024 s | 0 |
| STTN + ghi ảnh / inpaint riêng | 45.991 s | 53.845 s |
| Tổng uncached | 46.015 s | 54.170 s |
| Process peak RAM | 2,595,045,376 B | 2,038,337,536 B |
| ROI RGB buffer peak | không đo riêng | 3,732,480 B |
| Masked MAE với background thật | 2.592 | 2.551 |
| Temporal MAE (có motion nền) | 3.018 | 3.076 |

V2 giảm peak process RAM khoảng 21.5%, giảm disk khoảng 17.3%, nhưng cold CPU chậm hơn khoảng 17.7% trong lần đo này. Lần đo trước là 77.2 s / 75.4 s: biến động lớn, chưa có cơ sở công bố inference nhanh hơn. Dense-subtitle clip ít có lợi từ sparse interval skipping; clip nhiều khoảng không có chữ và export lần sau mới hưởng lợi rõ.

Reuse cùng pack: 0.033 s, 0 decoded ROI, 0 inference, không import/nạp model. Đây là benchmark cache layer, không phải tổng thời gian encode video/export.

Raw metrics được lưu trong `docs/benchmarks/text-removal-cpu-2026-10-04.json`. Timing này chưa bao gồm OCR; benchmark chỉ so phần removal. OCR thật ở desktop fixture 2 giây: 4 mẫu/4 OCR call @2fps, ~4.03 s. STTN fixture đó: 12 active frames/~43.47 s, persistent pack 880,688 B. Lần functional rerun sau sửa context-mask, chạy cùng Rust compilation: OCR 4.84 s / removal 57.00 s, preview cache 1.835 s gồm ghép clip. Không dùng lần chạy song song này làm benchmark hoặc so ngang hai clip/ROI khác nhau.

## Kiểm tra chất lượng

Đã chạy model thật cho 7 clip synthetic: nền tĩnh, nền chuyển động, mô phỏng camera pan, hình nhân chuyển động, Chinese 2 dòng, outline/shadow, scene cut. Mỗi case 3 frame; cả legacy/v2 đều đủ frame. Đã xem ảnh so sánh: không còn nét chữ trên các frame mẫu, nhưng vùng phục hồi có thể mềm/mất chi tiết; biên hình nhân vẫn có artifact ở cả hai bản. Không coi hình nhân vẽ là kiểm chứng người thật/phim thật.

Các MAE nằm trong `docs/benchmarks/sttn-quality-synthetic-2026-10-04.json`. Các case v2 không luôn có MAE thấp hơn legacy; MAE không chứng minh không flicker/không ghosting. Ảnh comparison local: `.release-cache/sttn-qa/quality-matrix/comparison.png` (cột input / known clean / legacy / v2).

## CUDA và Intel/OpenVINO

CUDA integration skip do không có NVIDIA; chưa đo VRAM/tốc độ CUDA thực tế.

Prototype dev `scripts/prototype-sttn-onnx.py` export STTN thật ONNX opset 17/dynamic time dimension, chạy ORT CPU và OpenVINO Intel GPU. Với 2/3 frame FP32: max absolute error khoảng 0.00008–0.00011 với PyTorch; execution device `GPU.0` thật. Default precision có sai số lớn, nên prototype bắt FP32.

ORT CPU: 3.04/4.86 s; OpenVINO GPU: 11.05/5.80 s (gồm first inference overhead). Conversion khả thi ở prototype, **chưa nhanh hơn, chưa chứng minh window lớn/chất lượng video dài**. Không đăng ký vào production, không quảng cáo Intel GPU STTN chỉ vì OCR hỗ trợ OpenVINO. Report: `docs/benchmarks/sttn-openvino-prototype-2026-10-04.json`.

## Tests/build đã chạy

- Frontend build thành công; Vite cảnh báo bundle ~501.6 kB, không phải build failure.
- Vitest 25 pass; CI script tests 4 pass.
- Rust 141 pass / 22 ignored (hardware/model opt-in), không fail.
- Python 71 tests: 64 pass / 7 skip; STTN v2 có 12 tests không cần model.
- Real STTN CPU integration: 1 pass (buffer/legacy forward parity ≤1 byte, encoder cache, invalidation); 1 CUDA skip.
- Real desktop: import video → RapidOCR detect → cancel/retry STTN → pack reuse → FFmpeg H.264 CPU export → FFprobe valid. Output `STTN-v2-QA.mp4`, 2 s, 11,525 B.
- Native desktop automation: seek 0.2/0.8/1.6 s đều `cleanPatches=1`; play/audio clock; route reopen vẫn đọc cache, bytes không đổi. Native decoder/compositor dùng Intel Iris Xe; STTN inference vẫn CPU.
- Release Rust executable build thành công. Chưa tạo installer mới trong task này.
- Công cụ computer-use không khởi động được (`failed to write kernel assets`); chưa kiểm tra thủ công click/resize screenshot native UI. Automated Tauri/native status và ảnh output đã kiểm tra.

## Chạy lại

```powershell
npm run build
npm test
npm run test:ci
cargo test --manifest-path src-tauri/Cargo.toml --lib
$env:PYTHONPATH=Join-Path (Get-Location) 'python'
python -m unittest discover -s python/tests
# Chỉ real integration mới cần Torch/model; không tự download trong unit test.
$env:JC_STTN_MODEL='C:\path\to\sttn.pth'
python -m unittest discover -s python/tests -p test_sttn_integration.py -v
python scripts/benchmark-text-removal.py --model $env:JC_STTN_MODEL --output .release-cache/my-fresh-benchmark
python scripts/quality-text-removal.py --model $env:JC_STTN_MODEL --output .release-cache/my-fresh-quality
```

Benchmark cần dev `psutil` và Pillow; ONNX prototype cần dev `onnx`, `onnxruntime`, `openvino`. Các dependency thử nghiệm không thêm vào runtime app. Smoke scripts chỉ chạy khi app debug dùng **QA data directory**, WebView CDP port 9224; scripts từ chối dữ liệu chứa project không bắt đầu `Native Preview QA`.

```powershell
$env:JC_TEXT_REMOVAL_SOURCE='C:\path\short-hard-sub.mp4'
node scripts/text-removal-smoke.mjs
$env:JC_TEXT_REMOVAL_PROJECT='UUID-QA-project-from-smoke'
node scripts/text-removal-preview-smoke.mjs
```

## Kiểm thử thủ công cần bổ sung

1. Cài RapidOCR và STTN trong Settings; chọn CPU/CUDA theo máy thực tế.
2. Import clip phim thật, khoanh vùng chữ, chọn STTN OCR-guided hoặc full-region, scan 1/2fps.
3. Bấm Nhận diện, kiểm tra số đoạn; bấm Xóa phụ đề, hủy giữa job rồi tiếp tục.
4. Seek trước/trong/sau interval, resize preview, đóng/mở app; xem source/patch/subtitle/logo đúng z-order.
5. Export CPU và NVIDIA (nếu có); export lần hai sau đổi translation/style xác minh log không gọi OCR/STTN lại.
6. So frame/clip nền tĩnh, pan, người/quần áo đi qua subtitle, scene cut, CJK hai dòng/viền dày; kiểm tra ghosting/flicker trước khi phát hành.

## Hạn chế và hướng Task 9

- Sparse scan có thể bỏ sót caption hiện quá ngắn hoặc text di chuyển nhanh; chưa thêm dense/adaptive tracking mode. Không tự OCR mọi frame để che vấn đề.
- Scene detector ROI chỉ là heuristic; cut tinh tế có thể lọt, chuyển động mạnh có thể gây false cut. Reference giới hạn context hữu hạn, không tìm xuyên toàn clip dài.
- First-pass CPU temporal attention/decoder vẫn nặng; bỏ I/O ảnh không đủ giải quyết toàn bộ thời gian inference. Full-region che nhiều pixel có thể mất chi tiết nền.
- RAM budget RGB mềm, PyTorch activation có thể dùng vài GB; chưa stress test 4K/8K ROI và video nhiều giờ.
- CPU compatibility tạo một NUT/input theo interval; nhiều interval có thể làm FFmpeg graph/process nặng. Cần stream unified compositor cho CPU thay vì rất nhiều input.
- Detection cache vẫn invalidates theo OCR device dù backend chỉ thay phần cứng; conservative, có thể tối ưu semantic identity sau.
- Chưa build lại packaged worker/installer, chưa test máy sạch hoặc chất lượng phim dài/NVIDIA.

Task 9 nên ưu tiên profiling trên NVIDIA với phim thật, quality gate theo scene/duration/VRAM, hỗ trợ CPU compositor một stream, cache quota/cleanup và adaptive detection; chỉ cân nhắc production Intel STTN sau benchmark ONNX/OpenVINO window thực tế đạt quality/performance.
