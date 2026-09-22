# TikTok Short Drama Downloader

Tool Python dùng Playwright để mở một TikTok series trong Chrome, đọc thông tin phim, điều hướng tập bằng các button trong episode grid và dùng `yt-dlp` để tải video.

## Cấu trúc dự án

```text
.
├── main.py
├── requirements.txt
├── cookies.txt
├── downloads/
│   └── <Tên phim>/
│       ├── metadata.json
│       ├── poster.<ext>
│       ├── EP0001.<ext>
│       ├── EP0002.<ext>
│       └── ...
└── tiktok_downloader/
    ├── app.py
    ├── browser.py
    ├── cli.py
    ├── config.py
    ├── cookies.py
    ├── downloader.py
    ├── metadata.py
    └── urls.py
```

## Cài đặt

```powershell
py -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -U pip
python -m pip install -r requirements.txt
```

## Chạy

Tải toàn bộ series được phát hiện:

```powershell
python main.py "https://www.tiktok.com/@user/video/123456789"
```

Giới hạn số tập tải:

```powershell
python main.py "<url>" --episodes 20
```

Các option khác:

```powershell
python main.py "<url>" --cookies cookies.txt --output downloads
```

## Cách điều hướng tập

Tool không scroll để đổi tập nữa.

1. Mở panel series.
2. Tìm episode grid.
3. Xác định tập đang xem bằng element có class chứa `SpanEpisodeActiveIcon`.
4. Click trực tiếp button của tập cần tải.
5. Nếu tập cần tải không còn trong grid hiện tại, tool chuyển sang tab kế tiếp bằng **index của tab**.
6. Nội dung text của tab như `1-24`, `25-48`, ... không được dùng để quyết định điều hướng.

Tool cũng duyệt các tab theo index để tự phát hiện toàn bộ số tập đang có.

## Metadata

Mỗi series được lưu vào một folder riêng theo tiêu đề phim:

```text
downloads/Ma Đế Đại Thừa Ẩn Sức Trở Về/
```

`metadata.json` chứa:

- tiêu đề
- tags
- mô tả
- lượt xem poster/series hiển thị trên TikTok
- URL poster
- tên file poster đã tải
- source URL
- tổng số tập
- danh sách số tập

Tên folder được sanitize để chạy an toàn trên Windows.
