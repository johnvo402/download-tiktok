# Task 9 — Runtime và đóng gói

Ngày kiểm tra: 2026-10-04. Branch `dev`; app `0.1.13-preview`, AI worker `0.1.7`, downloader worker `1.0.0`, protocol `2`, runtime ABI `7`.

Task 8 đã được commit thành `7a13725`. Task 9 là các thay đổi tiếp theo, chưa publish hoặc push tag.

## 1. Kiến trúc trước / sau

Trước đây một frozen Python worker chứa cả ASR, OCR, TTS và browser downloader. Dependency downloader có thể làm worker AI không khởi động được. Optional runtime dùng cùng một vùng CPU/CUDA/Intel; mỗi lần bổ sung feature có thể cài lại cả tập dependency.

Hiện tại:

```text
Tauri / WorkerManager
├── ai-worker.exe: faster-whisper CPU, RapidOCR CPU, VieNeu ONNX CPU
├── downloader-worker.exe: Playwright, curl-cffi, TikTok browser/session
├── advanced/current: Torch CPU hoặc CUDA + feature đã chọn
└── intel/current: OpenVINO + OCR/Whisper Intel đã chọn
```

Hai frozen worker có version, môi trường build, fingerprint, inventory và capability smoke riêng. Downloader không nằm trong AI hoặc optional AI source. Không thay đổi thuật toán nhận dạng, dịch, TTS, STTN, audio fit hoặc render video.

## 2. Files và dependency

Nhóm file chính:

- `python/jc_worker_protocol.py`, `worker/__main__.py`, `downloader_worker/**`: protocol stdlib và dispatcher lazy.
- `python/requirements-release.{in,txt}`, `requirements-sdk.txt`: closure CPU; `requirements-downloader.{in,txt}`: closure downloader riêng.
- `python/packaging/{ai_worker,downloader_worker}.spec`, `bundle_helpers.py`, `check_dependencies.py`, `write_worker_inventory.py`: PyInstaller, metadata và licenses.
- `python/runtime-matrix.json`, `worker/runtime_dependencies.py`: graph component, version pins và dependency reachability khi gỡ.
- `src-tauri/src/{worker_manager,runtime_registry,optional_runtime,runtime_paths,runtime_health}.rs`: lifecycle, routing, health, install plan, transactional install/uninstall.
- Các service ASR/TTS/OCR/STTN/BandIt/TikTok và `commands.rs`, `lib.rs`: sử dụng worker phù hợp, cleanup khi đóng app.
- `src/app/App.tsx`, `StartPage.tsx`, runtime services, các runtime card Settings, `styles.css`, `vite.config.ts`: lazy route, kiểm tra nhanh khi startup, dung lượng và kiểm tra/gỡ feature.
- `scripts/{build-worker,build-optional-runtime,build-release}.ps1`, `worker-cache.py`, audit/smoke scripts; workflow GitHub release; tests; benchmark JSON trong `docs/benchmarks/`.

CPU closure giữ faster-whisper 1.2.1, CTranslate2 4.8.2, PyAV 18.1.0, ONNX Runtime 1.30.0, RapidOCR 3.9.2, VieNeu 3.8.3, NumPy 2.5.3, headless OpenCV 5.0.0.93, sea-g2p 0.10.0, SoundFile 0.14.0, soxr 1.1.0 và các dependency trong lock file. Downloader dùng Playwright 1.63.0, curl-cffi 0.16.3 và dependency riêng. Không nâng version model SDK để lấy số liệu tối ưu giả.

Không đóng gói Torch, WhisperX, OpenVINO, browser, Gradio hoặc server UI trong core AI. VieNeu ONNX dùng SoundFile/soxr thay đường librosa khi librosa không có. Checker chỉ cho phép các ngoại lệ CPU đã kiểm chứng; lỗi dependency khác vẫn làm build thất bại. Optional NVIDIA giữ dependency SDK đầy đủ, gồm Gradio/librosa nếu SDK yêu cầu, cho đến khi kiểm chứng được việc loại bỏ trên GPU.

## 3. Optional graph và storage

```text
Torch CPU/CUDA (dùng chung)
├── WhisperX
├── BandIt
├── STTN
└── NVIDIA ASR/OCR/TTS

OpenVINO (dùng chung, độc lập Torch)
├── RapidOCR Intel
└── Whisper Intel
```

