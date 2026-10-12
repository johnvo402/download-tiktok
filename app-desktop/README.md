# JC AI

Ứng dụng desktop Windows cho quy trình bản địa hóa video: import/playback, transcript, translation, VieNeu-TTS, blur/logo và FFmpeg export. Hướng dẫn người dùng nằm trong [USER_GUIDE.md](USER_GUIDE.md); build developer ở [docs/BUILD_WINDOWS.md](docs/BUILD_WINDOWS.md).

Phiên bản **0.1.15** thêm timeline nhiều nguồn/lớp trong **Dịch**: cắt/trim,
di chuyển, undo/redo, ảnh/chữ/phụ đề/âm thanh, canvas/crop, fade, keyframe và
chuyển cảnh. Transcript, bản dịch và giọng bám từng video nguồn. Preview phát
trực tiếp; Export chụp trạng thái đã lưu và hỗ trợ toàn bộ timeline hoặc một
vùng. Xem [cách dùng timeline](USER_GUIDE.md#dựng-timeline-nhiều-nguồn--015)
và [kiểm tra bản phát hành](docs/RELEASE_0.1.15.md).

## Yêu cầu phát triển

- Node.js 22.12+ và npm
- Rust stable qua rustup
- Microsoft C++ Build Tools cùng Windows SDK trên Windows
- Microsoft Edge WebView2 Runtime
- FFmpeg + FFprobe trên `PATH` chỉ cho development; release tự đóng gói chúng
- Python 3.10+ (development worker; khuyến nghị Python 3.12)

### Cài toolchain trên Windows

Rustup cài đồng thời `rustc`, `cargo` và `rustup`; cài bằng hướng dẫn chính thức tại [rust-lang.org/tools/install](https://rust-lang.org/tools/install/), sau đó chạy:

```powershell
rustup default stable
rustc --version
cargo --version
```

Cài FFmpeg bằng winget:

```powershell
winget install --id Gyan.FFmpeg --exact
ffmpeg -version
ffprobe -version
```

Nếu winget chưa thêm executable vào `PATH`, mở lại terminal hoặc thêm thư mục `bin` của FFmpeg vào `PATH`. Development dùng FFmpeg/FFprobe từ `PATH`; release dùng binaries đã ghim và xác minh checksum trong `src-tauri/resources/ffmpeg`.

Tauri trên Windows cũng cần workload **Desktop development with C++** và Windows SDK trong Visual Studio Build Tools. Chọn workload này trong Visual Studio Installer theo [hướng dẫn Microsoft](https://learn.microsoft.com/en-us/cpp/build/vscpp-step-0-installation).

Trên macOS, cài FFmpeg bằng `brew install ffmpeg`; trên Debian/Ubuntu, dùng `sudo apt install ffmpeg`. Sau cài đặt kiểm tra cả `ffmpeg -version` và `ffprobe -version`.

## Chạy ứng dụng

```powershell
npm install
npm run tauri -- dev
```

`npm run dev` chỉ chạy giao diện trong trình duyệt để phát triển CSS/UI; quyền project filesystem chỉ hoạt động trong ứng dụng Tauri.

Dev dùng Python trong `.venv` và FFmpeg/FFprobe trên `PATH`, không cần build
worker thành `.exe`. Build installer cần các worker đã đóng gói bằng
`scripts/build-worker.ps1`.

Khi chạy dev, toàn bộ dữ liệu ứng dụng (project, settings, model, runtime AI,
cache, log và hồ sơ WebView) mặc định lưu trong `.app-data/` ở gốc repo,
đã được Git bỏ qua. Đổi nơi lưu bằng biến môi trường trước khi chạy:

```powershell
$env:JC_AI_DATA_DIR = "D:/JC AI/dev-data"
npm run tauri -- dev
```

Hoặc sao chép `.env.example` thành `.env` rồi sửa `JC_AI_DATA_DIR`.
Thứ tự ưu tiên: biến môi trường → `.env.local` → `.env` → `.app-data/`.
Đường dẫn tương đối tính từ gốc repo. `JC_QA_DATA_DIR` vẫn được ưu tiên cao
nhất cho các script kiểm thử. Bản release đọc nơi lưu đã chọn từ installer,
không đọc các biến dev hay `.env`. Đổi nơi lưu không tự di chuyển dữ liệu cũ.

Preview trên Windows dùng renderer wgpu chung với Export. Kiến trúc, tùy chọn tương thích và kết quả kiểm thử nằm trong [docs/NATIVE_PREVIEW_ENGINE.md](docs/NATIVE_PREVIEW_ENGINE.md).

### Python worker / downloader / Speech Recognition (development only)

Tạo môi trường riêng ở thư mục gốc repository và cài worker cho ASR cùng TikTok Drama Downloader:

```powershell
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install --upgrade pip
.\.venv\Scripts\python.exe -m pip install -e ".\python[downloader]"
```

Lệnh này cài faster-whisper cùng dependency cho TikTok Drama Downloader (Playwright và curl-cffi). Downloader dùng Chrome đã cài để mở series, nhận diện URL video trực tiếp từ trang và truyền dữ liệu vào file theo từng khối. Worker giao tiếp với Rust qua JSON Lines trên stdin/stdout; log chẩn đoán chỉ đi qua stderr. Nếu dùng một Python khác, đặt biến `LOCALIZATION_PYTHON` tới executable trước khi chạy Tauri. FFmpeg dùng executable `ffmpeg`/`ffprobe` trên `PATH`.

Model faster-whisper không nằm trong installer hoặc repository. Khi người dùng bấm **Generate Transcript**, ứng dụng kiểm tra model cache và hỏi xác nhận trước lần tải đầu tiên; model được lưu trong app-cache dùng chung cho các project để tránh đường dẫn cache quá dài trên Windows. Mặc định là `small`, `Auto`, và tự nhận dạng ngôn ngữ. Chọn CPU nếu CUDA không có hoặc không tương thích.

WhisperX là chế độ tùy chọn để căn chỉnh timestamp theo ngôn ngữ sau bước nhận dạng. Nó dùng chung model ASR faster-whisper; aligner và dữ liệu tokenizer được cài riêng từ **Settings → Speech Recognition** và lưu cạnh cache ASR. Cần chọn ngôn ngữ cụ thể thay vì `Auto`. Runtime WhisperX/PyTorch không nằm trong worker mặc định vì dung lượng lớn; trong development có thể cài bằng `./scripts/setup-whisperx-dev.ps1`. Sau khi cài, bấm **Kiểm tra lại**, chọn **WhisperX**, rồi tải aligner cho ngôn ngữ. Tạo transcript không tải model ngầm. Worker release hiện tại vẫn là bản CPU gọn và chưa đóng gói WhisperX.

### Translation providers

Mở **Workspace → Cài đặt → Translation** để chọn provider và cấu hình. Các API key được lưu bằng credential manager hệ điều hành (Windows Credential Manager, macOS Keychain hoặc secret service tương ứng), không nằm trong `app-settings.json`, `project.json` hay transcript. Nút **Test Connection** thực hiện request dịch thực tế.

- **Gemini:** nhập API key từ Google AI Studio, chọn model ID khả dụng, rồi chạy Test Connection. Hướng dẫn API: [Gemini structured output](https://ai.google.dev/gemini-api/docs/generate-content/structured-output).
- **Google Cloud Translation:** bật Cloud Translation Basic API v2 trong Google Cloud project, nhập API key có giới hạn phù hợp, rồi chạy Test Connection. Cấu hình hiện tại dùng model NMT và gửi tối đa 100 segment trong mỗi request: [API v2 translate](https://docs.cloud.google.com/translate/docs/reference/rest/v2/translate).
- **Codex CLI:** cài Codex CLI và đăng nhập bằng `codex login`, hoặc nhập đường dẫn trực tiếp tới executable. Ứng dụng chạy `codex exec` non-interactive, truyền transcript qua stdin, tắt user MCP/config, tắt shell tool và dùng sandbox read-only. Test Connection gọi provider thật và có thể tiêu thụ mức sử dụng Codex.

Batch size mặc định là 30 segment; giới hạn request mặc định là 20 KB. Context trước/sau được gửi cho Gemini và Codex; Google Cloud Translation không có context prompt. Kết quả từng batch được lưu ngay sau khi xác thực ID, nên batch lỗi không làm mất các batch trước. Bản dịch sửa tay có trạng thái `edited`; Translate Missing giữ nguyên chúng, còn các thao tác ghi đè yêu cầu xác nhận.

### VieNeu-TTS và lồng tiếng (development only)

VieNeu SDK là dependency tùy chọn. Sau khi cài Python worker cơ bản, mở **Cài đặt → Voice / TTS**. Để dùng CPU/ONNX trong development mà không cài thêm Gradio, Librosa và các dependency UI/legacy không dùng, chạy từ PowerShell:

```powershell
./scripts/setup-vieneu-dev.ps1
```

Lệnh đầy đủ của extra upstream (`pip install -e ".\python[tts]"`) cũng được hỗ trợ nhưng cài thêm các dependency UI/legacy mà app không dùng.

Sau khi package sẵn sàng, nhấn **Tải model VieNeu-TTS** nếu model chưa có. Model và ONNX codec được lưu một lần trong app-data dùng chung, không nằm trong project hoặc installer. Gói CPU mặc định chạy ONNX và không cần PyTorch, CUDA hay C++ build workload cho VieNeu. Môi trường GPU Windows cần cài PyTorch/TorchAudio tương thích CUDA trước, theo [hướng dẫn hiện hành của VieNeu-TTS](https://github.com/pnnbao97/VieNeu-TTS). VieNeu v3 Turbo dùng audio 48 kHz, có preset voice tích hợp và hỗ trợ reference clip 3–8 giây; danh sách preset lấy từ catalog cài cùng SDK. Repository và model card hiện công bố Apache 2.0 ([repository chính thức](https://github.com/pnnbao97/VieNeu-TTS), [model card](https://huggingface.co/pnnbao-ump/VieNeu-TTS-v3-Turbo)).

Để thêm voice profile, chọn audio tham chiếu 3–8 giây và xác nhận bạn có quyền sử dụng giọng đó. Ứng dụng kiểm tra stream, chuẩn hóa bản sao WAV mono 48 kHz vào app-data, và chỉ lưu ID profile trong project. Không tự động clone giọng từ video nguồn.

Mỗi segment đã dịch sinh một WAV riêng ở `projects/{projectId}/audio/tts/`; metadata và đường dẫn tương đối được lưu trong transcript. **Natural** giữ thời lượng tự nhiên, **Fit** tăng tốc tối đa 1.20× rồi báo Duration Overflow nếu vẫn không vừa, và **Custom** áp dụng tốc độ 0.85–1.20× đã chọn. Dubbing preview bỏ qua segment bị overflow để tránh phát chồng hoặc cắt tiếng; có thể preview riêng hoặc chạy Fit lại. Preview không render video đầu ra.

## Windows release

Sau khi cài toolchain build, tạo installer Windows x64 bằng một lệnh:

```powershell
./scripts/build-release.ps1 -Python python
```

Người dùng installer không cần Node.js, Rust, Python hoặc FFmpeg. AI models tải theo lựa chọn trong Settings, không nằm trong installer. Xem [docs/BUILD_WINDOWS.md](docs/BUILD_WINDOWS.md), [docs/SYSTEM_REQUIREMENTS.md](docs/SYSTEM_REQUIREMENTS.md) và [docs/MANUAL_E2E_TESTS.md](docs/MANUAL_E2E_TESTS.md).

## Build và kiểm tra

```powershell
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
python -m unittest discover -s python/tests -v
```

Các test Rust tạo fixture tạm bên trong thư mục tạm biệt lập; không đọc hoặc xóa project của người dùng.

Test provider API không tự gọi mạng hoặc phát sinh chi phí; chạy kiểm tra thật từ **Test Connection** sau khi cấu hình credentials. Test Codex CLI từ UI cũng tạo một request dịch thực tế.

## Kiến trúc chính

```text
src/
├── app/                         # Application shell, state và typed Tauri service
│   └── services/                 # Project và translation invoke clients
├── features/
│   ├── projects/                 # Start page, project dialogs và project types
│   ├── transcript/               # Transcript models, validation và test helpers
│   └── workspace/                # Sidebar, video player, transcript/media/properties và settings
├── shared/components/            # Brand, modal, toast
└── styles.css                    # Theme tokens và desktop layouts
src-tauri/src/
├── commands.rs                   # Tauri command entry points
├── project_service.rs             # Metadata, app-managed folders và kiểm tra an toàn
├── video_service.rs               # FFprobe, import staging, video paths và preview proxy
├── transcript_service.rs          # SRT/VTT parsing, validation, atomic persistence
├── translation_service.rs         # Gemini, Google Cloud, Codex CLI, batching và jobs
├── settings_service.rs            # Global settings và OS credential manager
├── asr_service.rs                  # Python worker jobs, cancellation, progress events
└── tts_service.rs                  # Model, profiles, TTS jobs, fit, cache và audio safety
python/
├── pyproject.toml
├── tests/
└── worker/
    ├── asr/                       # faster-whisper adapter
    └── tts/                       # JSONL TTS service và VieNeu provider adapter
```

Project nằm trong `appDataDir/projects/{UUID}`. Video được copy vào `media/source`, metadata nằm trong `project.json`, transcript đầy đủ nằm trong `subtitles/transcript.json`. Rust quản lý filesystem/processes; frontend chỉ truyền project ID, đường dẫn do native file picker/drag-drop trả về và nội dung transcript cần lưu. Player dùng Tauri asset protocol chỉ cho phép media trong app-data project và cache preview. Xóa từ chối ID không phải UUID, thư mục ngoài vùng quản lý, `project.json` dạng symlink và project tree có symlink/reparse point; transcription/translation jobs của project được hủy trước khi xóa.

## Luồng kiểm thử thủ công: video và transcript

1. Tạo project ở Project Manager.
2. Trong **Dịch**, chọn hoặc kéo thả MP4/MOV/MKV/WEBM. Xác nhận thay thế nếu project đã có video.
3. Mở **Media** để kiểm tra metadata, audio streams và subtitle tracks.
4. Chọn text subtitle track rồi **Extract Subtitle**, hoặc cấu hình model/device/language rồi **Generate Transcript**. Lần tải model đầu tiên yêu cầu xác nhận.
5. Chọn segment để seek video; sửa text/timestamp, thêm hoặc xóa segment.
6. Đợi trạng thái **Đã lưu**, đóng ứng dụng, mở lại project và kiểm tra video/transcript đã phục hồi.

## Luồng kiểm thử thủ công: dịch và subtitle preview

1. Trong **Cài đặt → Translation**, chọn provider, thêm credential/executable, lưu cấu hình và chạy **Test Connection**.
2. Mở project có transcript, đặt source/target language rồi nhấn **Lưu ngôn ngữ**.
3. Thử **Translate Missing**, **Translate All**, chọn một số dòng rồi **Translate Selected**, và dùng nút dịch lại ở mỗi segment. Thao tác ghi đè bản dịch `edited` sẽ hỏi xác nhận.
4. Theo dõi batch/progress và thử Cancel; các batch đã lưu vẫn được giữ.
5. Sửa source và bản dịch thủ công; source thay đổi đánh dấu bản dịch cần review, bản dịch chỉnh tay chuyển thành `edited`.
6. Bật/tắt translated subtitle trong **Properties**, chỉnh cỡ chữ, màu, viền và lề dưới; seek video vào/ra timestamp để kiểm tra đồng bộ.
7. Đợi autosave, đóng ứng dụng, mở lại project và xác nhận transcript, bản dịch và style vẫn còn.

## Luồng kiểm thử thủ công: tạo và phát dubbing

1. Tạo hoặc mở project, nhập video, tạo/trích xuất transcript rồi dịch ít nhất một segment.
2. Vào **Cài đặt → Voice / TTS**. Cài `vieneu` như trên nếu chưa có; nhấn tải model và xác nhận. Không cần cài model để build hoặc chạy unit tests.
3. Chọn CPU/Auto, kiểm tra preset voice thật từ SDK, lưu settings và nghe thử. Có thể thêm reference voice profile 3–8 giây sau khi xác nhận quyền sử dụng.
4. Quay lại **Dịch**, chọn một segment đã dịch và bấm tạo audio; nghe thử riêng, thử Fit Audio và kiểm tra duration/slot.
5. Trong **Properties → Âm thanh lồng tiếng**, chọn voice, tốc độ/Fit mode, bật dubbing, chỉnh âm thanh gốc/dub, mute original và ducking.
6. Play, pause, resume, seek vào giữa segment và thử đoạn bị overflow. Preview segment dừng video dubbing để hai chế độ không chồng tiếng.
7. Sửa bản dịch, xác nhận audio cũ chuyển thành Outdated, tạo lại; đợi autosave, đóng/mở app/project và kiểm tra audio cùng cài đặt đã phục hồi.

Unit tests không tải model và không gọi VieNeu thật. VieNeu inference end-to-end cần cài package, xác nhận tải khoảng 1.8 GB model/codec trong app, và có FFmpeg/FFprobe trên PATH. Model TTS không được bundle; blur/logo và export đã có trong ứng dụng.

## Audio Engine v2 (Task 7)

Preview Windows và export dùng chung Rust audio mixer. Kiến trúc, kết quả kiểm thử, benchmark và giới hạn hiện tại: [TASK_07_AUDIO_ENGINE.md](TASK_07_AUDIO_ENGINE.md).


## Production stability — Task 10

Bản beta `0.1.14-beta.1` thêm backup/khôi phục JSON, kiểm tra lưu khi đóng,
lịch sử job gián đoạn, export an toàn và chẩn đoán có lọc thông tin riêng tư.
Xem [báo cáo và release gates](docs/TASK_10_PRODUCTION_STABILITY.md).
Không coi kết quả trên CPU/Intel là kiểm thử NVIDIA/AMD hoặc cài đặt trên máy sạch.
