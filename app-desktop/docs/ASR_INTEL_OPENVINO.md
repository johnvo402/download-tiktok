# Whisper trên Intel GPU — Task 3

## Triển khai

ASR có lớp lựa chọn backend riêng và hai implementation: `FasterWhisperBackend` (CPU INT8 / NVIDIA FP16) và `OpenVinoWhisperBackend` (Intel GPU). `service.py` điều phối, chuẩn hóa kết quả và lưu transcript theo contract hiện tại. Rust tiếp tục trích WAV 16 kHz mono PCM16, quản lý job, hủy worker và lưu project. Không thay đổi FFmpeg, VieNeu, STTN hoặc BandIt.

Engine và thiết bị là hai cấu hình riêng. Engine gồm `auto`, `fasterWhisper`, `openVinoWhisper`, `whisperX`; thiết bị gồm `auto`, `cpu`, `cuda`, `openvino`. Các lựa chọn đã lưu vẫn được giữ; cấu hình mặc định cho cài đặt mới là Auto/Auto.

Auto/Auto ưu tiên faster-whisper CUDA → Whisper Intel nếu runtime **và model tương ứng** sẵn sàng → faster-whisper CPU INT8. Không tự chọn WhisperX, tự đổi model hay tải model. Khi backend tự động gặp lỗi lúc nạp/nhận dạng, worker phát thông báo chuyển backend. Chọn Intel hoặc NVIDIA thủ công thì báo lỗi, không âm thầm chuyển CPU.

WhisperX vẫn dùng flow faster-whisper + alignment hiện có. Intel + WhisperX không được hỗ trợ trong task này; frontend và backend đều chặn tổ hợp đó.

## Runtime và xác minh GPU

Sử dụng API native `openvino_genai.ASRPipeline`, pin cùng bộ phiên bản:

| Package | Phiên bản |
| --- | --- |
| openvino | 2026.4.0 |
| openvino-genai | 2026.4.0.0 |
| openvino-tokenizers | 2026.4.0.0 |
| faster-whisper, CPU dự phòng | 1.2.1 |

Không cần Torch, torchaudio, torchvision, CUDA, Transformers hoặc server Python để chạy Intel. Import inference được trì hoãn; import worker không nạp GenAI. Probe dùng chung với OCR, cache kết quả thành công trong worker: compile phép cộng trên GPU, kiểm tra `EXECUTION_DEVICES` thực sự là GPU và kết quả 1 + 2 = 3. Khi tải model, compile encoder/decoder trên GPU và chạy inference kiểm tra model; không chỉ dựa vào danh sách thiết bị.

Feature `whisper_openvino` dùng chung runtime với `ocr_openvino`:

```text
<app data>/runtimes/advanced/
├── current/
├── staging/
├── previous/
├── cache/                 # giữ cache tải package
└── compiled-whisper/      # cache biên dịch GPU
```

Thêm feature Intel vào runtime hiện có bằng clone staging, khóa các package hiện có, chỉ cài feature còn thiếu, verify tất cả feature, rồi atomic swap/rollback. Các package CPU/CUDA hiện có không được tự nâng/hạ phiên bản; nếu constraint xung đột, cài thất bại và giữ bản trước. Runtime chỉ có Intel không được chọn làm worker mặc định cho TTS và các tính năng khác. Đường dẫn constraint truyền cho uv dưới dạng file URI để hoạt động với thư mục có dấu/khoảng trắng.

Scheduler GPU hiện có được tái sử dụng cho job Intel và bước kiểm tra model. Không khởi chạy hai job AI GPU nặng đồng thời. Cài runtime/model và xóa model giữ các kiểm tra backend/path hiện có.

## Model, dung lượng và quản lý

Tải FP16 IR đã chuyển sẵn từ các repository Whisper chính thức của OpenVINO. Không chuyển đổi mỗi job, không dùng file CTranslate2 làm IR. Pin commit và danh sách file trong `openvino-models.json`; kiểm tra kích thước, hash SHA256 binary upstream, manifest local, revision và marker kiểm tra GPU trước khi báo ready. Tải vào staging có thể tiếp tục; chỉ promote sau kiểm tra thành công.

