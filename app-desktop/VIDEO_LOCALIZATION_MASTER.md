# AI Video Localization — Master Project Document

> **Phiên bản:** 1.0 · **Ngày:** 23/09/2026 · **Trạng thái:** Đặc tả khởi tạo, có thể cập nhật trong quá trình phát triển  
> **Tên làm việc:** Video Localization (có thể đổi)  
> **Mục tiêu:** Desktop app dịch phụ đề và lồng tiếng video tự động, có bước rà soát/chỉnh sửa, hỗ trợ làm mờ subtitle gốc và chèn logo; giao diện gọn, không phải trình dựng video kiểu CapCut.

## 0. Quy ước và quyết định đã chốt

- **Nền tảng ưu tiên:** Windows desktop trước; thiết kế đường dẫn/kiến trúc không khóa vào Windows để hỗ trợ macOS/Linux về sau.
- **Ứng dụng có hai cấp màn hình:** Start / Project Manager và Project Workspace.
- **Sidebar Workspace bên trái có đúng 3 mục:** `Kéo video` (placeholder, chưa xây), `Dịch` (chức năng chính), `Cài đặt` (cấu hình toàn cục). Không có timeline editor.
- **Layout trang Dịch:** sidebar trái → preview video lớn bên trái → panel bên phải chứa `Transcript`, `Media`, `Properties`. Bên dưới preview chỉ có play/pause, seek bar, thời gian, âm lượng.
- **Tạo dự án không hỏi vị trí lưu.** App tự tạo thư mục project trong app data mặc định. Tên project do người dùng nhập hoặc hệ thống đề xuất.
- **Xóa project là xóa cả thư mục project và toàn bộ dữ liệu trong đó**, sau hộp thoại xác nhận rõ ràng; không chỉ xóa khỏi danh sách. Xử lý an toàn, không xóa nhầm file nằm ngoài vùng do app quản lý.
- **Blur rectangle cố định về vị trí và kích thước**, có thể giới hạn khoảng thời gian xuất hiện nhưng **không có keyframe, object tracking**.
- **Video có soft-sub:** ưu tiên lấy trực tiếp; nếu không có, dùng ASR. Hard-sub đã dính vào hình không thể trích trực tiếp từ subtitle stream; OCR có thể phát triển sau, còn blur vùng chữ gốc là thủ công.
- **Bảng dịch có 2 cột nội dung gốc và nội dung dịch, chỉnh sửa được.** Dịch qua provider có thể cấu hình (Gemini, Google Translate, Custom CLI/Codex CLI; có thể mở rộng).
- **TTS mặc định đề xuất:** VieNeu-TTS local; có kiến trúc adapter để bổ sung API bên ngoài. Giọng được tổng hợp theo segment, không tạo một file dài ngay từ đầu.
- **Stack đề xuất ban đầu:** Tauri 2 + SolidJS + TypeScript + Vite + Python worker + FFmpeg. Các thư viện và model cụ thể được kiểm chứng khi triển khai, không xem con số kích thước/tốc độ là cam kết.

---

## 1. Phạm vi sản phẩm

### 1.1 Người dùng thực hiện được

1. Tạo, mở, đổi tên, nhân bản và xóa project trong màn hình Start.
2. Import video vào một project; xem video trong preview.
3. Kiểm tra subtitle track; trích soft-sub nếu có, hoặc tạo transcript qua ASR.
4. Xem, sửa transcript gốc và nội dung dịch song song; click segment để seek video.
5. Dịch toàn bộ hoặc từng segment bằng provider đã chọn trong Settings; giữ thứ tự, ID, timestamp.
6. Bật/tắt subtitle dịch khi preview/export; cấu hình kiểu chữ, vị trí.
7. Khoanh một hay nhiều hình chữ nhật làm mờ cố định; thay đổi vị trí, kích thước, cường độ, khoảng thời gian áp dụng.
8. Chèn logo/ảnh, kéo thả/resize trên preview, điều chỉnh vị trí, opacity và khoảng thời gian áp dụng.
9. Tạo và nghe thử giọng đọc cho từng segment, tạo hàng loạt; điều chỉnh trộn âm thanh gốc và giọng dịch.
10. Export video cuối và xuất riêng subtitle/audio nếu khả thi; theo dõi tiến độ, hủy tác vụ.
11. Tự động lưu các thay đổi của project; khôi phục project sau khi mở lại app.

### 1.2 Ngoài phạm vi MVP

Không xây timeline nhiều track, cắt-ghép nhiều clip, transition, motion graphics, keyframe, tracking blur, automatic lip-sync, tự động dịch chữ hard-sub bằng OCR, tách nhiều speaker và gán giọng tự động, cộng tác thời gian thực, upload project lên cloud. Mục `Kéo video` trong sidebar chỉ là placeholder; không thiết kế chức năng kéo/tải video tại thời điểm này.

