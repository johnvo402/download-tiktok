from __future__ import annotations

import logging
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Any, Callable
from urllib.parse import urlparse

from .config import BROWSER_LOCALE
from .cookies import load_netscape_cookies
from .urls import get_video_id, is_video_url, normalize_url

log = logging.getLogger(__name__)


def _development_cookie_file() -> Path | None:
    """Use a repository sidecar cookies.txt only during development."""
    repository_root = Path(__file__).resolve().parents[3]
    if not (repository_root / "python" / "worker" / "__main__.py").is_file():
        return None
    candidates = [repository_root / "cookies.txt"]
    workspace_root = repository_root.parent
    if (workspace_root / "tiktok_downloader" / "app.py").is_file():
        candidates.append(workspace_root / "cookies.txt")
    for candidate in candidates:
        try:
            if candidate.is_file() and not candidate.is_symlink():
                return candidate
        except OSError:
            continue
    return None


def _safe_downloader_error(value: object) -> str:
    message = str(value).strip()
    message = re.sub(r"https?://\S+", "[video URL]", message)
    message = re.sub(r"[\r\n\t]+", " ", message)
    return message[:360]


def _validated_url(value: str) -> str:
    url = normalize_url(value)
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    if parsed.scheme not in {"https", "http"} or not (
        host == "tiktok.com" or host.endswith(".tiktok.com")
    ):
        raise ValueError("Hãy nhập liên kết video/series thuộc tiktok.com.")
    if not parsed.path or parsed.path == "/":
        raise ValueError("Liên kết TikTok không có đường dẫn video hợp lệ.")
    return url