| Model Intel | Download xấp xỉ (MiB) | Commit pin |
| --- | ---: | --- |
| tiny | 75.3 | `44662d68573bd732e50fc295d1c1ec47e77f67df` |
| base | 142.0 | `84fbe975a79a8c996fd32c036558f29e2db6670f` |
| small | 465.3 | `2410d022171ca8a97343182f88eec8807a324db9` |

Medium/large-v3 vẫn hoạt động với CPU/NVIDIA, bị disable cho lựa chọn Intel thủ công. Auto với những model này dùng CPU/NVIDIA. UI lấy dung lượng model từ manifest backend, không hardcode size frontend.

```text
<app data>/models/
├── fw/<model>/                    # CTranslate2 hiện có, giữ nguyên
└── whisper/<model>/openvino/       # IR, tokenizer và manifest Intel
```

Settings phân biệt cài tăng tốc và tải model. Cả hai cần xác nhận. Xóa model Intel chỉ xóa artifact Intel, giữ model CPU/NVIDIA và transcript. Dung lượng sử dụng có dòng Whisper Intel riêng và nút mở thư mục tương ứng. Không tải khi mở Settings hoặc khi bắt đầu transcription.

Một môi trường sạch đã cài chung OCR + Whisper Intel và kiểm tra `pip check`, GPU inference, không có Torch. Runtime venv chiếm **691,273,440 bytes (~659 MiB)**, so với OCR-only **517,711,416 bytes (~494 MiB)**: tăng khoảng **166 MiB**, gồm cả CPU dự phòng. Đây là dung lượng cài đặt đo được, **không phải dung lượng tải mạng**; chưa tính managed Python, model, compiled cache và uv cache. Dung lượng tải phụ thuộc package đã được cài/cache. Cài thêm Intel không tải lại OpenVINO nếu đã có đúng phiên bản.

## Transcript, timestamp và video dài

Đọc tuần tự tối đa 30 giây PCM vào RAM, tiến 28 giây/window. Model nạp một lần cho toàn job. Hai giây overlap cung cấp ngữ cảnh; phân chia quyền sở hữu theo midpoint và gộp đoạn giống nhau sát/chồng nhau để tránh lặp biên. Offset được cộng vào timestamp model; không chia đều thời gian từ raw text.

Native model trả segment timestamps và language detection. Chuẩn hóa/sắp xếp, giữ Unicode và manual newline, kiểm tra số hữu hạn, `start >= 0`, `end > start`, end không vượt duration ngoài tolerance 0.25 giây. Timestamp thiếu end bị từ chối thay vì tự bịa; Auto có thể chuyển sang faster-whisper. Capability `wordTimestamps=false`: chưa có word alignment Intel, không mô phỏng.

Model đa ngôn ngữ hỗ trợ Auto và language code Whisper, gồm zh/vi/en/ja/ko. Chưa benchmark chất lượng thực tế từng ngôn ngữ đó. Video dài chỉ giữ một waveform window trong RAM, xử lý tuần tự; transcript kết quả tăng theo số segment. Progress theo giây audio đã xử lý, có heartbeat khi nạp model. Rust có thể kill worker ngay khi Cancel; Python dừng giữa các chunk. Không ghi transcript mới khi lỗi/hủy; output chỉ được atomic replace sau toàn bộ job thành công.

## Cách sử dụng

1. Cài/cập nhật driver Intel Graphics nếu probe GPU không chạy được.
2. Cài đặt → Nhận dạng lời thoại → chọn tiny/base/small.
3. Trong Whisper · Intel GPU, chọn **Cài tăng tốc Intel**, xác nhận.
4. Chọn **Tải model Intel**, xác nhận dung lượng, chờ nạp và kiểm tra GPU.
5. Chọn Engine Auto + Thiết bị Intel GPU để bắt buộc Intel, hoặc Auto/Auto để cho phép fallback.
6. Lưu cấu hình; tạo transcript như trước. Job hiển thị thiết bị thực tế đang chạy.