Matrix pins Torch/torchaudio 2.8.0, torchvision 0.23.0, torchcodec 0.7.0, WhisperX 3.8.6, Transformers 4.57.6, Hugging Face Hub 0.36.2, ORT CPU 1.30.0 / GPU 1.26.0, OpenVINO 2026.4.0. CPU và GPU ORT không giữ đồng thời các file provider xung đột. CPU → CUDA vẫn cần rebuild staging vì ABI khác; cache tải được giữ.

Đường dẫn được resolve bằng backend, không hardcode ổ đĩa:

```text
app-data/
├── models/                  # giữ nguyên trọng số hiện có, không xóa khi gỡ runtime
│   ├── fw/
│   ├── huggingface/
│   ├── rapidocr/
│   ├── whisper/             # model OpenVINO
│   └── text-removal/
└── runtimes/
    ├── advanced/
    │   ├── current/, staging/, previous/
    │   ├── current.json, installation.lock
    │   ├── python/          # Python managed dùng chung
    │   └── cache/           # uv cache dùng chung; không tự xóa sau install
    └── intel/
        ├── current/, staging/, previous/
        └── current.json
```

Cài feature vào pack tương thích: copy độc lập current → staging, freeze package đang dùng + constraint matrix, chỉ install feature mới, kiểm tra dependency và toàn bộ feature, rồi promote. Không dùng hard link từ staging vào current. Cache dùng chung giúp không tải Torch lại khi version không đổi; staging vẫn tốn thêm dung lượng đĩa để rollback an toàn. CPU pack STTN/BandIt đo được khoảng 3.55 GB; riêng Torch CPU 3.15 GB, trong đó `dnnl.lib` khoảng 2.33 GB. Ước tính installed CPU component đã điều chỉnh lên 3.3 GB theo phép đo. Chưa xóa linker libraries/headers của optional Torch vì chưa kiểm tra mọi provider hoặc CUDA; đây là giới hạn dung lượng còn lại cần đánh giá ở Task 10.

Install plan hiển thị component thiếu/dùng lại, dung lượng ước tính download/install và staging trước khi xác nhận. Đây là ước tính, không phải số byte thực tế từ server; models tính riêng. Khi feature đã có, download estimate bằng 0.

Gỡ feature trong staging bằng graph metadata gồm dependency bắc cầu và extras; giữ package còn được feature khác sử dụng. Cuối cùng verify feature sống sót rồi promote. Không gỡ models, managed Python hoặc UV cache. UV tạo alias junction bên trong `python/`; quá trình gỡ chỉ kiểm tra cây venv và thư mục cha được quản lý, không đi theo alias này.

Khóa trong app và khóa file Windows ngăn hai lượt install/uninstall đồng thời. Promotion bị chặn nếu một worker đang dùng current. Runtime cũ có marker ABI cũ không bị tự xóa; được sửa khi người dùng yêu cầu. Pack Intel riêng ưu tiên hơn pack cũ chứa Intel. Startup phục hồi previous nếu directory/marker switch chưa commit; marker + manifest của current đã commit thì cleanup previous.

ANTLR và BandIt được build thành pure-Python wheel trong môi trường build, kèm filename/SHA-256 và nguồn pinned. Người dùng không cần compiler để build hai source distribution này. Bộ cài xác minh wheel trước khi dùng. Không tin filename/path tùy ý.

## 4. Startup, protocol và health

Startup Project Manager chỉ kiểm tra file và storage; không khởi động model hoặc tải dependency. Settings có deep health và kiểm tra session/backend thực tế. Settings/Workspace được tải bằng route lazy; Tailwind chỉ quét source UI, Vite bỏ qua venv/resource/cache.

Worker chỉ khởi động khi có command. Dispatcher import ASR/TTS/OCR/browser sau khi đọc operation; ping/health/version dùng stdlib, không import model. JSONL UTF-8 chứa protocolVersion, workerVersion, workerKind và requestId. Debug/log ghi stderr. Request sai không phá request tiếp theo; request vượt 8 MiB bị từ chối. Surrogate lỗi được xử lý trước serialize.

WorkerManager kiểm tra ready trong 30 giây, giới hạn ready reply 64 KiB; bounded output helper giới hạn stdout/stderr còn 8 MiB mỗi stream và có deadline. Giữ timeout/cancel riêng của tác vụ hiện có. Windows Job Object quản lý cả process tree, giải phóng khi job/app kết thúc; không giữ model process thường trú ở idle. Pipe của process con không được giữ app đợi EOF vô hạn. Một số reader streaming cũ vẫn theo timeout riêng của service; chưa thay mọi reader thành cùng một bounded parser.

