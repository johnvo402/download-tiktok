# Preview nhiều nguồn (T08)

Trong phần Dịch, playhead điều khiển hình, tiếng gốc, TTS, nhạc, caption và các lớp ảnh/chữ của timeline. Chọn video để chỉnh transcript vẫn độc lập với video đang phát. Preview đọc nguồn trực tiếp; chỉnh ảnh/chữ và seek không xuất MP4 hay file trộn audio.

## Điều khiển

- **Phát / tạm dừng**, Space: phát từ playhead; ở cuối dự án, nút Phát bắt đầu lại từ đầu.
- **Tiến / lùi một khung hình**, ← / →: dừng phát và chuyển tới frame kế bên theo FPS đầu ra. FPS phân số như 30000/1001 dùng lưới thời gian nguyên để tránh sai số cộng dồn. Shift + mũi tên seek một giây; Home / End tới hai đầu timeline.
- Kéo thước/playhead để scrub. Hình được seek tại vị trí mới, tiếng ngắt trong lúc kéo. Nếu trước đó đang phát, thả chuột sẽ tiếp tục sau khi mọi nguồn cần thiết sẵn sàng.
- Trạng thái **Đang nạp hình và âm thanh tại playhead** giữ đồng hồ tại chỗ trong lúc chờ metadata, frame, audio hoặc manifest transcript/TTS. Một lớp bị chậm cũng dừng các tiếng khác; không tự bỏ qua đoạn lời.
- Lỗi file/giải mã hiện ngay ở preview, dừng phát và có **Thử lại preview** để tải lại player và manifest. Lỗi của nguồn chỉ đang prefetch không chặn cảnh hiện tại; sẽ được báo khi playhead tới nguồn đó.
- **Xem video đang chọn** trở về editor nguồn để xử lý OCR/xóa chữ/AI Remove riêng. Khi ứng dụng bị ẩn, timeline dừng để tránh audio tiếp tục với hình không cập nhật.

## Scene, clock và seek

`scene-plan.ts` nhận document snapshot, các manifest nguồn và thời điểm timeline. Kết quả gồm lớp theo order từ sau tới trước, clip instance, asset, timestamp nguồn, chữ/caption, vùng audio và biên kế tiếp. Preview dùng trực tiếp plan này, các hàm geometry/typography của T06 và offset/gain của T07. Draft hình/chữ/canvas chỉ được ghép vào document trong bộ nhớ khi xem thử. T09 sẽ dùng cùng contract snapshot và quy tắc thời gian cho export; export toàn timeline chưa nằm trong T08.

`PreviewClock` tính thời gian từ mốc tuyệt đối của đồng hồ monotonic, không cộng từng delta RAF đã làm tròn và không cắt delta dài về 100 ms. Mỗi nhịp chỉ đi tới biên gần nhất rồi đánh giá cảnh kế tiếp, nên không nhảy qua clip/đoạn giọng khi nhịp màn hình chậm. Chờ nguồn, pause và seek đặt lại mốc; khi tiếp tục, không chạy bù qua khoảng đã chờ.

Clock publish và thao tác seek là hai việc riêng. User seek tăng revision; player lập tức pause, đưa gain về 0, hủy callback frame cũ và thay seek đang chờ bằng timestamp mới. Chỉ seeked/frame đúng token và timestamp mới được hiện. Video đang dừng có thể đã present trước sự kiện seeked; khi không có callback frame mới, HAVE_CURRENT_DATA/seeked được xác minh lại sau 40 ms với cùng token. Promise phát cũ không được báo lỗi lên transport sau khi đã chuyển seek/nguồn hoặc dispose.

Một `PreviewPlayers` điều phối các player đang cần. Chỉ khi mọi player và manifest của cảnh sẵn sàng mới phát tất cả. Gain/fade/ducking cũng lấy timestamp này; video browser luôn muted vì mixer T07 sở hữu tiếng. Khi nguồn chậm hoặc drift cần hiệu chỉnh, tất cả dừng cùng nhau để seek lại. Khoảng trống vẫn tiến theo clock và nền canvas, audio riêng vẫn phát nếu nằm trong khoảng đó.

## Prefetch và tài nguyên