### 1.3 Định nghĩa thành công MVP

Người dùng tạo project → nhập video → nhận transcript/soft-sub → dịch → chỉnh tay → bật subtitle → thêm blur/logo cố định → xuất file video; đóng/mở lại app không mất dữ liệu. TTS VieNeu có thể là milestone tiếp theo sau pipeline phụ đề ổn định (xem §15).

---

## 2. Kiến trúc thông tin và màn hình

```text
Application
├── Start / Project Manager
│   ├── Danh sách dự án gần đây (chỉ các project hợp lệ)
│   ├── Tạo dự án
│   ├── Mở dự án
│   ├── Đổi tên dự án
│   ├── Nhân bản dự án
│   └── Xóa dự án + thư mục
└── Project Workspace
    ├── Header: tên project, trạng thái lưu/xử lý, Export, quay lại Start
    ├── Left Sidebar
    │   ├── Kéo video (placeholder)
    │   ├── Dịch (mặc định)
    │   └── Cài đặt (global settings)
    └── Dịch
        ├── Left: video preview + overlay + playback controls
        └── Right: Transcript | Media | Properties
```

### 2.1 Start / Project Manager

- Header ứng dụng, nút `+ Dự án mới`.
- Danh sách project: tên, cập nhật gần nhất, thumbnail nếu có, thời lượng/nguồn video nếu đã import; tìm kiếm theo tên là tính năng tùy chọn.
- Click dự án → mở. Menu `⋯`: `Đổi tên`, `Nhân bản`, `Xóa dự án`.
- Không yêu cầu người dùng nhập hoặc chọn folder.
- Nếu danh sách trống: empty state hướng dẫn `Tạo dự án đầu tiên`.
- Project bị thiếu/corrupt: báo rõ, không tự xóa record hoặc thư mục khi chưa có hành động người dùng.

**Dialog tạo project:** một ô `Tên dự án` (gợi ý: `Dự án 01`) và hai nút `Hủy`, `Tạo dự án`. Kiểm tra tên rỗng/độ dài, ngăn tên chỉ có khoảng trắng; tên hiển thị khác với ID thư mục. Sau khi tạo thành công, mở thẳng `Dịch`.

**Dialog xóa project:** hiển thị tên project, số lượng/dung lượng dữ liệu nếu tính được, lời cảnh báo `Hành động này xóa vĩnh viễn thư mục dự án cùng video đã nhập, transcript, bản dịch, audio TTS và các file export lưu trong dự án.` Nút `Hủy` và `Xóa dự án` dạng nguy hiểm. Có thể yêu cầu nhập tên project ở phiên bản sau; không bắt buộc MVP.

### 2.2 Workspace / Dịch

```text
┌─────────────────────────────────────────────────────────────────────────┐
│ ← Projects   Tên dự án                 ● Đã lưu    [ Export ]          │
├──────────┬────────────────────────────────┬────────────────────────────┤
│ Kéo video│                                │ Transcript | Media | Props │
│ (soon)   │                                ├────────────────────────────┤
│          │                                │                            │
│ Dịch  ●  │         VIDEO PREVIEW          │  SOURCE     TRANSLATED     │
│          │     [logo]  [blur rect]         │  00:01.20 → 00:03.80       │
│ Cài đặt  │         subtitle text          │  Hello      Xin chào       │
│          │                                │                            │
│          ├────────────────────────────────┤  00:03.80 → 00:06.10       │
│          │ ▶ 00:32 / 04:21 ━━━●━━━━━━ 🔊 │  Today      Hôm nay        │
└──────────┴────────────────────────────────┴────────────────────────────┘
```

- Sidebar icon + nhãn (khoảng 72–88 px; có thể thu gọn). Mục `Kéo video` hiện nhãn `Sắp có` và không có hành động chưa được triển khai.
- Khung preview duy trì tỉ lệ gốc (fit/letterbox), không kéo méo video. Có thể kéo divider để tăng diện tích panel dịch; tối thiểu đảm bảo xem được video và sửa text.
- Panel phải mặc định `Transcript`, các tab còn lại là `Media` và `Properties`. Không lặp tab `Dịch` ở trong mục sidebar `Dịch`.
- Settings mở trong vùng làm việc chính với menu `General | Translation | Speech Recognition | Voice | Export/Advanced`; cấu hình toàn cục được giữ khi chuyển project. Khi quay lại Dịch, giữ playback/selection nếu có thể.

### 2.3 Transcript panel

