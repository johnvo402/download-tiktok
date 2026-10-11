# Xử lý AI theo video (T03)

Trong phần Dịch, bộ chọn **Video đang chỉnh sửa** chọn clip để tìm `videoContentId`. Hai clip dùng chung nội dung nguồn cùng dùng transcript/bản dịch/giọng; di chuyển hoặc trim clip không đổi timestamp nguồn hay xóa audio đã tạo. Bộ chọn giữ lựa chọn sau khi lưu cài đặt. Chuyển video chờ autosave transcript, cấu hình lồng tiếng và visual settings; lỗi lưu giữ lại bản sửa và lựa chọn hiện tại.

Ngôn ngữ dịch, prompt/context, giọng/profile, cấu hình audio, chế độ ASR/OCR và vùng OCR thuộc `VideoContent.settings`. Hộp cài đặt áp dụng cho video đang chỉnh sửa. Provider/model/runtime vẫn là cài đặt dùng chung của ứng dụng.

Bảng transcript có các thao tác đầy đủ cho video hiện tại, gồm thay transcript bằng ASR/OCR, trích subtitle, sửa lời, dịch và tạo/phát giọng theo dòng. Thanh phía trên có phạm vi video hiện tại/các video đã chọn/tất cả video. Các nút “phần thiếu” giữ transcript và lời dịch đã có; một nguồn xuất hiện nhiều lần trên timeline chỉ được xếp hàng một lần. Lỗi khởi chạy từng nguồn được hiển thị riêng và không chặn những nguồn còn lại. Mở “Tiến độ từng video” để xem trạng thái và hủy đúng job.

## API và dữ liệu

Các lệnh `get_transcript`, `save_transcript`, `extract_subtitle`, `get_tts_audio_path`, `get_video_asset_path`, `update_translation_languages`, `update_dubbing_settings`, `update_project_preferences`, `update_transcript_settings` nhận `videoContentId` tùy chọn cùng `projectId`. Không truyền ID thì dùng video chính qua `legacyBinding`, giữ tương thích các luồng cũ. Các lệnh trả Project trả tài liệu đầy đủ với root vẫn là video chính; projection của video đang chỉnh sửa chỉ dùng bên trong processor/editor.

Request ASR/OCR, dịch và TTS chứa `videoContentId`; backend chuẩn hóa ID trước khi kiểm tra job trùng. Nhật ký/retry payload giữ ID này. Sự kiện tiến độ bổ sung `videoContentId` và `contentRevision` lấy từ job đã đăng ký, không lấy từ lựa chọn giao diện. Giao diện giữ sự kiện riêng theo dự án/nội dung/loại tác vụ, nên chuyển lựa chọn không làm mất trạng thái đang chạy.

`ProjectStore.for_video` cung cấp source/settings của một nội dung mà không chuyển `legacyBinding`. Lưu scoped merge **chỉ** settings của nội dung đó vào tài liệu mới nhất dưới mutex dự án. Các thay đổi placement và nội dung khác được giữ lại; snapshot settings/source cũ của cùng nội dung bị từ chối. Lưu transcript tăng revision ngay cả khi metadata/số dòng không đổi; lưu cấu hình không đổi giá trị không tăng revision.

Transcript schema 1 thêm `videoContentId` tùy chọn để đọc dữ liệu cũ. Transcript có ID khác bị từ chối ngay cả khi checksum bằng nhau. Chỉ video chính được đọc sidecar cũ không có ID; lần lưu scoped đầu tiên ghi sidecar mới và giữ nguyên byte của file cũ. Root metadata được đồng bộ với metadata của video chính để preview/export một nguồn hiện có tiếp tục dùng đúng nội dung.

Đường dẫn theo nội dung:

```text
subtitles/contents/{videoContentId}/transcript.json
subtitles/contents/{videoContentId}/original.srt
audio/tts/contents/{videoContentId}/...
audio/extracted/contents/{videoContentId}/...
cache/contents/{videoContentId}/...
```

Các đường dẫn tương đối và thư mục sở hữu vẫn được kiểm tra. Đọc audio scoped từ chối audio của nội dung khác; riêng video chính tiếp tục đọc các file TTS cũ trực tiếp trong `audio/tts`. Cache mô hình và thumbnail/waveform theo asset không bị nhân bản.

## Kết quả nền và đồng thời

- ASR/OCR/subtitle capture ID + revision và checksum; trước khi thay transcript, kiểm tra lại dưới khóa nội dung. Người dùng sửa cùng nội dung trong lúc nhận dạng thì kết quả cũ bị bỏ. Thay đổi clip hoặc video khác vẫn cho phép lưu.
- Dịch giữ baseline theo segment (text, revision, trạng thái lời dịch), checksum, ngôn ngữ và prompt. Chỉ merge kết quả còn hợp lệ; giữ dòng đã sửa thủ công và cập nhật context của đúng video.
- TTS giữ baseline lời dịch, revision, timestamp nguồn, cửa sổ giọng, checksum, ngôn ngữ và cấu hình tạo giọng. Thay đổi các đầu vào này làm kết quả cũ bị từ chối. Sửa lời dịch chỉ đánh dấu audio của dòng đó `outdated`; audio của dòng/video khác được giữ.
- Khóa transcript và xung đột ASR/dịch/TTS dùng định danh nội dung. Hai video khác nhau có thể được xếp hàng riêng; CPU/GPU vẫn đi qua resource scheduler. Export/import/relink/xóa dự án tiếp tục dùng khóa toàn dự án. Retry dùng ID được lưu, hủy dùng job ID.
- Callback tải transcript/phát TTS của editor đã đóng không cập nhật editor mới. Snapshot project cũ và kết quả reload đến sau một lần sửa mới không ghi đè bản sửa hiện tại.

## Preview và các task tiếp theo

Video chính giữ preview native/audio hiện có. Video khác có preview nguồn WebView với phụ đề, seek theo thời gian nguồn, audition TTS theo dòng và chọn/di chuyển/đổi kích thước vùng OCR riêng. Đây là preview nguồn để xử lý T03; chưa phải playback nối tiếp/composite timeline. Timeline UI, dựng lớp, ánh xạ giọng/phụ đề theo clip và preview/export toàn timeline nằm ở T04–T09. Các thao tác visual/text-removal chỉ có ở preview video chính hiện tại và không được ghi nhầm sang video khác.

## Kiểm tra

Regression Rust kiểm tra A/B lưu–mở giữ ngôn ngữ, transcript, giọng và cache riêng; merge vào tài liệu mới nhất sau reorder/trim/chỉnh A; từ chối revision/ID cũ; giữ file legacy; chỉ làm cũ đúng audio. Test TTS dùng WAV thật với cùng tên cache ở hai nội dung, commit sau reorder, đọc audio đúng nguồn, từ chối cấu hình giọng đã đổi, và hủy/khóa riêng.

Regression TypeScript kiểm tra projection theo clip, merge B giữ nguyên root A, lọc sự kiện theo ID, scope deduplicate và loại nguồn đã xóa. Kiểm tra trình duyệt với bridge giả lập kiểm tra autosave trước đổi clip, A hoàn tất khi đang chọn B, lựa chọn giữ sau lưu, ngôn ngữ riêng, TTS đúng ID, scope tất cả, hủy riêng, preview/OCR của B và panel hẹp không tràn ngang. Không gọi dịch vụ trả phí hay tạo giọng AI thật trong các test này.