Inference chỉ đọc model local; không dùng Hugging Face download API trong transcription. Model/tokenizer nằm cùng artifact. Diagnostics trả backend khả dụng, runtime/model status, resolved backend và selection error qua `get_asr_environment`; log nội bộ có lựa chọn, duration, elapsed, RTF, segment count và language, không ghi nội dung thoại.

## API và file thay đổi

Giữ nguyên tên Tauri commands, mở rộng tham số tùy chọn:

- `get_asr_environment(model, alignment_language?, engine?, device?)`.
- `install_asr_model(model, backend?)`, `remove_asr_model(model, backend?)`: `ctranslate2` mặc định hoặc `openvino`.
- `generate_transcript` tiếp tục nhận options engine/device mới, cùng hệ thống job/cancel/persistence.

Các nhóm file:

- Backend Python mới: `asr/backends.py`, `openvino_backend.py`, `openvino_models.py`, `openvino-models.json`.
- Điều phối/manifest: `asr/service.py`, `worker/__main__.py`, `model_manifest.py`.
- Rust: `asr_service.rs`, `commands.rs`, `optional_runtime.rs`, `runtime_paths.rs`, `settings_service.rs`, `storage_service.rs`.
- Frontend: các service/types ASR/runtime/storage, `SettingsView.tsx`, `IntelWhisperCard.tsx`, `OptionalAiRuntimeCard.tsx`, `TranslationPanel.tsx`, `batch-workflow.ts`.
- Dependency/packaging: `requirements-optional-whisper-openvino.in`, `pyproject.toml`, `ai_worker.spec`, `build-optional-runtime.ps1`.
- Kiểm thử: `test_asr_backends.py`, `test_asr_openvino_integration.py`, `benchmark-whisper-openvino.py`, `test-openvino-runtime.py`.
- Notices: `docs/THIRD_PARTY_NOTICES.md`.

Release worker cơ bản vẫn không bundle OpenVINO/PyTorch. Manifest JSON được đóng gói trong worker và optional source; môi trường Intel tải riêng khi xác nhận. Không đổi app version hoặc release tag trong task này.

## Kết quả kiểm thử

Máy thực tế: **Intel Iris Xe**, driver **32.0.101.6790**, Windows, Python 3.12.10. Fixture tiếng Anh thật dài 2.461 giây; GPU và faster-whisper CPU đều trả “How are you doing today?”, `en`, segment 0.0 → 2.0 giây. Intel test không có Torch. Tiny model thật đã được download, GPU verify, tạo manifest và atomic promote trong thư mục test biệt lập.

| Kiểm tra | Kết quả |
| --- | --- |
| `npm run build` | Thành công, TypeScript strict + Vite |
| `npm test` | 23 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 105 passed, 3 hardware encoder tests ignored |
| `python -m unittest discover -s python/tests -v` trong release venv | 52 passed, 5 hardware tests skipped |
| Python stdlib-only để mô phỏng CI thiếu package AI | 43 passed, 14 optional/hardware tests skipped |
| Intel Whisper integration opt-in | 1 passed, inference GPU thật |
| Worker cache/release script tests | 3 Python + 4 Node passed |
| Staging optional source | Thành công |
| Môi trường sạch OCR + Whisper Intel | pip check, real GPU ASR, không Torch: passed |

Unit tests kiểm tra resolver priority/manual selection, backend/model compatibility/integrity, Unicode/time validation, load-once/window/offset/dedup, language, progress, cancellation, fallback, và giữ output cũ khi thất bại. Test GPU cần opt-in và model local; CI không tải model.