- Toolbar: ngôn ngữ nguồn/đích, `Lấy phụ đề / Transcribe`, `Dịch tất cả`, `Tạo giọng`, `Tìm kiếm` (tùy chọn).
- Mỗi hàng: timestamp `start → end`, text gốc (editable), text dịch (editable), tình trạng dịch/TTS, nút phát thử giọng nếu đã tạo.
- Click timestamp hoặc hàng → video seek về `start`. Khi video chạy, segment hiện hành highlight. Không làm tự động cuộn gây giật khi người dùng đang gõ (tạm dừng auto-scroll khi focus editor).
- Sửa nội dung dịch đánh dấu segment `edited` và **đánh dấu audio TTS của segment đó là cần tạo lại**; không xóa file ngay khi sửa. Sửa text gốc không tự ghi đè bản dịch đã được người dùng sửa.
- Chức năng `Dịch lại` từng segment hoặc toàn bộ phải cảnh báo nếu thao tác ghi đè nội dung người dùng đã chỉnh.
- Khi panel đủ rộng: hai cột song song; khi hẹp: source trên / translated dưới. Tối ưu cho transcript dài bằng virtual list khi cần.

### 2.4 Media panel

- Video nguồn (tên, độ phân giải, thời lượng, codec nếu đọc được).
- Subtitle streams đã phát hiện, ngôn ngữ, định dạng, nút trích.
- Các tệp logo; thêm/thay/xóa.
- Reference audio / file audio đã tạo; không coi đây là multi-track editor.

### 2.5 Properties panel

Hiển thị theo đối tượng đang chọn trên preview hoặc theo loại đang chỉnh:

| Đối tượng | Các thuộc tính |
|---|---|
| Subtitle | bật/tắt chèn phụ đề dịch, font, size, màu, stroke/shadow, alignment, margin, vị trí |
| Blur | x, y, width, height, strength, start/end, xóa |
| Logo | ảnh nguồn, x, y, width/height giữ tỉ lệ, opacity, start/end, xóa |
| Segment/Voice | voice, tốc độ, volume, tạo lại, nghe thử, trạng thái stale |
| Video/audio | âm lượng gốc, âm lượng lồng tiếng, chế độ ducking (nếu hỗ trợ) |

---

## 3. Vòng đời project và lưu trữ mặc định

### 3.1 Nơi lưu

Dùng **thư mục dữ liệu người dùng theo chuẩn hệ điều hành**, do Tauri cung cấp (`appDataDir` hoặc đường dẫn app-specific do Rust path API trả về), không hard-code tài khoản hoặc ổ đĩa. Chỉ tên hiển thị nằm trong metadata; **folder name là UUID**, an toàn cho đổi tên và tránh path traversal.

Ví dụ minh họa Windows, không phải path hard-code:

```text
%APPDATA%/com.example.video-localization/
├── app-settings.json           # Không có plaintext API key
├── projects-index.json         # Danh sách ID + metadata
└── projects/
    └── <project-uuid>/
        ├── project.json
        ├── media/
        │   ├── source.mp4       # Bản sao video nhập, nếu copy mode
        │   └── logos/
        ├── subtitles/
        │   ├── original.srt
        │   └── translated.ass
        ├── audio/
        │   ├── extracted/
        │   └── tts/
        ├── cache/
        ├── exports/
        └── .jobs/
```

**Quy tắc mặc định import video:** copy tệp video vào `media/` của project để project tự chứa các asset thiết yếu; giữ tên gốc trong metadata. Người dùng được cảnh báo khi video lớn/thiếu dung lượng. Về sau có thể thêm chế độ `link to original` để tiết kiệm chỗ, nhưng phải ghi rõ file gốc bên ngoài project **không bị xóa** khi xóa project.

**Model AI** (Whisper/VieNeu) lưu ở `appData/models/` dùng chung, **không nằm trong project** và không xóa khi xóa một project. **API key** nằm trong secure OS credential store, không lưu trong project hoặc file JSON plaintext.

### 3.2 CRUD

**Create**
1. Validate tên hiển thị.
2. Sinh UUID; resolve `projectsRoot/UUID` và xác nhận folder chưa tồn tại.
3. Tạo folder tạm `projects/.creating-UUID`, `project.json` hợp lệ, thư mục con.
4. Đổi tên nguyên tử sang `projects/UUID` nếu filesystem hỗ trợ; cập nhật index bằng temp file + atomic replace.
5. Mở Workspace / Dịch. Nếu lỗi giữa chừng, dọn folder tạm chỉ khi do đúng tiến trình tạo.

**Read/Open**: dựa vào ID trong index; đọc/validate `project.json`, schema version; nếu thiếu file/media thì báo lỗi và cho phép liên kết lại/cứu dữ liệu, không âm thầm tạo project trống đè lên.

**Update/Rename**: chỉ sửa `name` trong `project.json` và index; không rename folder UUID. Autosave debounce, ghi vào file tạm rồi replace để giảm nguy cơ JSON hỏng sau crash.

**Duplicate**: sinh UUID mới, copy `project.json` + asset cần thiết vào folder mới, cập nhật ID/name/timestamps; loại bỏ `cache`, `.jobs` và tùy chọn copy `exports` (mặc định không copy exports để tiết kiệm chỗ); không copy secrets.

