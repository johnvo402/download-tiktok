# Công cụ dựng bổ sung (T10)

Hoàn tất ngày 2026-10-12. Trong Properties, “Fade hình & keyframe” điều khiển
video, ảnh/logo, chữ và phụ đề. “Chuyển cảnh từ clip trước” dành cho video/ảnh.
Toolbar có nút Hiệu ứng/keyframe, phím K mở phần này. Chuột phải hoặc
Shift+F10 trên clip mở menu cắt, copy/paste, nhân bản, xóa/ripple và hiệu ứng;
menu hỗ trợ mũi tên, Home/End, Escape và trả focus về clip.

## Keyframe và fade

`Clip.effects` là trường tùy chọn của timeline schema 1:

```json
{
  "fadeInUs": 500000,
  "fadeOutUs": 500000,
  "keyframes": [
    { "sourceUs": 1000000, "x": 0.25, "y": 0.5, "scaleX": 0.5, "scaleY": 0.5, "opacity": 0.5 },
    { "sourceUs": 3000000, "x": 0.75, "y": 0.5, "scaleX": 1, "scaleY": 1, "opacity": 1 }
  ]
}
```

Keyframe dùng timestamp nguyên microsecond **trong nguồn**, cùng ánh xạ in/out
và tốc độ phân số của clip. Giao diện nhập vị trí tính bằng giây từ đầu clip;
chuyển sang nguồn khi thêm điểm. Di chuyển clip không đổi keyframe. Trim/split
giữ các điểm nguồn để các phần tiếp tục lấy mẫu đúng; fixed retime chỉ đổi tốc
độ chạy qua chúng. Các điểm ngoài trim được giữ và đánh dấu, không tự xóa.

Nội suy tuyến tính x/y/scaleX/scaleY/opacity giữa hai điểm; trước/sau vùng có
keyframe giữ giá trị đầu/cuối. Mỗi clip tối đa 64 điểm, thời gian tăng nghiêm
ngặt; scale `(0,100]`, opacity `[0,1]`. Không có điểm thì dùng transform cơ bản.
Khi có điểm, năm thuộc tính trên do keyframe điều khiển; crop/fit/rotation/flip
vẫn dùng transform cơ bản. Xóa toàn bộ keyframe để trở lại giá trị cơ bản.

Fade hình là ramp tuyến tính theo đầu/cuối **clip hiện tại**, nhân với opacity;
thời lượng mỗi ramp được giới hạn bởi thời lượng clip. Trim/split/rate có thể
đổi mép và thời lượng fade; keyframe vẫn bám nguồn. Fade tiếng và tốc độ cố định
đã có ở phần Âm thanh/thời gian clip từ T07, không bị thay bằng fade hình.

Nhập fade, thêm/cập nhật/xóa điểm chỉ tạo bản nháp preview. “Áp dụng hiệu ứng”
lưu toàn bộ thành một transaction/một bước undo; Hủy hoặc đổi lựa chọn thu hồi
bản nháp. Track khóa không cho thay đổi. Export lấy dữ liệu đã lưu, không ghi
bản nháp. Keyframe không sửa timestamp transcript, nội dung dịch hay WAV TTS.

## Chuyển cảnh

`Clip.transitionIn` trên B tham chiếu clip A liền trước:

```json
{ "kind": "dissolve", "fromClipId": "uuid-of-A", "durationUs": 1000000 }
```

Có `dissolve` (hòa tan) và `wipeLeft` (quét từ trái sang phải). Hai clip cần cùng
track hình, thuộc video/ảnh và liền nhau trước khi áp dụng. Thời lượng phải
dương, ngắn hơn **cả hai** clip. Hiệu ứng tiêu thụ phần cuối A/đầu B trong
in/out đã trim; không vượt nguồn, kéo dài clip hay lặp frame để bù.

Ví dụ A `[0,4)` và B `[4,8)`, thêm 1 giây chuyển cảnh cho B thành `[3,7)`.
A `[0,4)` giữ nguyên; `[3,4)` là vùng chồng. Các clip sau B cùng track và các
lớp liên kết của chúng cũng dịch lên 1 giây. Nhạc, ảnh/chữ hoặc track độc lập
giữ vị trí; thời lượng toàn dự án vẫn lấy clip kết thúc muộn nhất.