Release gọi frozen exe hoặc Python của pack đã chọn bằng arguments, không fallback Python toàn cục/PATH. Intel OCR/Whisper không bị chuyển nhầm sang runtime NVIDIA. Capability của các pack được kiểm tra và ghép, giữ nguyên lựa chọn backend đang dùng.

## 5. Đóng gói và licenses

AI spec giữ VAD assets và `sea_g2p.bin` cần cho phát âm VieNeu. Loại demo/tests, RapidOCR weights đi kèm SDK, browser dependencies và plugin OpenCV videoio FFmpeg không được worker dùng. OpenCV vẫn giữ để xử lý ảnh; video dùng PyAV/FFmpeg. Không thay thế FFmpeg hoặc xóa DLL chỉ vì dung lượng lớn.

License inventory lấy các distribution thực tế của PyInstaller Analysis, sao chép metadata/licenses tương ứng, gồm CPython và ngoại lệ bootloader PyInstaller. Optional pack có inventory theo package đã cài sau verify. Rust/npm notices được sinh lại; FFmpeg notices giữ nguyên.

CI cache nhận diện cả hai worker; unit CI cài PyAV/NumPy nhẹ để chạy tests media, không tải model AI. Tag app không làm đổi worker version hoặc tải model. Các wheels và bộ cài được tái tạo bởi build script, không commit binary lớn.

## 6. Số liệu thực tế

Các con số dưới đây là bytes; artifact hashes/timestamps và package inventory nằm trong benchmark JSON.

| Hạng mục | Trước | Sau |
| --- | ---: | ---: |
| AI worker | 567,975,039 | 441,930,221 |
| Downloader worker | chung trong AI | 138,065,572 |
| Tổng hai worker | 567,975,039 | 579,995,793 |
| Thư mục worker/FFmpeg/optional | 961,631,249 | 973,831,073 |
| Installer NSIS | 389,095,663 | 368,267,204 |
| JS entry | khoảng 507 KB | 99.22 KB |
| Debug Project Manager ready | 61,060.5 ms | 3,853.9 ms |
| Debug project read command | 12.26 ms | 11.37 ms |
| Debug process-tree RSS | 636,559,360 | 704,233,472 |

AI worker nhỏ hơn khoảng 22.2%, nhưng tổng hai worker tăng khoảng 2.1%. Kiểm thử synthesize thật phát hiện bộ đóng gói cũ thiếu từ điển phoneme `sea_g2p.bin` 62,829,820 bytes; bản mới phải giữ file này để TTS thực sự hoạt động. Không được loại nó chỉ để làm đẹp số liệu.

Installer baseline đã được tạo trước Task 9; không có tree đã cài của bản cũ để đo tổng installed size chính xác. Resource bytes chỉ cộng các thư mục worker/FFmpeg/optional, không gồm notices ở resource root, app exe/WebView/models/cache; không phải toàn bộ dung lượng sau cài. Payload installer mới sau extract, không tính $PLUGINSDIR: 1,008,607,924 bytes / 1,905 files. Con số này chưa tính uninstaller sinh lúc cài, WebView2, models và optional runtime. Installer cuối SHA-256 `e41dff580f5c05bfc077ba8957b0ce5216abeb9baf174dfb15dd8c1aea455741`, NSIS unsigned; tất cả inventory/seal của worker và các binary/resource chính khớp build. Executable khớp build ngoại trừ đúng 3 bytes marker bundle `UNK` → `NSS` do Tauri đóng gói NSIS. Xem `task09-installer-payload.json`.

Startup đo debug app + Vite, không flush OS/disk cache; các lượt đo có tải CPU build khác nhau. Vì vậy không kết luận production khởi động nhanh hơn 94%. App native RSS vẫn khoảng 35–37 MB, phần tăng là WebView; chưa chứng minh giảm idle RAM. Worker startup là closed-stdin/no-model warm launches trong JSON, không phải thời gian load model. Native files trùng giữa hai worker khoảng 18.24 MB được giữ để môi trường frozen độc lập; chưa chia sẻ DLL khi chưa verify loader/ABI. Median closed-stdin: AI trước 0.237s, AI sau 0.308s, downloader 0.179s. Không xem sai khác này là tốc độ inference. Installer giảm 5.35% so với artifact baseline, trong khi resource bytes tăng 1.27%. Tách worker chủ yếu cô lập lỗi/dependency và tránh tải browser vào tác vụ AI.