**Delete (xóa dữ liệu thật)**
1. Người dùng nhấn `Xóa dự án`, nhận dialog cảnh báo và xác nhận.
2. Dừng playback, hủy/chờ hoàn tất tất cả job của project, đóng file handles đang dùng; không xóa khi process vẫn ghi.
3. Rust/backend tra ID trong index rồi resolve canonical path. Chỉ cho phép xóa thư mục **con trực tiếp** của `projectsRoot`, basename khớp UUID đã đăng ký; từ chối path tùy ý và symlink/junction/reparse-point bất thường. Tuyệt đối không nhận đường dẫn cần xóa từ UI.
4. Đưa folder sang vùng `projects/.deleting-UUID` nằm trong cùng root (đổi tên khi có thể), cập nhật index có journal/trạng thái deletion, rồi xóa đệ quy **chỉ vùng đã xác thực**. Nếu dọn dẹp thất bại, báo lỗi + trạng thái cần dọn, không tuyên bố đã xóa sạch.
5. Không xóa file nguồn ngoài project, model shared, API key, setting toàn cục. File export nằm ngoài folder project do người dùng chủ động lưu ra ngoài không bị xóa.

**Lưu ý quan trọng:** một lệnh xóa thư mục đơn thuần có thể đi theo symlink hoặc nhầm root nếu code sai. Cần unit/integration test cho canonical path, symlink/junction, job đang chạy, folder thiếu/quyền bị từ chối và khôi phục sau crash. Khi app có tính năng import file ZIP/project từ bên ngoài, không tin bất kỳ đường dẫn nào trong file manifest.

### 3.3 Index và trạng thái

`projects-index.json` chỉ dùng cho danh sách, không là nguồn sự thật duy nhất. Có thể khôi phục index bằng cách scan các thư mục project hợp lệ. Các job dài có trạng thái `queued/running/succeeded/failed/canceled`, ghi kết quả vào project khi hoàn tất. Cache có thể xóa/tái tạo mà không ảnh hưởng transcript hoặc bản dịch.

---

## 4. Pipeline xử lý

```text
Import video
  ↓
ffprobe: duration, streams, subtitle tracks, dimensions, audio
  ↓
Có soft-sub được chọn? ─ yes → extract/parse → normalize segment
  │ no
  ↓
Extract audio → VAD/ASR local (faster-whisper) → normalize segment
  ↓
Transcript editor (có thể sửa)
  ↓
Translator adapter → bản dịch giữ ID/timing → editor (có thể sửa)
  ↓
Tùy chọn: TTS từng segment → preview/fit duration → mix/duck audio
  ↓
Subtitle ASS + blur/logo config → FFmpeg render → export
```

### 4.1 Import và nhận diện subtitle

- Dùng `ffprobe` đọc metadata, tất cả video/audio/subtitle stream; ưu tiên loại văn bản trích được (SRT/ASS/WebVTT/mov_text tùy container).
- Nếu có nhiều subtitle track, người dùng chọn track/language; không tự chọn track sai ngôn ngữ một cách im lặng.
- Nếu chỉ có subtitle bitmap (ví dụ PGS) hoặc hard-sub dính ảnh, không xem như subtitle text có thể trích trực tiếp. MVP dùng ASR từ audio và blur thủ công nếu cần che chữ gốc.
- Chuẩn hóa transcript: ID ổn định, start/end theo giây hoặc milliseconds có độ chính xác xác định; normalize Unicode, không tự làm mất dấu câu.
- Nếu video không có audio hoặc ASR thất bại, vẫn cho người dùng import SRT/VTT bên ngoài hoặc nhập tay.

### 4.2 Translation

Adapter contract khái niệm:

```ts
interface Translator {
  id: string;
  translate(input: {
    segments: Array<{ id: string; source: string; startMs: number; endMs: number }>;
    sourceLanguage: string;
    targetLanguage: string;
    context?: string;
  }): Promise<Array<{ id: string; translated: string }>>;
}
```

- Providers mục tiêu: Gemini API, Google Translation API, `Custom CLI` (Codex CLI là một cấu hình nếu phiên bản CLI/đăng nhập phù hợp), có thể thêm OpenAI-compatible local/API.
- Settings lưu provider active + các tham số không nhạy cảm. Backend giữ secrets, **không gửi API key vào log hoặc project**. Cảnh báo trước khi gửi transcript ra API bên ngoài.
- Dịch theo batch có ngữ cảnh, trả JSON có ID; validate schema, count, ID uniqueness, lỗi/thành công từng segment. Không map kết quả chỉ dựa theo vị trí mảng.
- Không thay đổi timestamp trong bước dịch. Cho phép dịch lại từng dòng; toàn bộ dịch lại phải xử lý xung đột với edited rows (skip/confirm overwrite).
- Google Translate không đảm bảo cùng kiểu văn phong/ngữ cảnh như LLM; hiển thị provider rõ ràng và cho phép sửa tay.
- CLI chạy subprocess với executable/arguments **tách biệt**, giới hạn timeout, kích thước input/output, không concat chuỗi shell thiếu kiểm soát; parse stdout theo contract/version được kiểm chứng.

