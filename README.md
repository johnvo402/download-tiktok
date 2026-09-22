# TikTok Short Drama Downloader

Tool Python dùng Playwright để mở một tập TikTok trong Chrome, di chuyển qua từng tập bằng thao tác scroll và dùng `yt-dlp` để tải video.

## Cấu trúc dự án

```text
.
├── main.py
├── requirements.txt
├── cookies.txt                 # không commit
├── downloads/                  # không commit
└── tiktok_downloader/
    ├── __init__.py
    ├── app.py                  # orchestration
    ├── browser.py              # thao tác TikTok bằng Playwright
    ├── cli.py                  # argparse / CLI
    ├── config.py               # cấu hình và constants
    ├── cookies.py              # đọc Netscape cookies
    ├── downloader.py           # gọi yt-dlp
    └── urls.py                 # xử lý URL / video id
```

## Yêu cầu

- Python 3.11+
- Google Chrome
- Cookie TikTok dạng Netscape lưu tại `cookies.txt`

## Cài đặt

```powershell
py -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -U pip
python -m pip install -r requirements.txt
```

## Chạy

```powershell
python main.py "https://www.tiktok.com/@user/video/123456789"
```

Tuỳ chọn:

```powershell
python main.py "<url>" --episodes 82 --cookies cookies.txt --output downloads
```

Tool cũng chấp nhận URL bị copy dưới dạng Markdown.

## Luồng hoạt động

1. Đọc cookie Netscape.
2. Mở Chrome qua Playwright.
3. Mở URL tập đầu tiên.
4. Nếu có nút `Xem tất cả ... tập`, tool thử mở danh sách tập.
5. Lấy URL video hiện tại.
6. Gọi `yt-dlp` để tải tập.
7. Scroll sang tập tiếp theo và lặp lại.

## Ghi chú

TikTok thay đổi UI và cơ chế chống bot thường xuyên, nên phần điều hướng bằng Playwright có thể cần điều chỉnh theo thời gian. Không commit `cookies.txt` vào repository.