Warmed benchmark tiny trên clip ngắn: load khoảng **1.16s**, inference **0.34s**, RTF inference **0.14**. Hardware integration lần cuối RTF inference **0.121**. Môi trường sạch chưa cache: load **5.11s**, inference **0.94s**. Worker cold có thể mất khoảng **14.8s** do compile/startup; warmed worker khoảng **4.7s**. CPU tiny worker cùng clip khoảng **1.17s**. Vì vậy không đảm bảo GPU nhanh hơn trên mọi clip ngắn; phải tách thời gian load và inference khi so sánh.

Có fixture lặp speech/silence ~59 giây để kiểm tra bounded windows/progress, không dùng làm bằng chứng chất lượng thoại tự nhiên/video một giờ.

### Chạy tests và benchmark

```powershell
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
$env:PYTHONPATH = (Resolve-Path python).Path
python -m unittest discover -s python/tests -v

# Model/audio phải có sẵn, các lệnh sau không tải model.
$env:JC_AI_TEST_WHISPER_OPENVINO = '1'
$env:JC_AI_TEST_WHISPER_MODEL = 'D:\fixtures\whisper-tiny-openvino'
$env:JC_AI_TEST_WHISPER_AUDIO = 'D:\fixtures\speech-16k-mono.wav'
python -m unittest discover -s python/tests -p test_asr_openvino_integration.py -v
python scripts/benchmark-whisper-openvino.py --model-dir $env:JC_AI_TEST_WHISPER_MODEL --audio $env:JC_AI_TEST_WHISPER_AUDIO
```

Hardware opt-in yêu cầu interpreter đã cài optional dependencies; nếu thiếu runtime/GPU/model thì skip. `scripts/test-openvino-runtime.py --whisper-model-dir ... --audio ...` tạo môi trường tạm sạch và kiểm tra cài chung OCR/Whisper; lệnh này có tải Python packages, không tải AI model.

## Giới hạn và bước tiếp theo

- Chỉ **tiny tiếng Anh trên Iris Xe** đã được kiểm tra inference thực tế. Base/small được pin và validate format/size; chưa chạy chất lượng trên máy này. Intel Arc, máy NVIDIA cùng cài Intel, các language zh/vi/ja/ko và video thoại tự nhiên dài 1–2 giờ chưa được kiểm tra end-to-end.
- Intel chưa có word timestamps/WhisperX alignment, medium/large chưa hỗ trợ. Biên window rất dài hoặc câu bị model bỏ end timestamp có thể báo lỗi; cần benchmark thực tế trước khi đổi chính sách merge.
- First-load compile có overhead. Runtime clone staging cần dung lượng tạm, uv cache giữ lại để tránh tải lặp.
- Đã build frontend, compile/test Rust và stage optional resources; **chưa build installer đầy đủ và chưa kiểm thử bằng GUI của bản cài** trong task này.
- Tests logic/regression đã qua; không khẳng định TTS/BandIt/CUDA thực tế đã được kiểm tra trên phần cứng không có ở đây.

Đề xuất Task 4: kiểm thử bộ audio zh/vi/en/ja/ko (tiny/base/small), video phim dài với thoại sát biên 30 giây, benchmark warm/cold trên Iris Xe/Arc/NVIDIA, rồi kiểm thử bản cài sạch (CPU-only → Intel OCR → Intel Whisper, và CUDA + Intel coexistence). Đánh giá timestamp/omission/WER và memory trước khi mở thêm medium/large.

## Tham chiếu

- [OpenVINO GenAI speech recognition](https://openvinotoolkit.github.io/openvino.genai/docs/use-cases/speech-recognition/)
- [OpenVINO Whisper tiny FP16 IR](https://huggingface.co/OpenVINO/whisper-tiny-fp16-ov)
- [OpenVINO Whisper base FP16 IR](https://huggingface.co/OpenVINO/whisper-base-fp16-ov)
- [OpenVINO Whisper small FP16 IR](https://huggingface.co/OpenVINO/whisper-small-fp16-ov)
- [uv constraint path issue](https://github.com/astral-sh/uv/issues/12639)