### 4.3 Subtitle và overlay

- `subtitle.enabled=false`: không chèn subtitle dịch; **blur vẫn hoạt động độc lập** nếu bật.
- `subtitle.enabled=true`: hiển thị subtitle dịch theo segment và phong cách cấu hình.
- Với soft-sub: export không tự burn subtitle nguồn nếu không được yêu cầu. Với hard-sub: rectangle blur có thể che vùng chữ có sẵn; không hứa khôi phục được nền hoàn hảo.
- Preview dùng HTML video + overlay DOM/CSS; tọa độ overlay **chuẩn hóa theo vùng hình ảnh thực** (không tính thanh đen letterbox). Render cuối ánh xạ về kích thước pixel gốc; kiểm tra chính xác trên video ngang, dọc, khác tỉ lệ.
- Blur: `{x,y,width,height}` trong `[0,1]`, strength và optional `[startMs,endMs]`; hình chữ nhật không thay đổi tọa độ trong khoảng đó.
- Logo: tọa độ chuẩn hóa, opacity, kích thước, duration; kéo thả/resize bằng Pointer Events, không cần Konva. Giữ logo nằm trong biên preview, chống kéo vượt ngoài ảnh khi không chủ ý.
- Output text dùng ASS/libass khi phù hợp, FFmpeg xử lý blur/composite/logo và encode. Preview CSS là tham chiếu; **render mới là kết quả chính thức**, cần test sai khác font, vị trí, line wrap.

### 4.4 TTS và trộn âm thanh

- Provider mặc định dự kiến: VieNeu-TTS local. Kiểm tra model version, license áp dụng, hardware requirements và voice presets trước khi đóng gói/phân phối.
- Generate từng segment → file audio riêng (theo ID + fingerprint của text, voice, model/settings) → xác định duration → đặt đúng timestamp.
- Khi sửa bản dịch hoặc đổi voice, đánh dấu audio của segment `stale`; không phát audio cũ như thể vẫn phù hợp. Có `Generate segment`, `Generate all missing/stale`, `Preview`.
- Nếu audio dài hơn slot, cảnh báo; có thể fit tốc độ trong ngưỡng cấu hình, hoặc đề xuất rút gọn câu dịch. Không cắt mất lời không báo.
- Có gain gốc, gain dub, optional ducking; dự kiến MVP giảm âm thanh gốc toàn track trong đoạn dub, **không hứa tách riêng lời khỏi nhạc nền**. Tách stem/vocal và nhiều speaker để giai đoạn sau.
- Clone voice chỉ hỗ trợ khi người dùng có quyền và sự đồng ý cần thiết đối với giọng tham chiếu; chưa cần trong MVP.

### 4.5 Export

- Tùy chọn: output path (đây là đường dẫn **video export**, không phải đường dẫn project); default `project/exports/`, nút `Save As…` để chọn bên ngoài nếu muốn.
- Tối thiểu: MP4/H.264/AAC, FPS/độ phân giải giữ nguồn mặc định nếu khả thi, bật/tắt phụ đề dịch, blur, logo, dubbed audio. Tham số encode nâng cao mở sau.
- Build filtergraph theo dữ liệu project, không lấy DOM screenshot làm video xuất. Phải validate thời gian, đường dẫn, kiểu file, font path, audio layout và escape filter args.
- Render vào file tạm rồi rename khi thành công; cancel/fail dọn file tạm hoặc đánh dấu có thể resume tùy kiến trúc. Cho xem tiến độ, lỗi có khả năng hành động, đường dẫn output.
- Xem thử export một đoạn ngắn (optional) để kiểm tra vị trí blur/sub/logo và đồng bộ TTS.

---

## 5. Kiến trúc kỹ thuật

```text
Tauri 2 desktop shell (Rust)
├── Project service: CRUD, paths, metadata, autosave, deletion safety
├── Settings/secret service: config + OS credential store
├── Job manager: spawn/cancel/progress, concurrency limits
├── FFmpeg/ffprobe process management
└── Events/commands ↔ SolidJS UI
    ├── Start / Project Manager
    └── Workspace
        ├── Sidebar
        ├── Video + DOM overlay
        └── Transcript / Media / Properties / Settings

Python worker (subprocess/sidecar; JSON Lines hoặc giao thức IPC có version)
├── ASR: faster-whisper (WhisperX khi cần align/diarization sau)
├── Translation adapters (hoặc HTTP call ở Rust nếu ít dependency)
└── VieNeu-TTS / external TTS adapter

FFmpeg: demux, extract, audio mix, subtitles, blur, logo, export
```