def download_drama(request: dict[str, Any], emit: Callable[[dict[str, Any]], None]) -> None:
    """Read the drama page and download each discovered episode, emitting JSONL progress."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError as error:
        raise RuntimeError(
            "Thiếu Playwright trong Python worker. Cài dependencies bằng "
            "`python -m pip install -e python` rồi khởi động lại ứng dụng."
        ) from error
    from .browser import TikTokNavigator
    from .metadata import create_series_directory, download_poster, write_metadata

    url = _validated_url(str(request.get("url", "")))
    output_root = Path(str(request.get("outputDirectory", ""))).resolve()
    if not output_root.is_dir():
        raise ValueError("Thư mục lưu TikTok không tồn tại hoặc không thể truy cập.")

    cookie_file_value = request.get("cookieFile")
    cookie_file = (
        Path(str(cookie_file_value)).resolve()
        if cookie_file_value
        else _development_cookie_file()
    )
    cookies: list[dict[str, Any]] = []
    if cookie_file:
        if not cookie_file.is_file():
            raise FileNotFoundError("Không tìm thấy file cookies đã chọn.")
        cookies = load_netscape_cookies(cookie_file)

    cookie_note = " Có cookies.txt cục bộ để xác thực." if cookie_file and not cookie_file_value else ""
    emit({"type": "progress", "job": "tiktok_download", "progress": 0.02, "stage": "browser", "message": f"Đang mở TikTok để đọc thông tin series…{cookie_note}"})
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True, channel="chrome")
        try:
            context = browser.new_context(locale=BROWSER_LOCALE)
            if cookies:
                context.add_cookies(cookies)
            page = context.new_page()
            navigator = TikTokNavigator(page)
            navigator.open(url)
            info = navigator.get_drama_info()
            if not info.title or info.title == "untitled":
                raise RuntimeError("TikTok không trả thông tin series. Hãy kiểm tra link, đăng nhập/cookies hoặc thử lại sau.")
            emit({"type": "progress", "job": "tiktok_download", "progress": 0.05, "stage": "discover", "message": "Đang mở chế độ Phim ngắn và chờ TikTok tải danh sách tập…"})
            navigator.try_open_episode_list()
            episode_numbers = navigator.discover_episode_numbers()
            if not episode_numbers:
                raise RuntimeError(
                    "TikTok không hiển thị danh sách tập sau khi chờ tối đa 60 giây. "
                    "Có thể mạng đang chậm, trang chưa tải xong hoặc link không phải trang series. Hãy kiểm tra kết nối/link rồi thử lại."
                )

            series_dir = create_series_directory(output_root, info.title)
            poster_file = download_poster(context.request, info.poster_url, series_dir)

            write_metadata(
                info=info,
                series_dir=series_dir,
                source_url=url,
                episode_numbers=episode_numbers,
                poster_file=poster_file,
            )
            details = {
                "title": info.title,
                "tags": info.tags,
                "description": info.description,
                "playCount": info.play_count,
                "posterFile": poster_file,
                "totalEpisodes": len(episode_numbers),
                "episodeNumbers": episode_numbers,
                "outputDirectory": str(series_dir),
            }
            emit({"type": "info", "job": "tiktok_download", "details": details})
            emit({"type": "progress", "job": "tiktok_download", "progress": 0.12, "stage": "discover", "message": f"Đã tìm thấy {len(episode_numbers)} tập · {info.title}"})

            previous_video_id: str | None = None
            episode_count = len(episode_numbers)
            for index, episode in enumerate(episode_numbers):
                if not navigator.select_episode(episode):
                    raise RuntimeError(f"Không thể chọn tập {episode} trên TikTok.")
                video_url = navigator.get_current_video_url()
                if not video_url:
                    raise RuntimeError(f"Không lấy được liên kết video của tập {episode}.")
                current_video_id = get_video_id(video_url)
                if previous_video_id and current_video_id == previous_video_id:
                    raise RuntimeError(f"Video ID không đổi sau khi chọn tập {episode}; đã dừng để tránh tải nhầm tập.")

                base = 0.12 + 0.86 * index / episode_count
                span = 0.86 / episode_count
                emit({"type": "progress", "job": "tiktok_download", "progress": base, "stage": "download", "episode": episode, "episodeIndex": index + 1, "episodeCount": episode_count, "message": f"Đang tải tập {episode:04d} ({index + 1}/{episode_count})…"})
                _download_episode(
                    video_url,
                    episode,
                    cookie_file,
                    str(request["ffmpegPath"]) if request.get("ffmpegPath") else None,
                    series_dir,
                    lambda fraction: emit({
                        "type": "progress",
                        "job": "tiktok_download",
                        "progress": min(0.98, base + span * fraction),
                        "stage": "download",
                        "episode": episode,
                        "episodeIndex": index + 1,
                        "episodeCount": episode_count,
                        "message": f"Đang tải tập {episode:04d} · {round(fraction * 100)}%",
                    }),
                )
                previous_video_id = current_video_id

            files = sorted(
                (path for path in series_dir.glob("EP*.*") if path.is_file()),
                key=lambda path: path.name.casefold(),
            )
        finally:
            browser.close()

    emit({
        "type": "completed",
        "job": "tiktok_download",
        "progress": 1.0,
        "message": f"Đã tải xong {len(files)} tập.",
        "details": details,
        "files": [path.name for path in files],
        "outputDirectory": str(series_dir),
    })


def _download_episode(
    url: str,
    episode: int,
    cookie_file: Path | None,
    ffmpeg_path: str | None,
    output_dir: Path,
    on_progress: Callable[[float], None],
) -> None:
    try:
        import yt_dlp
    except ImportError as error:
        raise RuntimeError("Chức năng tải video chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.") from error

    # The standalone downloader uses yt-dlp's CLI. Keep the desktop worker on
    # that same code path: some extractor/postprocessor failures raised through
    # YoutubeDL(options) are otherwise surfaced as an unhelpful bare AssertionError.
    # A packaged worker dispatches this private mode from worker_launcher.py.
    cli_args = [
        "--impersonate", "chrome",
        "--no-playlist",
        "--no-overwrites",
        "--retries", "3",
        "--fragment-retries", "3",
        "--extractor-retries", "2",
        "--socket-timeout", "30",
        "--newline",
        "--progress-template",
        "download:__AVL_PROGRESS__%(progress.downloaded_bytes)s|%(progress.total_bytes_estimate)s|%(progress.total_bytes)s",
        "-o", str(output_dir / f"EP{episode:04d}.%(ext)s"),
    ]
    if cookie_file:
        cli_args.extend(["--cookies", str(cookie_file)])

    # Tauri's development resolver may pass the bare command name "ffmpeg".
    # yt-dlp interprets --ffmpeg-location as a filesystem path, so resolve PATH
    # commands first and omit the option when the executable is unavailable.
    resolved_ffmpeg = None
    if ffmpeg_path:
        ffmpeg_candidate = Path(ffmpeg_path).expanduser()
        if ffmpeg_candidate.exists():
            resolved_ffmpeg = str(ffmpeg_candidate.resolve())
        else:
            resolved_ffmpeg = shutil.which(ffmpeg_path)
    if resolved_ffmpeg:
        cli_args.extend(["--ffmpeg-location", resolved_ffmpeg])

    cli_args.append(url)
    if getattr(sys, "frozen", False):
        command = [sys.executable, "--yt-dlp-cli", *cli_args]
    else:
        command = [sys.executable, "-m", "yt_dlp", *cli_args]

    creation_flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    startup_info = None
    if os.name == "nt":
        startup_info = subprocess.STARTUPINFO()
        startup_info.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup_info.wShowWindow = subprocess.SW_HIDE

    last_error: str | None = None
    return_code: int | None = None
    child_environment = os.environ.copy()
    child_environment["PYTHONUTF8"] = "1"
    child_environment["PYTHONIOENCODING"] = "utf-8:backslashreplace"
    # CLI and standalone downloader both consume the Netscape export, not the
    # browser's transient in-memory cookies.
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            env=child_environment,
            creationflags=creation_flags,
            startupinfo=startup_info,
        )
        if process.stdout is None:
            process.kill()
            process.wait()
            raise RuntimeError("Không mở được output của yt-dlp.")
        for line in process.stdout:
            progress_match = re.search(
                r"__AVL_PROGRESS__([^|]*)\|([^|]*)\|([^\s|]*)", line
            )
            if progress_match:
                downloaded_raw, estimate_raw, total_raw = progress_match.groups()
                try:
                    downloaded = float(downloaded_raw)
                    total = None
                    for candidate in (total_raw, estimate_raw):
                        try:
                            parsed = float(candidate)
                        except (TypeError, ValueError):
                            continue
                        if parsed > 0:
                            total = parsed
                            break
                    if total is not None:
                        on_progress(max(0.0, min(1.0, downloaded / total)))
                except (TypeError, ValueError):
                    pass
                continue
            if "[error]" in line.lower() or "error:" in line.lower():
                last_error = _safe_downloader_error(line)
        return_code = process.wait()
    except RuntimeError:
        raise
    except OSError as error:
        detail = _safe_downloader_error(error)
        auth_hint = (
            ""
            if cookie_file
            else " Nếu series yêu cầu đăng nhập, hãy chọn file cookies Netscape còn hiệu lực."
        )
        raise RuntimeError(f"Không khởi chạy được yt-dlp cho tập {episode}: {detail}{auth_hint}") from None

    downloaded_files = [
        path
        for path in output_dir.glob(f"EP{episode:04d}.*")
        if path.is_file() and path.suffix.lower() not in {".part", ".ytdl"} and path.stat().st_size > 0
    ]
    if return_code != 0 or not downloaded_files:
        detail = last_error or (
            f"yt-dlp CLI kết thúc với mã {return_code}."
            if return_code
            else "yt-dlp không tạo được file video hợp lệ."
        )
        auth_hint = (
            ""
            if cookie_file
            else " Nếu series yêu cầu đăng nhập, hãy chọn file cookies Netscape còn hiệu lực."
        )
        raise RuntimeError(f"Không tải được tập {episode}: {detail}{auth_hint}")
