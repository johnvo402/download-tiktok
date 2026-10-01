from __future__ import annotations

from pathlib import Path
from time import monotonic
from typing import Any, Callable
from urllib.parse import urlparse

from .config import BROWSER_LOCALE
from .cookies import load_netscape_cookies
from .direct_media import (
    BrowserMediaObserver,
    VideoCandidate,
    browser_video_element_candidates,
    download_video_candidate,
    embedded_video_candidates,
    rank_candidates,
)
from .urls import get_video_id, normalize_url


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
    use_browser_cookies = bool(request.get("useBrowserCookies")) and not cookie_file_value
    browser_profile_directory = request.get("browserProfileDirectory") if use_browser_cookies else None
    cookie_file = (
        Path(str(cookie_file_value)).resolve()
        if cookie_file_value
        else None if use_browser_cookies else _development_cookie_file()
    )
    cookies: list[dict[str, Any]] = []
    if cookie_file:
        if not cookie_file.is_file():
            raise FileNotFoundError("Không tìm thấy file cookies đã chọn.")
        cookies = load_netscape_cookies(cookie_file)

    cookie_note = (
        " Đang dùng phiên TikTok đã kết nối trong trình duyệt ứng dụng."
        if use_browser_cookies
        else " Có cookies.txt cục bộ để xác thực." if cookie_file and not cookie_file_value else ""
    )
    emit({"type": "progress", "job": "tiktok_download", "progress": 0.02, "stage": "browser", "message": f"Đang mở TikTok để đọc thông tin series…{cookie_note}"})
    browser = None
    context = None
    media_observer: BrowserMediaObserver | None = None
    with sync_playwright() as playwright:
        try:
            if use_browser_cookies:
                from .browser_session import AUTH_COOKIE_NAMES, has_saved_session

                profile_directory = Path(str(browser_profile_directory or "")).resolve()
                if not has_saved_session(profile_directory):
                    raise RuntimeError("Chưa kết nối phiên TikTok bằng trình duyệt. Hãy đăng nhập TikTok trước.")
                context = playwright.chromium.launch_persistent_context(
                    str(profile_directory),
                    channel="chrome",
                    headless=True,
                    locale=BROWSER_LOCALE,
                )
                browser_cookies = context.cookies(["https://www.tiktok.com"])
                if not any(
                    str(cookie.get("name", "")).lower() in AUTH_COOKIE_NAMES and cookie.get("value")
                    for cookie in browser_cookies
                ):
                    raise RuntimeError("Phiên TikTok đã hết hạn. Hãy kết nối lại bằng trình duyệt rồi thử lại.")
            else:
                browser = playwright.chromium.launch(headless=True, channel="chrome")
                context = browser.new_context(locale=BROWSER_LOCALE)
                if cookies:
                    context.add_cookies(cookies)

            page = context.new_page()
            media_observer = BrowserMediaObserver(page)
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
            download_cookies = context.cookies()
            for index, episode in enumerate(episode_numbers):
                previous_sources = {
                    candidate.url for candidate in browser_video_element_candidates(navigator.page)
                }
                media_observer.start_episode()
                if not navigator.select_episode(episode):
                    raise RuntimeError(f"Không thể chọn tập {episode} trên TikTok.")
                current_video_id, candidates = _wait_for_episode_media(
                    navigator,
                    media_observer,
                    episode,
                    previous_video_id,
                    previous_sources,
                    lambda message: emit({
                        "type": "progress",
                        "job": "tiktok_download",
                        "progress": min(0.98, 0.12 + 0.86 * index / episode_count),
                        "stage": "discover_media",
                        "episode": episode,
                        "episodeIndex": index + 1,
                        "episodeCount": episode_count,
                        "message": message,
                    }),
                )
                if previous_video_id and current_video_id == previous_video_id:
                    raise RuntimeError(f"Video ID không đổi sau khi chọn tập {episode}; đã dừng để tránh tải nhầm tập.")

                base = 0.12 + 0.86 * index / episode_count
                span = 0.86 / episode_count
                _download_episode_direct(
                    candidates,
                    download_cookies,
                    episode,
                    series_dir,
                    lambda done, total: emit({
                        "type": "progress",
                        "job": "tiktok_download",
                        "progress": min(0.98, base + span * (done / total if total else 0.05)),
                        "stage": "download",
                        "episode": episode,
                        "episodeIndex": index + 1,
                        "episodeCount": episode_count,
                        "message": _download_progress_message(episode, done, total),
                    }),
                )
                previous_video_id = current_video_id

            files = sorted(
                (path for path in series_dir.glob("EP*.*") if path.is_file()),
                key=lambda path: path.name.casefold(),
            )
        finally:
            if media_observer is not None:
                media_observer.close()
            try:
                if context is not None:
                    context.close()
            finally:
                if browser is not None:
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


def _wait_for_episode_media(
    navigator: Any,
    observer: BrowserMediaObserver,
    episode: int,
    previous_video_id: str | None,
    previous_sources: set[str],
    on_wait: Callable[[str], None],
) -> tuple[str, list[VideoCandidate]]:
    deadline = monotonic() + 45
    last_notice = 0.0
    next_embedded_check = 0.0
    embedded_video_id: str | None = None
    embedded: list[VideoCandidate] = []
    while monotonic() < deadline:
        video_url = navigator.get_current_video_url()
        current_video_id = get_video_id(video_url or "")
        if navigator.get_active_episode() != episode:
            raise RuntimeError(f"TikTok đã chuyển khỏi tập {episode} trong khi chờ nguồn video.")

        now = monotonic()
        if current_video_id != embedded_video_id:
            embedded_video_id = current_video_id
            embedded = []
            next_embedded_check = 0.0
        if current_video_id and now >= next_embedded_check:
            embedded = embedded_video_candidates(navigator.page, current_video_id)
            next_embedded_check = now + 2.5

        candidates = rank_candidates(
            [
                *(
                    candidate
                    for candidate in browser_video_element_candidates(navigator.page, current_video_id)
                    if not previous_video_id or candidate.url not in previous_sources
                ),
                *embedded,
                *observer.candidates(current_video_id),
            ]
        )
        if current_video_id and current_video_id != previous_video_id and candidates:
            return current_video_id, candidates

        if now - last_notice >= 3:
            on_wait(f"Đang chờ TikTok tải nguồn trực tiếp của tập {episode}…")
            last_notice = now
        navigator.page.wait_for_timeout(350)

    raise RuntimeError(
        f"Không tìm thấy luồng MP4/WebM trực tiếp của tập {episode}. "
        "Tập này có thể chỉ phát bằng HLS/blob hoặc TikTok chưa tải xong; hãy thử lại sau."
    )


def _download_episode_direct(
    candidates: list[VideoCandidate],
    cookies: list[dict[str, Any]],
    episode: int,
    output_dir: Path,
    on_progress: Callable[[int, int | None], None],
) -> Path:
    last_error = "không có nguồn video hợp lệ"
    for candidate in candidates[:12]:
        try:
            return download_video_candidate(candidate, cookies, episode, output_dir, on_progress)
        except Exception as error:
            last_error = str(error).replace("\r", " ").replace("\n", " ")[:240]
    raise RuntimeError(f"Không tải được tập {episode}: {last_error}")


def _download_progress_message(episode: int, downloaded: int, total: int | None) -> str:
    downloaded_mb = downloaded / (1024 * 1024)
    if total and total > 0:
        return f"Đang tải tập {episode:04d} · {downloaded_mb:.1f}/{total / (1024 * 1024):.1f} MB"
    return f"Đang tải tập {episode:04d} · đã nhận {downloaded_mb:.1f} MB"