### 5.1 Vì sao stack này

- UI SolidJS + Vite dễ tạo SPA nhỏ, state cập nhật theo segment; Tauri sử dụng webview hệ điều hành thay vì bundle Chromium như Electron. **Hiệu năng/kích thước thực tế cần benchmark**, đặc biệt model Python có thể chiếm nhiều RAM/disk hơn UI.
- Rust/Tauri là lớp duy nhất có quyền đụng filesystem và spawn process. Frontend không được truyền lệnh shell tùy ý hoặc path xóa không kiểm soát.
- Không bắt buộc chạy FastAPI/WebSocket local. Worker riêng giao tiếp qua JSON Lines stdout/stdin hoặc IPC; stderr dành cho diagnostic logs, stdout dành cho event theo schema. Nếu streaming audio/model yêu cầu protocol khác, ghi ADR trước khi đổi.
- FFmpeg và Python runtime/model được quản lý riêng; app installer ban đầu không cần bundle weights vài GB. Model manager tải theo lựa chọn sau khi người dùng đồng ý, có checksum/version và thông báo dung lượng.

### 5.2 Đề xuất cấu trúc source

```text
video-localization/
├── src/                         # SolidJS UI
│   ├── app/
│   │   ├── routes/
│   │   ├── stores/
│   │   └── services/             # typed Tauri invoke wrappers
│   ├── features/
│   │   ├── projects/
│   │   ├── workspace/
│   │   ├── transcript/
│   │   ├── media/
│   │   ├── overlays/
│   │   ├── dubbing/
│   │   ├── export/
│   │   └── settings/
│   └── shared/
├── src-tauri/
│   ├── src/
│   │   ├── commands/
│   │   ├── project_service/
│   │   ├── jobs/
│   │   ├── ffmpeg/
│   │   ├── security/
│   │   └── settings/
│   └── capabilities/
├── worker/
│   ├── main.py
│   ├── protocol.py
│   ├── asr/
│   ├── translation/
│   └── tts/
├── schemas/
│   ├── project.schema.json
│   ├── settings.schema.json
│   └── worker-events.schema.json
├── tests/
├── docs/
└── README.md
```

### 5.3 IPC / job contract

```json
{"v":1,"event":"progress","jobId":"job-01","projectId":"uuid","stage":"transcribe","done":37,"total":100}
{"v":1,"event":"completed","jobId":"job-01","artifact":"subtitles/original.srt"}
{"v":1,"event":"failed","jobId":"job-01","code":"MODEL_MISSING","message":"Download the selected ASR model."}
```

- Mỗi job có project ID, job ID, cancel token, status; không update vào project đã bị xóa hoặc project khác.
- Đảm bảo dùng path tương đối đã kiểm chứng cho artifact. Không tin path được worker trả về nếu nó nằm ngoài project.
- Giới hạn đồng thời (đặc biệt GPU) để tránh nhiều job chiếm VRAM; UI tiếp tục responsive khi xử lý.

---

## 6. Mô hình dữ liệu v1

**Nguồn sự thật:** `project.json` thuộc từng project, `app-settings.json` toàn cục. Dữ liệu lớn như WAV/video/subtitle không nhúng base64 vào JSON.

```ts
interface ProjectV1 {
  schemaVersion: 1;
  id: string;                       // UUID, khớp folder project
  name: string;                     // tên hiển thị
  createdAt: string;                // ISO 8601
  updatedAt: string;
  media: {
    video?: {
      relativePath: string;       // media/source.ext
      originalName: string;
      durationMs: number;
      width: number;
      height: number;
      hasAudio: boolean;
    };
    logos: Array<{ id: string; relativePath: string; originalName: string }>;
  };
  language: { source: string; target: string };
  segments: SegmentV1[];
  subtitle: {
    enabled: boolean;
    style: SubtitleStyleV1;
    importedTrack?: { streamIndex: number; language?: string; format?: string };
  };
  dubbing: {
    enabled: boolean;
    voiceId?: string;
    originalGain: number;
    dubGain: number;
    duckOriginal: boolean;
  };
  overlays: Array<BlurOverlayV1 | LogoOverlayV1>;
  export?: { format: 'mp4'; lastOutputRelativePath?: string };
}

interface SegmentV1 {
  id: string;
  startMs: number;
  endMs: number;
  speakerId?: string;
  source: string;
  translated: string;
  translationStatus: 'empty' | 'machine' | 'edited';
  tts?: {
    audioRelativePath: string;
    fingerprint: string;
    durationMs: number;
    status: 'ready' | 'stale' | 'failed';
  };
}

interface RectangleBase {
  id: string;
  x: number; y: number; width: number; height: number; // 0..1
  startMs: number;
  endMs: number | null;          // null = đến hết video
}
interface BlurOverlayV1 extends RectangleBase {
  type: 'blur'; enabled: boolean; strength: number;
}
interface LogoOverlayV1 extends RectangleBase {
  type: 'logo'; enabled: boolean; logoId: string; opacity: number;
}
```

