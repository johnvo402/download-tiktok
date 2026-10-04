from __future__ import annotations

import json
import os
from pathlib import Path
from time import monotonic, sleep
from typing import Any, Callable

from .config import BROWSER_LOCALE, TIKTOK_BASE_URL

SESSION_MARKER = ".jc-ai-tiktok-session.json"
LOGIN_TIMEOUT_SECONDS = 5 * 60
AUTH_COOKIE_NAMES = {"sessionid", "sessionid_ss", "sid_tt", "sid_guard"}


def session_marker(profile_directory: Path) -> Path:
    return profile_directory / SESSION_MARKER


def has_saved_session(profile_directory: Path) -> bool:
    marker = session_marker(profile_directory)
    try:
        return marker.is_file() and not marker.is_symlink()
    except OSError:
        return False


def capture_browser_session(
    request: dict[str, Any], emit: Callable[[dict[str, Any]], None]
) -> None:
    """Open an app-owned browser profile and wait for the user to log into TikTok."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError as error:
        raise RuntimeError("Không thể mở trình duyệt đăng nhập TikTok. Hãy cập nhật hoặc cài lại ứng dụng.") from error

    profile_directory = Path(str(request.get("profileDirectory", ""))).expanduser()
    if not profile_directory.is_absolute():
        raise ValueError("Không xác định được hồ sơ đăng nhập TikTok của ứng dụng.")
    profile_directory.mkdir(parents=True, exist_ok=True)

    emit({
        "type": "progress",
        "job": "tiktok_cookie_login",
        "progress": 0.05,
        "stage": "browser",
        "message": "Đang mở cửa sổ trình duyệt riêng để đăng nhập TikTok…",
    })

    with sync_playwright() as playwright:
        try:
            context = playwright.chromium.launch_persistent_context(
                str(profile_directory),
                channel="chrome",
                headless=False,
                locale=BROWSER_LOCALE,
                args=["--no-first-run", "--no-default-browser-check"],
            )
        except Exception as error:
            detail = str(error).lower()
            if "executable" in detail or "chrome" in detail or "browser" in detail:
                raise RuntimeError("Không tìm thấy Google Chrome. Cài Chrome rồi thử kết nối TikTok lại.") from error
            raise RuntimeError("Không thể mở cửa sổ đăng nhập TikTok. Hãy đóng các cửa sổ đăng nhập JC AI rồi thử lại.") from error

        try:
            page = context.pages[0] if context.pages else context.new_page()
            try:
                page.goto(f"{TIKTOK_BASE_URL}/login", wait_until="domcontentloaded", timeout=60_000)
            except Exception:
                # TikTok may reject the initial navigation while still rendering a
                # usable login page. Keep the browser open and observe its cookies.
                pass

            deadline = monotonic() + LOGIN_TIMEOUT_SECONDS
            next_notice = monotonic()
            while monotonic() < deadline:
                cookies = context.cookies([TIKTOK_BASE_URL])
                has_auth_cookie = any(
                    str(cookie.get("name", "")).lower() in AUTH_COOKIE_NAMES
                    and bool(cookie.get("value"))
                    for cookie in cookies
                )
                if has_auth_cookie:
                    marker = session_marker(profile_directory)
                    temporary_marker = marker.with_suffix(marker.suffix + ".tmp")
                    temporary_marker.write_text(
                        json.dumps({"schemaVersion": 1}, separators=(",", ":")),
                        encoding="utf-8",
                    )
                    os.replace(temporary_marker, marker)
                    emit({
                        "type": "completed",
                        "job": "tiktok_cookie_login",
                        "progress": 1.0,
                        "stage": "completed",
                        "message": "Đã kết nối phiên đăng nhập TikTok. App sẽ dùng lại phiên này khi tải video.",
                        "details": {"cookieCount": len(cookies)},
                    })
                    return

                if monotonic() >= next_notice:
                    emit({
                        "type": "progress",
                        "job": "tiktok_cookie_login",
                        "progress": 0.2,
                        "stage": "waiting_for_login",
                        "message": "Hãy đăng nhập TikTok trong cửa sổ trình duyệt vừa mở. App đang chờ xác nhận đăng nhập…",
                    })
                    next_notice = monotonic() + 10
                sleep(1)

            raise TimeoutError("Chưa phát hiện đăng nhập TikTok sau 5 phút. Hãy thử kết nối lại.")
        finally:
            context.close()