## 7. Kiểm thử và cách chạy

```powershell
npm test
npm run test:ci
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
$env:PYTHONPATH = (Join-Path (Get-Location) 'python')
python/.release-ai-venv/Scripts/python.exe -m unittest discover -s python/tests -v
python -m unittest discover -s scripts/tests -p 'test_*.py' -v
./scripts/build-release.ps1 -Python python
```

Kết quả:

- Frontend strict TypeScript + Vite thành công; 25 tests frontend và 4 tests CI tooling qua.
- Rust: 155 passed, 22 ignored, 0 failed; không còn warning trong build mới.
- Python core: 84 tests, 77 chạy qua, 7 skip; worker cache: 4 qua. Skip có lý do model/runtime tùy chọn không cài.
- STTN thật trên CPU: 1 integration test qua, CUDA test skip vì không có NVIDIA. Legacy/cache pixel comparison qua.
- Frozen worker từ installer đã extract, PATH chỉ System32 và đường dẫn chứa dấu tiếng Việt: ASR/OCR/TTS thật, downloader capability và FFmpeg export/FFprobe đều qua. Đây là kiểm tra payload tương đương, không phải chạy toàn bộ installer trên máy sạch.
- Frozen ASR medium CPU và RapidOCR Tiny CPU trên fixture tiếng Việt/Trung: text + timestamp khớp với môi trường source cũ. VieNeu CPU tạo WAV PCM 48 kHz hợp lệ; duration có sai khác do synthesis ngẫu nhiên, chưa kết luận chất lượng nghe.
- Tauri debug thực tế: create project → import fixture → sparse OCR → cancel STTN → retry → cache reuse → CPU export → FFprobe qua.
- Intel thực tế: install OCR/OpenVINO → probe Intel Iris Xe GPU → cài lại download estimate 0 → uninstall, giữ feature STTN cũ, qua.
- Shared Torch thực tế: STTN CPU → thêm BandIt → verify → gỡ BandIt → verify STTN, qua. Đã chạy lại từ STTN chưa có BandIt: plan nhận ra Torch dùng lại và feature BandIt còn thiếu. Sửa lỗi kiểm tra UV alias và thêm retry có hạn 8 giây cho Windows access/sharing lock; test khóa thư mục Windows thực tế qua.
- Unit failure coverage: malformed/oversize JSON, thiếu module, protocol mismatch, timeout, process-tree cleanup/pipe inheritance, bounded output, graph extras/shared ownership, broken wheel/path/hash, marker write failure, manifest failure, interrupted directory promotion và rollback.

Benchmark/audit scripts không tự download model. `scripts/core-worker-smoke.py` cần models/fixtures đã cài; `JC_TASK09_FROZEN_ONLY=1` cùng `JC_TASK09_BUNDLE_ROOT` kiểm tra executable từ installer với PATH chỉ System32. `scripts/app-startup-audit.mjs` là QA debug trên root biệt lập; các flag RUNTIME_SMOKE/SHARED_RUNTIME_SMOKE chủ động cài runtime, không đưa vào unit CI phát sinh download lớn.

## 8. Giới hạn và Task 10

Chưa thực hiện: NVIDIA inference/end-to-end WhisperX alignment/BandIt separation, clean Windows installer UI/uninstall, app ký số, TikTok series download live bằng phiên đăng nhập, benchmark production cold-start có kiểm soát, full disk/network interruption vật lý. Unit rollback/write-failure không được xem là thay thế mọi lỗi môi trường thật. Chưa kiểm thử migration mọi runtime ABI cũ trên máy thực; recovery được kiểm chứng bằng fixture và pack ABI 7 hiện có.

Bản preview unsigned dành cho kiểm thử; không có push hoặc phát hành GitHub trong Task 9. Task 10 nên dùng Windows VM sạch cho core app và một máy NVIDIA cho từng tổ hợp WhisperX/VieNeu/OCR/STTN/BandIt, đánh giá nghe/nhìn với video thực, ký installer và đo cold/warm startup, idle RAM, installed size từ cùng commit và điều kiện máy.