**Validation:** `0 <= startMs < endMs <= durationMs` (khi đã biết duration), segment IDs duy nhất, rectangle có kích thước dương, `x + width <= 1`, `y + height <= 1`, opacity trong `[0,1]`, path file project phải relative và không chứa traversal. Cần bổ sung schema migrations khi đổi cấu trúc.

**Settings v1:** giao diện/theme, default source/target nếu muốn, `translation.provider`, `translation.model`, `asr.provider/model/device`, `tts.provider/model/voice/device`, `export.defaults`, đường dẫn model manager. API key tham chiếu bằng `credentialId` không chứa token thật. Lựa chọn provider toàn cục, còn project lưu kết quả/text/voice/output đúng thời điểm đã tạo; đổi Settings không tự viết lại bản dịch đã lưu.

---

## 7. Cài đặt (Global Settings)

| Mục | Nội dung | Quy tắc |
|---|---|---|
| General | giao diện, autosave, cache, dữ liệu app | Đường dẫn gốc project do app quản lý, không hỏi khi tạo project |
| Translation | Gemini, Google Translate, Custom CLI; model/timeout/batch/context; API key | Có `Test connection`; kiểm tra lỗi auth/quota/CLI không tìm thấy |
| Speech Recognition | faster-whisper model, device CPU/GPU, compute type, model manager | Hiển thị trạng thái cài model, dung lượng ước tính trước tải |
| Voice/TTS | VieNeu-TTS, voice preset, device/speed; API provider sau | `Test voice`; audio stale sau sửa text/voice |
| Export/Advanced | encoder, quality, hardware acceleration khi hỗ trợ, ffmpeg diagnostics | Mặc định an toàn; setting nâng cao tùy chọn |

**Bảo mật:** API key lưu qua OS credential store hoặc giải pháp secret storage có kiểm chứng; không lưu plaintext vào `project.json`, logs, crash reports. Không thực thi commands tùy ý từ project import. Công khai việc provider bên ngoài nhận transcript/audio được gửi lên khi bật chức năng tương ứng.

---

## 8. Trạng thái UI, lỗi và trải nghiệm

- Các trạng thái Import / Extract / ASR / Translate / TTS / Export đều có `idle`, `running`, `succeeded`, `failed`, `canceled`; hiển thị stage/progress thực khi có, nếu không có chỉ báo hoạt động không bịa `%`.
- Cho hủy tác vụ; khóa `Xóa dự án` đến khi job ngừng an toàn hoặc cho hủy job ngay trong flow xóa.
- Nếu không có video: preview hiển thị dropzone/nút `Chọn video` **trong mục Dịch**; mục `Kéo video` của sidebar vẫn chưa triển khai. Điều này không mâu thuẫn với việc video cần được import để dịch.
- Nếu video không có subtitle track: nút `Tạo transcript từ audio`.
- Không đủ GPU/model: đề nghị dùng CPU/model nhỏ hoặc tải model; không crash cả app.
- API fail/quota: bảo toàn nội dung đã dịch thành công; retry chỉ những segment thiếu.
- File video mất/corrupt: hiện thông báo, giữ project và text; không tự xóa.
- Unsaved/autosave fail: hiển thị `Chưa lưu được` và giữ draft trong bộ nhớ; báo trước khi đóng nếu có nguy cơ mất chỉnh sửa.

---

## 9. Tính riêng tư, bản quyền, quyền sử dụng

- Xử lý ASR/TTS local khi được chọn; với dịch/TTS API bên ngoài, thông báo nội dung nào sẽ được gửi tới provider.
- Người dùng chịu trách nhiệm có quyền sử dụng nội dung video, logo, subtitle và giọng tham chiếu; không tự động sao chép/clone giọng người khác mà không có quyền thích hợp.
- Không lưu key vào project để tránh rò rỉ khi backup/chia sẻ; không log transcript hay audio thô mặc định.
- Kiểm tra license, điều khoản và giới hạn phân phối model/provider trước khi ship bản thương mại.

---

## 10. Acceptance criteria / kiểm thử thiết yếu

### Project management

- [ ] Tạo project chỉ nhập tên, không hiện hộp chọn folder; app tạo folder UUID trong app data mặc định.
- [ ] Mở lại app vẫn có danh sách project; đổi tên không đổi folder ID.
- [ ] Nhân bản tạo project ID + folder mới, không trỏ vào cùng một folder dữ liệu có thể bị xóa chung.
- [ ] Xóa project A sau xác nhận xóa toàn bộ folder A và mục index tương ứng; project B, model shared và file nguồn bên ngoài nguyên vẹn.
- [ ] Hủy dialog xóa không thay đổi dữ liệu. Xóa thất bại vì file lock/quyền được báo đúng, có thể thử lại.
- [ ] Project path giả mạo, `../`, symlink/junction không được xóa ngoài projectsRoot.

