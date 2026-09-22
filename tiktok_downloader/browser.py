import re

from playwright.sync_api import Page

from .config import (
    INITIAL_PAGE_WAIT_MS,
    NEXT_EPISODE_WAIT_MS,
    PAGE_LOAD_TIMEOUT_MS,
    SHOW_ALL_WAIT_MS,
)
from .urls import is_video_url, normalize_video_href


class TikTokNavigator:
    """Handle browser interactions required to move through a TikTok series."""

    def __init__(self, page: Page) -> None:
        self.page = page

    def open(self, url: str) -> None:
        self.page.goto(
            url,
            wait_until="domcontentloaded",
            timeout=PAGE_LOAD_TIMEOUT_MS,
        )
        self.page.wait_for_timeout(INITIAL_PAGE_WAIT_MS)

    def try_open_episode_list(self) -> None:
        """Open the "Xem tất cả ... tập" panel when TikTok shows it."""
        try:
            show_all = self.page.get_by_text(
                re.compile(r"Xem tất cả .* tập", re.I)
            )

            if show_all.count():
                show_all.first.click()
                self.page.wait_for_timeout(SHOW_ALL_WAIT_MS)
        except Exception:
            return

    def get_current_video_url(self) -> str | None:
        """Get the current video URL from the page URL or video links in DOM."""
        current_url = self.page.url

        if is_video_url(current_url):
            return current_url.split("?", 1)[0]

        links = self.page.locator('a[href*="/video/"]')

        for index in range(links.count()):
            try:
                href = links.nth(index).get_attribute("href")
            except Exception:
                continue

            if href and is_video_url(href):
                return normalize_video_href(href)

        return None

    def go_to_next_episode(self) -> None:
        """Scroll the video area to move TikTok to the next episode."""
        viewport = self.page.viewport_size

        if viewport:
            self.page.mouse.move(
                viewport["width"] * 0.45,
                viewport["height"] * 0.5,
            )

        self.page.mouse.wheel(0, 800)
        self.page.wait_for_timeout(NEXT_EPISODE_WAIT_MS)
