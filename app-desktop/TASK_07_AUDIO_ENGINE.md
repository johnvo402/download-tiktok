# Task 7 — Audio Engine v2

## Phạm vi và trạng thái

Task 6 đã commit tại `33ca23d`. Task 7 bổ sung audio engine dùng chung cho preview Windows và export, giữ nguyên backend VieNeu/BandIt, timestamp subtitle và các renderer video.

Code, build và kiểm thử waveform/desktop đã thực hiện. **Chưa xác nhận chất lượng phát âm bằng nghe trực tiếp và chưa kiểm thử inference BandIt thật**, do máy kiểm thử chưa có runtime/model BandIt. Không coi các kiểm tra số học là kết quả nghe đạt yêu cầu.

## Nguyên nhân mất đầu câu

Ở code trước Task 7, `TranslationView.tsx` tải WAV của mỗi segment bằng HTMLAudioElement rồi tính `elapsed` lúc nhận `loadedmetadata`:

```ts
dubbingAudio.currentTime = elapsed <= 0.25 ? 0 : elapsed;
// Trong lúc phát:
if (Math.abs(dubbingAudio.currentTime - offset) > 0.32)
  dubbingAudio.currentTime = offset;
```

Nếu tải/chuyển file trễ hơn 250 ms, đoạn đầu bị bỏ qua; việc sửa drift có thể bỏ thêm audio. Đây là đường cắt đầu có thể xác định từ code, độc lập với model. Mixer mới không đổi file/seek WAV tại mỗi ranh giới: chỉ seek khi người dùng seek, đọc PCM đúng vị trí và dừng đồng hồ khi chưa có PCM.

Pipeline cũ dùng `atempo` trực tiếp lên audio không có guard. Pipeline mới đặt padding **trước WSOLA**. Test attack xuất hiện ngay sample 0 chứng minh transient còn tồn tại qua xử lý, mix và AAC. Đây là kiểm chứng tín hiệu, không chứng minh model phát âm đúng từng từ.

Đã đọc VieNeu v3 Turbo ONNX đang cài: đường tạo WAV không trim giọng sinh ra; chức năng trim reference voice khác với giọng output. SDK có babble guard tự sinh lại một số câu ngắn. Không thay thuật toán SDK/model và không khẳng định lỗi phát âm gốc đã được sửa.

## Kiến trúc và file

```text
Python VieNeu → raw WAV immutable
  → Rust processing: validate → resample → pad/fade guard → atempo → RMS
  → processed float32 mono 48 kHz + .audio.json
  → Audio Timeline
      original audio + BandIt speech stem + processed voices
  → Rust Mixer.next_block (1024 frames, stereo float32 48 kHz)
      ├─ CPAL/WASAPI bounded queue → audio master clock → video preview
      └─ stream master WAV → FFmpeg AAC once → mux MP4
```

| Module/file | Trách nhiệm |
|---|---|
| `src-tauri/src/audio/signal.rs` | Đọc/validate WAV, peak, RMS, onset/end; từ chối NaN, WAV thiếu dữ liệu và liên kết |
| `audio/processing.rs` | Padding, fit, normalization, atomic write, sidecar, ingest cache cho WAV cũ |
| `audio/timeline.rs` | Resolve file đã kiểm tra, audio/speech intervals, collision, giới hạn cuối video |
| `audio/mixer.rs` | Streaming decode, reduction, ducking, gains, limiter, shared block renderer |
| `audio/transport.rs` | WASAPI qua CPAL, pause/seek, session ownership, audio clock |
| `audio/commands.rs` | Bốn Tauri commands mới |
| `audio/tests.rs` | Unit, FFmpeg/VieNeu integrations, benchmarks |
| `tts_service.rs` | Raw cache, processing cache, validation/commit TTS, progress refit |
| `dialogue_service.rs` | Validate stem trước khi tái sử dụng/commit |
| `render_service.rs` | Render master chung và gửi cùng audio cho CPU, wgpu staging, native GPU mux; bỏ audio graph cũ |
| `preview/manager.rs` | Video lấy clock trực tiếp từ audio manager |
| `translation/audio-preview.ts` | Mở/đóng phiên, control, telemetry; không truyền PCM qua IPC |
| `TranslationView.tsx` | Bỏ WebAudio mix riêng, dùng Rust transport; browser video luôn muted |
| `TranslationPanel.tsx` | Cấu hình nâng cao và giữ nghe thử WAV từng đoạn |
| `project_service.rs`, project types, commands | Defaults tương thích project cũ và lưu cấu hình |
| `ExportDialog.tsx`, render service TS | Cảnh báo collision/overflow |
| `Cargo.toml/lock`, `lib.rs` | Đăng ký audio engine và dependencies |
| `scripts/audio-engine-tts-smoke.py` | VieNeu offline có sẵn, tùy chọn benchmark 100 câu |
| `scripts/audio-preview-smoke.mjs`, `native-preview-smoke.mjs` | Desktop integration trong dữ liệu QA biệt lập |

