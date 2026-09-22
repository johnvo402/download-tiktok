# TikTok Series Downloader (Windows)

Tool này nhận URL EP1 TikTok, lấy creator/profile của EP1, quét các bài đăng công khai của creator,
tìm các video có số tập, gom theo EP và ưu tiên candidate có dấu hiệu tiếng Việt.

> Lưu ý: TikTok không cung cấp một API công khai đảm bảo rằng "đây là bản audio Việt" cho mọi video.
> Vì vậy `vi-score` là heuristic. Nếu các phiên bản Việt/Trung/Thái nằm ở các tài khoản khác nhau,
> tool này cần được mở rộng thêm bước tìm candidate từ nhiều tài khoản.

## 1. Cài Python

Khuyến nghị Python 3.11 hoặc 3.12.

## 2. Cài FFmpeg

Cài FFmpeg và bảo đảm `ffmpeg.exe` có trong PATH.

## 3. Cài dependency

```powershell
py -m pip install -U pip
py -m pip install -r requirements.txt