### Dịch và preview

- [ ] Extract text subtitle stream hoặc ASR fallback đúng nhánh; hard-sub không bị nhận nhầm thành text stream.
- [ ] Edit hai cột được, autosave và reload giữ nguyên ID/time/text.
- [ ] Click segment seek đúng time; playback highlight đúng segment; text focus không bị auto-scroll giật.
- [ ] Toggle subtitle OFF không làm blur OFF; blur/logo map đúng vị trí qua nhiều tỉ lệ video.
- [ ] Sửa bản dịch sau khi tạo giọng đánh dấu TTS stale; export không dùng voice stale không cảnh báo.
- [ ] Export video có subtitle/blur/logo/audio theo setting, không dùng các overlay giao diện làm nguồn render.

### Performance/reliability

- [ ] Scroll transcript dài không lag đáng kể; import/export không chặn UI.
- [ ] Hủy job và xóa project khi đang render không để process ghi vào folder đã xóa.
- [ ] Crash khi autosave không làm hỏng bản project cuối cùng đã lưu thành công; có phương án recovery từ file tạm/journal.

---

## 11. Roadmap thực hiện

### Phase 0 — Foundation

Scaffold Tauri 2 + SolidJS + Vite; Rust commands typed; app-specific paths; settings service; logger; project schema + migrations; FFmpeg discovery/bundling strategy; cơ chế job cancellation.

### Phase 1 — Project Manager + UI shell

Start CRUD đầy đủ, auto app-data folder, xóa folder an toàn, sidebar 3 mục, Workspace Dịch layout, preview và input chọn video trong Dịch, autosave. **Đây là mốc code đầu tiên.**

### Phase 2 — Transcript + translation

ffprobe → soft-sub extraction → ASR fallback faster-whisper; bảng hai cột; Gemini/Google Translation adapters; sửa tay, retry, progress, subtitle preview/export đơn giản.

### Phase 3 — Overlay + export

Blur rectangle cố định, logo drag/resize, subtitle style, properties, FFmpeg filtergraph, output MP4, export status/cancel.

### Phase 4 — Voice

VieNeu-TTS local model manager; voice presets; TTS segment-by-segment; stale/regenerate, playback; audio mixing, timing validation; external TTS adapter tùy chọn.

### Phase 5 — Hardening

Test hiệu năng video dài, migration, crash recovery, GPU/CPU paths, installer, licensing, export preview, batch processing nếu cần.

---

## 12. Những quyết định còn mở (không cản khởi tạo)

1. **Tên sản phẩm + application identifier:** thay `com.example.video-localization` bằng ID chính thức **trước khi phát hành**, vì ID ảnh hưởng đường dẫn app-data.
2. **Chiến lược Python packaging:** sidecar executable hoặc managed Python env; thử nghiệm kích thước, antivirus/Windows compatibility và GPU packages trước khi chốt.
3. **Model ASR/TTS và license bản phân phối:** chọn phiên bản cụ thể sau benchmark tiếng Việt/Anh, CPU/GPU và điều khoản sử dụng.
4. **Copy vs link source:** MVP copy video vào project, có thể thêm lựa chọn link khi dung lượng lớn.
5. **Output export ngoài project:** mặc định trong `exports/`; file user lưu ở nơi khác không bị xóa khi xóa project.
6. **Retention/backup:** delete là vĩnh viễn theo yêu cầu; cân nhắc `Export Project / Backup` về sau để tránh mất dữ liệu ngoài ý muốn.

---

## 13. Nguyên tắc phát triển

- UI phục vụ workflow dịch và kiểm duyệt; **không thêm timeline editor** trừ khi có yêu cầu mới.
- Settings là **global**, video/subtitle/translation/blur/logo/dubbing là **project state**.
- Dữ liệu người dùng sửa tay có mức ưu tiên cao hơn output từ model; không overwrite âm thầm.
- Mọi tác vụ AI/render là job có thể theo dõi/hủy; project deletion kiểm soát vòng đời các job.
- Source video, preview UI và video export là ba lớp khác nhau; kiểm thử render đầu ra thay vì suy luận chỉ từ preview.
- Lưu đồ dữ liệu ổn định, schemas versioned, các provider là adapter có thể thay thế; chỉ tối ưu/bổ sung tính năng khi pipeline end-to-end đã hoạt động.

**Tài liệu này là baseline v1; mọi thay đổi phạm vi nên cập nhật phần Quyết định đã chốt, schema và acceptance criteria tương ứng.**