Đổi thời lượng dùng chênh lệch với hiệu ứng cũ; gỡ trả lại vị trí chuỗi theo
thời lượng đã chồng. Nhóm kéo cả A theo B hoặc track phụ thuộc đang khóa làm
thao tác bị từ chối. Gỡ chuyển cảnh trước khi đổi điểm cắt/tốc độ hoặc di
chuyển riêng một phía làm hỏng quan hệ; không âm thầm sửa nguồn để giữ hiệu ứng.

Trong vùng chồng, B ở trên A theo thứ tự bắt đầu trong cùng track. Hòa tan
giữ A và ramp opacity B `0→1`; wipe giữ opacity và mở mask của B. Tiếng gốc,
giọng tự động và các lớp tiếng liên kết crossfade A `1→0`, B `0→1`. Lớp
phụ đề/ảnh/chữ liên kết cũng fade theo video chủ. Gain này nhân với volume,
fade tiếng và ducking sẵn có.

Validation TS/Rust chỉ cho phép chồng cùng track khi có chuyển cảnh hợp lệ,
hoặc hai lớp liên kết của A/B giao nhau **bên trong** vùng chuyển cảnh. Chồng
vô ý, ba clip chồng hoặc tham chiếu sai bị chặn trước khi lưu. Copy/paste một
cặp đổi ID tham chiếu; copy riêng B bỏ tham chiếu chuyển cảnh. Xóa A bỏ tham
chiếu của B; undo khôi phục toàn layout và giữ kết quả AI nguồn mới.

## Preview/export

`effects.ts` và `timeline_effects.rs` chia sẻ fixture
`test-fixtures/timeline-effects.json` cho thời gian/giá trị nội suy. Scene plan
đánh giá transform/reveal tại clock timeline; compositor trình duyệt áp dụng
CSS transform, opacity và clip-path. Cảnh có hiệu ứng dùng compositor thay vì
surface native một video không hỗ trợ ghép lớp.

FFmpeg dùng biểu thức số đã validation cho scale/position/alpha theo cùng
clock nguồn. Alpha/mask áp dụng trước affine transform; scale động được pad
vào footprint cố định rồi xoay/ghép. Xuất vùng giữ timestamp tuyệt đối khi
đánh giá hiệu ứng, sau đó đổi PTS đầu ra về 0, nên không khởi động lại fade
hoặc chuyển cảnh tại mép vùng. Master audio dùng ramp chuyển cảnh tương tự.

Các hiệu ứng mới giữ contract snapshot/tiến độ/hủy/verify của T09. Project cũ
không có effects/transitionIn vẫn đọc được và render đường nhanh cũ. Không có
speed ramp, easing hay keyframe rotation/crop trong T10; tốc độ phi tuyến cần
thiết kế ánh xạ transcript/TTS riêng.

## Kiểm tra

- Build frontend và 93 test frontend qua. Kiểm tra shared clock, trim/split/
  retime, linked captions/voices, thêm/đổi/gỡ chuyển cảnh, khóa, nhóm, source
  bounds, chồng ba clip, copy remap và giữ dữ liệu nguồn.
- Suite Rust: 230 test qua, 27 test tùy chọn bỏ qua. Test mới kiểm tra cùng
  fixture, lưu/mở/undo/redo, tương thích clip cũ và lỗi validation không ghi
  đè document hợp lệ.
- `cargo test --lib timeline_export -- --ignored --nocapture`: ba test export
  media thật qua, gồm hồi quy T09 và test T10. Video đỏ/xanh, tiếng 440/880 Hz,
  lấy mẫu pixel/tần số để đối chiếu dissolve/wipe/crossfade, motion x/scale/
  opacity, fade và xuất vùng giữa hiệu ứng.
- UI Playwright với media thật và mock command/event: thêm hai keyframe,
  preview bản nháp, payload chỉ chứa trường cho phép, lưu/undo/redo, fade tại
  mép, phụ đề theo B khi dịch chuỗi, dissolve/wipe/gỡ, menu chuột phải và
  Shift+F10/mũi tên/Escape/focus, track khóa và inspector hẹp.

Đo hiệu năng dự án dài và kiểm thử phát hành tổng thể tiếp tục ở T11.