Dependencies mới: `hound 3.5.1`, `cpal 0.16.0` chỉ Windows. Python không tham gia mixer realtime. API streaming/seek của [Hound](https://docs.rs/hound/latest/hound/struct.WavReader.html) và timestamp callback của [CPAL](https://docs.rs/cpal/0.16.0/cpal/struct.OutputStreamTimestamp.html) được dùng cho đọc WAV/đồng hồ audio.

## Cache, duration và guard

- `audio/tts/raw-{hash}.wav`: raw hash gồm text, ngôn ngữ đích, voice/reference hash, model/revision. Không phụ thuộc speed/fit/padding.
- Processed hash gồm fingerprint cũ (segment/source revision/timing/speed/fit/voice) và cấu hình Audio Engine v2. File output riêng; sidecar `.audio.json` lưu raw/processed signal, trimmed/slot/playback durations, tempo, lead, overflow và settings.
- Không trim speech: `trimmedDuration = raw.duration`. Onset threshold dùng để đo/cảnh báo, không dùng cắt WAV.
- Head mặc định 100 ms, tail 60 ms; fade tối đa 10 ms chỉ trong silence vừa thêm. Không fade 100 ms vào phụ âm.
- Đặt audio sớm `min(head / tempo, segment.start)`. Không timestamp âm, không thay timing subtitle. Đoạn ở 0 s giữ head trong slot.
- Float32 mono 48 kHz cho voice, stereo 48 kHz cho master. WAV cũ khác sample rate/channels được normalize một lần trong cache theo content hash.
- Đổi fit/padding rồi tạo lại audio sẽ dùng raw cache, không gọi model cho những raw đã có. WAV/sidecar bị hỏng bị từ chối hoặc xử lý lại; file tạm tự dọn, raw hợp lệ được giữ.
- Các field mới dùng serde defaults; không đổi project schema 11 hoặc phá project cũ.

## Fit và chất lượng

Natural giữ tốc độ 1×; custom dùng tốc độ người dùng đã chọn. Auto Fit giữ 1× khi câu ngắn; chỉ tăng tốc khi cần. Default max 1,20×, cấu hình 1–2×. Đầu timeline không đủ chỗ đặt head thì tính thêm head vào khung cần fit. `atempo` giữ pitch.

Giọng quá dài vẫn giữ toàn bộ WAV, ghi `durationOverflow` và cảnh báo. Không crop, không ép câu ngắn lấp đầy slot. Audio vẫn có thể dùng khoảng nghỉ đến câu sau theo quy tắc slot hiện có. Collision giữ audio đầy đủ và cảnh báo; speech vượt cuối video bị chặn export, phần silence padding vượt cuối có thể bỏ.

Voice RMS target 0,10, gain giới hạn 0,5–2×, peak không quá 0,92. Master dùng soft knee trên 0,9, giới hạn 0,98, giữ nguyên gain của tín hiệu nhỏ. Log một lần/mixer nếu tổng trước limiter vượt full scale; không tự kéo nhạc nền lên lớn.

## Dialogue reduction và ducking

BandIt worker lấy đúng stem `speech` từ `stems`, không lấy background làm thoại. Shared mixer áp dụng:

```text
bed = (original - speech × reduction) × originalVolume
bed *= 1 - duckingAmount × envelope
master = limiter(bed + summedVoices × dubbedVolume)
```

0%, 50%, 100% có nghĩa giảm tuyến tính amplitude của thoại. Slider không gọi inference lại. Stem được cache theo video hash/model; validate full WAV, stereo 48 kHz, duration sai lệch tối đa 150 ms. Chưa cài BandIt thì control bị khóa, backend trả lỗi rõ ràng.

Ducking lấy speech intervals của processed audio, merge overlap/gap ngắn, attack mặc định 60 ms và release 180 ms. Không nhân nhiều lần mức giảm khi hai câu overlap. Gains/mute/enable thay đổi live không mở lại device/decoder; bật lại voice giữa lúc phát không tiếp tục reader cũ sai timestamp.

## Playback, seek, export

- Queue PCM bounded khoảng tối đa 4096 frames stereo (~32 KiB/85 ms), decoder queue hai block; không đọc 2 giờ WAV vào RAM.
- WASAPI callback không đọc file/gọi Python/chờ FFmpeg. Đếm frame đã gửi, hiệu chỉnh latency thiết bị, dừng clock khi thiếu mẫu.
- Seek xóa software queue, tạo lại stream để bỏ audio cũ trong thiết bị, reset mixer; revision ngăn kết quả seek cũ và EOF cũ ghi đè trạng thái mới.
- Đợi buffer cuối của WASAPI phát xong trước khi pause ở EOF. Clock base/frame/latency đọc cùng snapshot.
- Mỗi session có UUID. Cleanup của component cũ không đóng/control session mới cùng project. Source thay đổi reset vị trí; chỉnh gain/scene không tạo lại phiên.
- Master offline dùng cùng `next_block`, ghi WAV theo block, validate header/rate/channels/duration, có cancellation. AAC chỉ encode một lần ở mux. Export CPU/wgpu/native nhận cùng master.
- Preview listening volume là gain thiết bị; không lưu vào cấu hình export. Original/dub/reduction/ducking là cấu hình mix chung.
- Debug `JC_AUDIO_DEBUG=1` giữ `raw`, `trimmed` (bản raw), `fitted` WAV. File mixed lấy từ test/export master; không bật mặc định production.

## Kết quả kiểm thử

| Kiểm tra | Kết quả |
|---|---|
| npm build | PASS |
| Rust production build | PASS, `cargo build --release`, không warning; executable Windows 29.891.072 bytes, chưa tạo installer |
| Vitest | 25/25 PASS |
| Node CI script tests | 4/4 PASS |
| Cargo suite | 137 PASS, 22 integration/benchmark opt-in ignored, không warning |
| Python unittest | 57 tests: 52 PASS, 5 hardware opt-in skipped |
| FFmpeg attack/padding/fit/raw immutability | PASS |
| Stereo/original/stem seek, decode failure, PCM parity | PASS |
| AAC encode/decode preserves attack; legacy resample cached once | PASS |
| Real Vietnamese raw→natural/fit→mix (4 câu) | PASS về waveform/timing, chưa xác nhận nghe |
| Native GPU export H.264, HEVC, 1080p, dub-only | PASS trên Intel QSV |
| wgpu staging + CPU/QSV encoding + cancel | PASS |
| Desktop Play/Pause/rapid seek/resize/scene | PASS, hardware decode Intel Iris Xe; drift đo khoảng 20–60 ms ở các lần chạy |
| Desktop gain/mute/enable, stale close, pause/EOF, export | PASS hai lần liên tiếp trên bản debug mới với project QA có WAV VieNeu thật; buffer thiết bị được drain ở EOF |
| MP4 tạo từ desktop | FFprobe: H.264 640×360, AAC stereo 48 kHz, 10,000 s |
| Nút nghe thử từng WAV | Thao tác trên desktop không báo lỗi giải mã; không khẳng định nghe/phát âm đã đạt |

Audio tests còn kiểm tra: corrupt/empty/NaN WAV, backward compatibility, source không audio, TTS tắt, cancellation không commit file, peak limiter, reduction 0/50/100%, envelope smooth, bật lại voice, seek flush và stale EOF/session. PCM của preview block renderer và export WAV so sánh **bằng nhau từng sample** trên fixture; test AAC dùng tolerance vì mã hóa lossy.

### Benchmark

Máy Windows CPU + Intel Iris Xe, VieNeu v3 Turbo ONNX/CPU đã cài, không tải model trong test. Có các build khác chạy đồng thời; số liệu là phép đo local, không phải cam kết tốc độ.

| Bước | Kết quả |
|---|---|
| Generate 100 câu (4 câu tiếng Việt lặp ×25), gồm model loading/đo memory | 101,08 s, 100 thành công, không lỗi |
| Tổng thời gian inference từng câu báo bởi worker | 94,42 s |
| Peak working set của worker + descendants (Windows venv có redirector) | 1.430.155.264 bytes (~1,33 GiB) |
| Process 100 raw WAV, debug | 18,30 s |
| Mix 100 processed voices, timeline 400 s, debug | 6,85 s |
| PCM block lớn nhất | 8192 bytes |
| Timeline 2 h silence hoàn chỉnh | 345.600.000 frames, ~0,10 s với fast path zeros |

Test 2 h silence xác nhận không cấp phát buffer toàn timeline; **không phải** benchmark decode/mix video thực dài 2 h. RAM của mô hình VieNeu khác với RAM mixer. Không đo riêng tổng RAM mọi FFmpeg child ở benchmark processing.

## Chạy lại

```powershell
npm run build
npm test
npm run test:ci
cargo test --manifest-path src-tauri/Cargo.toml
$env:PYTHONPATH = (Resolve-Path python).Path
python/.release-venv/Scripts/python.exe -m unittest discover -s python/tests -v
cargo build --release --manifest-path src-tauri/Cargo.toml

# Không cần model hoặc mạng:
cargo test --manifest-path src-tauri/Cargo.toml audio::tests::ffmpeg_ -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml audio::tests::final_aac -- --ignored --nocapture

# Chỉ dùng model đã có, HF offline; output là thư mục QA riêng:
python/.release-venv/Scripts/python.exe scripts/audio-engine-tts-smoke.py --cache <model-cache> --output <qa-folder>
$env:JC_AUDIO_TTS_QA_DIR = '<qa-folder>'
cargo test --manifest-path src-tauri/Cargo.toml audio::tests::real_vietnamese_raw_fit_and_mixed_waveforms -- --ignored --nocapture

# Benchmark 100 câu, cũng offline:
python/.release-venv/Scripts/python.exe scripts/audio-engine-tts-smoke.py --cache <model-cache> --output <benchmark-folder> --count 100
$env:JC_AUDIO_BENCH_DIR = '<benchmark-folder>'
cargo test --manifest-path src-tauri/Cargo.toml audio::tests::benchmark_100_real_cached_voices_processing_and_mix -- --ignored --nocapture
```

Desktop smoke yêu cầu app dev với `JC_QA_DATA_DIR` trỏ vào dữ liệu QA (tất cả project tên `Native Preview QA`) và WebView2 debug port 9224. Script từ chối chạy trên project thật. `audio-preview-smoke.mjs` dùng project fixture `9bab3f9b-4811-4e19-938a-0cc4ec9d300d`, WAV đã xử lý và video 10 s. Chạy `node scripts/native-preview-smoke.mjs`, `node scripts/audio-preview-smoke.mjs`. Đóng/mở lại QA app giữa thay đổi frontend/HMR và kiểm thử. Không bật debug port trong release.

Commands mới: `audio_preview_open`, `audio_preview_control`, `audio_preview_status`, `audio_preview_close`. Control/close cần project ID và session UUID; IPC không gửi PCM.

## Xác nhận nghe và giới hạn còn lại

1. Nghe raw, natural, fit của **Ma Đế**, **Cảnh giới**, **Thánh Nhân**, **Đại Đế**, nghe hai câu sát nhau trong video và MP4 đã xuất; kiểm tra phụ âm đầu/cuối và độ tự nhiên.
2. Trên máy có BandIt thật: tách một video thoại/nhạc, thử 0/50/100%, đảm bảo nhạc còn và thoại giảm; so sánh preview/export, đổi slider không tạo job BandIt mới.
3. Chưa có NVIDIA trên máy kiểm thử nên chưa kiểm thử VieNeu CUDA/BandIt CUDA. Không đổi backend BandIt trong Task 7.
4. Transport hiện hỗ trợ Windows stereo 48 kHz; thiết bị không hỗ trợ trả lỗi. macOS/Linux chưa có transport audio mới.
5. Export 2 h tạo master PCM tạm khoảng 2,57 GiB trên đĩa, không nằm trong RAM; WAV RIFF >4 GiB chưa hỗ trợ. Có thể feed PCM trực tiếp vào encoder trong task sau.
6. Xử lý raw mới vẫn spawn FFmpeg một lần/voice. Cache xử lý/raw hạn chế lần chạy lại; tối ưu bằng resampler/WSOLA native hoặc batch processing cần benchmark riêng.
7. Chuẩn bị stem BandIt cho preview dùng cơ chế hiện có; chưa có cancel riêng cho bước chuẩn bị này. Cần mở rộng cancellation/resource scheduler nếu làm tiếp.

Task 8 nên giữ audio contract này, bổ sung kiểm thử BandIt thật CPU/CUDA, đo RAM cả process tree và tối ưu AI/inpainting riêng. Không thay timestamp subtitle để che vấn đề audio; không quay lại TTS theo nhóm trong task này.