Prefetch nhìn trước 1.5 giây. Tất cả lớp đang dùng được giữ; tối đa 6 clip sắp tới và 8 vùng audio sắp tới được mở thêm. Video/audio được seek tới điểm bắt đầu đã trim và giữ pause/gain 0, ảnh được tải trước. Nguồn ngoài cửa sổ này bị tháo khỏi DOM; pending path resolution có cờ hủy, media được pause, gỡ src và load để giải phóng decoder/buffer, gain/source node bị disconnect. Context và compressor dùng chung được đóng khi rời preview.

Manifest không còn tải toàn bộ video trong dự án lúc mở preview: chỉ nguồn hiện tại, sắp tới và đang chọn. Cache giữ tối đa 50 mục, trừ các mục đang được consumer pin; mục không còn dùng trong cùng dự án cũng được phép đẩy ra. Kết quả request đã bị thay thế/evict không ghi lại cache. Không decode toàn WAV vào bộ nhớ.

## Native/GPU và compositor

Cảnh đúng một video nguyên khung, không crop/xoay/lật/opacity/PiP, cùng tỷ lệ nguồn/canvas và nền đen có thể dùng native surface. API `preview_open` nhận `timelineClipId` tùy chọn, kiểm tra clip thuộc dự án rồi mở đúng `videoContentId` bằng processing view T03. View raw tắt subtitle/overlay nguồn để không vẽ trùng; không ghi thay đổi vào dự án. Editor nguồn cũ tiếp tục dùng API mặc định.

Phiên timeline dùng `external_clock`: chỉ lấy mẫu timestamp nguồn từ clock timeline, không tự chạy clock native hay lấy clock audio của video legacy. Seek flush decoder có generation; status chỉ xác nhận frame đã present của generation hiện tại. Surface được render khi ẩn để chuẩn bị frame đầu rồi mới hiện; không chạy decoder browser của video đó song song. Đổi clip hoặc seek ẩn surface cũ; phiên mở đến muộn được đóng. Decode GPU lỗi tiếp tục dùng fallback CPU hiện có; native/session/HDR không phù hợp chuyển về browser và báo trạng thái. Dialog/menu tạm ẩn HWND để không che UI; pointer từ native surface vẫn chọn đúng clip.

Nếu có caption, ảnh/chữ, nhiều video hoặc transform/canvas không phù hợp surface native, preview dùng compositor browser/WebView. Các lớp giữ cùng clock/geometry/order; video chồng lớp và PiP không đi qua surface một nguồn. Codec trong chế độ này cần được WebView hỗ trợ; file không giải mã được hiện lỗi có thể thử lại. Xử lý pixel OCR/blur/AI Remove vẫn ở editor nguồn, như phạm vi T06.

## Kiểm tra

`npm test`: 85 test/19 file. Có test clock mô phỏng 90 phút, chờ nguồn/pause/seek/biên, player đồng bộ và prefetch im lặng, seek/play/frame callback cũ, dispose, FPS phân số và scene snapshot/order/clip lặp/giới hạn prefetch. `npm run build` thành công.

`cargo test --lib`: 223 qua, 24 test tùy chọn bỏ qua. Test mới kiểm tra A/B processing view raw không ghi lại editor, từ chối clip không đúng và external clock không tự chạy khi chờ.

Test tùy chọn `cargo test --lib timeline_hidden_gpu_sources_and_seek_generations -- --ignored --nocapture` chạy thành công trên Intel Iris Xe: hai MP4 màu khác nhau, đường dẫn Unicode, surface GPU ẩn, nhiều seek, hủy frame cũ và fallback software FFmpeg. Test này cần Windows, driver GPU và FFmpeg.

UI headless Chrome với MP4 có tiếng, WAV TTS và ảnh thật đã qua A/B nối tiếp, nạp trước im lặng, gap, repeated A, overlay và hai video chồng lớp; seek nhanh/chờ path resolution không chạy clock hay lẫn tiếng, frame stepping, decoder release và file lỗi/thử lại. Lượt đo hai video chồng lớp ở fixture 25 FPS có drift lớn nhất khoảng 31 ms trong mẫu kiểm tra. Đây là fixture ngắn; test clock 90 phút là mô phỏng, không phải chứng nhận phát media 90 phút. Hồi quy UI T05/T06/T07 qua. Native protocol dùng bridge mô phỏng kiểm tra phiên A/B, seek/open cũ, timestamp external, occlusion và fallback khi có caption; bài GPU riêng kiểm tra decoder/surface thật. Kiểm tra tổng thể và nhiều codec/FPS dài tiếp tục ở T11.
