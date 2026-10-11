# Thao tác dựng và lịch sử (T05)

## Chọn và dựng

Click chọn clip; Ctrl/Command hoặc Shift + click bật/tắt một clip trong lựa chọn. Ctrl/Command+A chọn các clip trong dự án. Các thao tác di chuyển/xóa/copy mở rộng lựa chọn tới các clip cùng nhóm và quan hệ video–lớp phụ thuộc.

Kéo thân clip để đổi thời điểm/track. Các clip được chọn trên track của clip đang kéo cùng chuyển sang track đích; các lớp trên track khác giữ track và dịch cùng khoảng thời gian. Kéo hai mép clip đang chọn để trim. Ghost chỉ minh họa trước khi thả, không sửa dữ liệu đã lưu. Esc/pointer cancel hủy gesture. Việc thay đổi metadata do AI trong lúc kéo không làm mất pointer; nếu bố cục clip đã đổi thì từ chối gesture cũ.

Hít mép dùng khoảng 8 pixel tới mép các clip không thuộc lựa chọn, điểm 0 và playhead. N bật/tắt hít; Alt tạm bỏ hít khi kéo. Nút nhập thời điểm và track đích, Trim đầu/Trim cuối cho phép thao tác bằng bàn phím. Thời gian nhập là giây trên dự án.

Không nhận clip chồng lấn cùng track; lớp chồng nhau đặt lên track riêng. Không trim vượt nguồn hoặc tạo đoạn âm/rỗng. Source window không bị viết lại khi chỉ di chuyển/đổi thứ tự. Cắt/trim ở tốc độ phân số căn theo quantum microsecond của rational để giữ ánh xạ thời gian ổn định.

## Clip phụ thuộc và nhóm

Nhóm giữ các clip cùng di chuyển, copy, xóa và chia nhóm bên trái/bên phải khi cut. Liên kết lớp với video nhận đúng một video cha; subtitles/voice phải dùng cùng video content ID. Lớp được liên kết ban đầu phải nằm trong khoảng video.

Trim video đồng thời trim subtitle/voice và giao khoảng hiển thị của các lớp phụ thuộc. Không âm thầm xóa cả một lớp vì trim: nếu giao rỗng, báo lỗi để người dùng tách liên kết trước. Trim riêng ảnh/chữ/audio phụ thuộc chỉnh khoảng riêng của lớp trong video; trim subtitle/voice điều khiển cạnh video cha. Split đổi liên kết của các phần bên phải sang instance video mới. Bản dịch/transcript/TTS nguồn và file audio gốc vẫn được giữ.

Đổi thứ tự clip hoán đổi với clip kề cùng track, giữ khoảng trống giữa hai clip và dịch các lớp liên kết. Nhóm chứa cả hai phía phải được tách nếu muốn hoán đổi riêng. Đổi thứ tự track hoán đổi order với track kề; vẫn áp dụng khóa.

Copy/duplicate/paste tạo ID clip/nhóm mới, giữ asset/content ID và remap liên kết bên trong bản sao. Nhân bản đưa cụm vào cuối dự án; paste dùng playhead. Nếu track cũ bận hoặc khóa, paste tạo track tương thích để không đè clip đang có. Clipboard nằm trong bộ nhớ, chỉ dán trong dự án sở hữu nguồn.

## Xóa và ripple

Xóa thường bỏ clip và lớp liên kết, giữ nguyên thời điểm clip khác. Kho nguồn, video content và transcript/TTS vẫn còn để dùng lại. Legacy binding chọn instance còn lại của cùng nguồn hoặc clip ID null; không tự tái tạo clip đã xóa.

Shift+Delete/ripple lấy hợp các khoảng thời gian của clip cần xóa, áp dụng lên toàn bộ track và dồn phần sau. Clip khác đi qua khoảng xóa được cắt thành phần còn lại, giữ source offset tương ứng. Liên kết được remap theo phần video cha còn lại. Nếu ảnh hưởng track khóa hoặc không thể giữ quan hệ nguồn/thời gian hợp lệ, từ chối toàn bộ transaction.

## Lưu và undo/redo

Mỗi gesture chỉ gửi một lệnh khi thả. Lệnh thành công tự lưu rồi thêm một bước undo; thao tác lỗi/hủy không vào lịch sử. Ctrl/Command+Z undo, Ctrl/Command+Shift+Z hoặc Ctrl/Command+Y redo. Lịch sử giữ tối đa 50 bước trong phiên theo dự án; mở lại ứng dụng bắt đầu phiên lịch sử mới. Cắt S, copy C/paste V với Ctrl/Command, nhân bản D, nhóm G/tách nhóm Shift+G với Ctrl/Command. Delete xóa thường.

`apply_timeline_edit(projectId, expected, next)` gửi clips, cấu trúc/thứ tự tracks, legacy binding và (từ T06) khung hình timeline. Backend compare-and-swap layout trong khóa ghi trên project mới nhất, giữ assets, settings/content/revision của video và control hiện tại của track đã có. Backend từ chối layout stale, track khóa, loại track sai, nguồn vượt thời lượng và chồng lấn. Undo/redo dùng cùng transaction/validator và không ghi đè kết quả AI mới. Thay đổi timeline bên ngoài lịch sử phải được xử lý trước; không undo đè bố cục mới.

Trước transaction, chờ các draft transcript/voice/visual. Save task timeline tham gia hàng rào đóng/chuyển dự án, nhưng chỉ chờ các save task khác để tránh chờ chính nó. Thêm clip từ kho nguồn cũng dùng lịch sử này. Ảnh/audio chèn tại giờ dự án đang xem; khi xem riêng nguồn, thời gian nguồn được ánh xạ qua clip đang chọn (ví dụ nguồn B giây 2, clip bắt đầu giây 10 thì chèn tại giây 12). Video thêm nối sau các video đã có. Nhập/relink nguồn là thao tác kho nguồn T02; nội dung nguồn không thuộc lịch sử dựng clip. T06 bổ sung transform, kiểu chữ từng instance và khung hình vào lịch sử.

Transform nhiều lớp và hình ảnh caption đã có ở T06; audio TTS, native compositor và export tiếp tục ở T07–T09.
