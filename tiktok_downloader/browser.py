import re

from playwright.sync_api import Locator, Page, expect

from .config import (
    EPISODE_CHANGE_TIMEOUT_MS,
    EPISODE_CLICK_SETTLE_MS,
    INITIAL_PAGE_WAIT_MS,
    PAGE_LOAD_TIMEOUT_MS,
    SHOW_ALL_WAIT_MS,
    TAB_SWITCH_WAIT_MS,
)
from .metadata import DramaInfo
from .urls import is_video_url, normalize_video_href


class TikTokNavigator:
    """Handle TikTok series metadata and episode navigation."""

    def __init__(self, page: Page) -> None:
        self.page = page
        self._current_tab_index: int | None = None

    def open(self, url: str) -> None:
        self.page.goto(
            url,
            wait_until="domcontentloaded",
            timeout=PAGE_LOAD_TIMEOUT_MS,
        )
        self.page.wait_for_timeout(INITIAL_PAGE_WAIT_MS)

    def try_open_episode_list(self) -> None:
        """Open the full episode panel when TikTok exposes the button."""
        try:
            show_all = self.page.get_by_text(
                re.compile(r"Xem tất cả .* tập", re.I)
            )

            if show_all.count():
                show_all.first.click()
                self.page.wait_for_timeout(SHOW_ALL_WAIT_MS)
                self._current_tab_index = None
        except Exception:
            return

    def get_drama_info(self) -> DramaInfo:
        section = self.page.locator(
            'section[class*="DivTopInfoSection"]'
        ).first

        title = self._safe_inner_text(
            section.locator('h2[class*="HDramaTitle"]').first
        )
        tags = [
            text.strip()
            for text in section.locator(
                '[class*="SpanThemeTag"]'
            ).all_inner_texts()
            if text.strip()
        ]
        description = self._safe_inner_text(
            section.locator('p[class*="PDescription"]').first
        )
        play_count = self._safe_inner_text(
            section.locator(
                '[class*="DivPosterPlayCount"] span'
            ).first,
            default=None,
        )

        poster = section.locator("picture img").first
        poster_url: str | None = None

        try:
            if poster.count():
                poster_url = poster.evaluate(
                    "(img) => img.currentSrc || img.src || null"
                )
        except Exception:
            poster_url = None

        return DramaInfo(
            title=title or "untitled",
            tags=tags,
            description=description or "",
            play_count=play_count,
            poster_url=poster_url,
        )

    def get_current_video_url(self) -> str | None:
        """Get the current video URL from page URL or video links in the DOM."""
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

    def discover_episode_numbers(self) -> list[int]:
        """
        Discover episode numbers by visiting tabs by index.

        Tab labels such as "1-24" or "25-48" are intentionally ignored.
        """
        active_episode = self.get_active_episode()
        numbers: set[int] = set()
        tab_count = self.get_episode_tab_count()

        if tab_count == 0:
            return self.get_visible_episode_numbers()

        # TikTok can already have tab 0 selected. Detect it before
        # scanning to avoid clicking the selected tab unnecessarily.
        self._current_tab_index = self._detect_selected_tab_index()

        for index in range(tab_count):
            self._switch_to_tab(index)
            numbers.update(self.get_visible_episode_numbers())

        if active_episode is not None:
            self._ensure_episode_visible(active_episode)

        return sorted(numbers)

    def get_episode_tab_count(self) -> int:
        return self._episode_tabs().count()

    def get_visible_episode_numbers(self) -> list[int]:
        numbers: list[int] = []

        for index in range(self._episode_buttons().count()):
            text = self._episode_buttons().nth(index).inner_text().strip()

            if text.isdigit():
                numbers.append(int(text))

        return numbers

    def get_active_episode(self) -> int | None:
        active_button = self._active_episode_button()

        if not active_button.count():
            return None

        text = active_button.first.inner_text().strip()
        return int(text) if text.isdigit() else None

    def select_episode(self, episode: int) -> bool:
        """
        Select an episode from the current grid.

        If the requested episode is not visible, tabs are visited by index
        until the matching episode button appears.
        """
        if self.get_active_episode() == episode:
            return True

        if not self._ensure_episode_visible(episode):
            return False

        button = self._episode_button(episode)

        if not button.count():
            return False

        button.first.click()

        try:
            expect(self._active_episode_button()).to_have_text(
                re.compile(rf"^\s*{episode}\s*$"),
                timeout=EPISODE_CHANGE_TIMEOUT_MS,
            )
        except Exception:
            return False

        self.page.wait_for_timeout(EPISODE_CLICK_SETTLE_MS)
        return self.get_active_episode() == episode

    def _ensure_episode_visible(self, episode: int) -> bool:
        visible = self.get_visible_episode_numbers()

        if episode in visible:
            return True

        tab_count = self.get_episode_tab_count()
        if tab_count == 0:
            return False

        indexes = self._tab_search_order(
            episode=episode,
            visible=visible,
            tab_count=tab_count,
        )

        for index in indexes:
            self._switch_to_tab(index)

            if episode in self.get_visible_episode_numbers():
                return True

        return False

    def _tab_search_order(
        self,
        episode: int,
        visible: list[int],
        tab_count: int,
    ) -> list[int]:
        current = self._current_tab_index

        if current is None:
            return list(range(tab_count))

        if visible and episode > max(visible):
            return [
                *range(current + 1, tab_count),
                *range(0, current),
            ]

        if visible and episode < min(visible):
            return [
                *range(current - 1, -1, -1),
                *range(current + 1, tab_count),
            ]

        return [
            index
            for index in range(tab_count)
            if index != current
        ]

    def _switch_to_tab(self, index: int) -> None:
        tabs = self._episode_tabs()

        if index < 0 or index >= tabs.count():
            raise IndexError(f"Episode tab index out of range: {index}")

        if self._current_tab_index is None:
            self._current_tab_index = self._detect_selected_tab_index()

        if self._current_tab_index == index:
            return

        # Use a normal click so Playwright still checks whether a real
        # overlay is blocking the target. Do not bypass it with force=True.
        tabs.nth(index).click()
        self.page.wait_for_timeout(TAB_SWITCH_WAIT_MS)
        self._current_tab_index = index

    def _detect_selected_tab_index(self) -> int | None:
        """Find the tab under TikTok's visual selection indicator."""
        tabs = self._episode_tabs()

        if not tabs.count():
            return None

        try:
            return tabs.first.evaluate(
                """(tab) => {
                    const control = tab.closest(
                        '[data-testid="tux-segmented-control"]'
                    );
                    const indicator = control?.querySelector(
                        '[class*="tux-segmented-control__indicator"]'
                    );

                    if (!indicator) return null;

                    const indicatorRect = indicator.getBoundingClientRect();
                    if (!indicatorRect.width) return null;

                    const indicatorCenter =
                        indicatorRect.left + indicatorRect.width / 2;
                    const buttons = Array.from(control.querySelectorAll(
                        '[data-testid="tux-segment-item"]'
                    ));

                    const selected = buttons.findIndex((button) => {
                        const rect = button.getBoundingClientRect();
                        return (
                            indicatorCenter >= rect.left &&
                            indicatorCenter <= rect.right
                        );
                    });

                    return selected >= 0 ? selected : null;
                }"""
            )
        except Exception:
            return None

    def _episode_section(self) -> Locator:
        # Clicking "Xem tất cả ... tập" may create a floating panel while
        # the original grid remains in the DOM behind it. The popup is
        # portaled after the page content, so prefer the LAST visible grid.
        return self.page.locator(
            'section:visible:has(> h3:text-is("Tập"))'
        ).last

    def _episode_tabs(self) -> Locator:
        return self._episode_section().locator(
            '[data-testid="tux-segment-item"]'
        )

    def _episode_buttons(self) -> Locator:
        return self._episode_section().locator(
            'button[class*="ButtonEpisode"]'
        )

    def _episode_button(self, episode: int) -> Locator:
        return self._episode_buttons().filter(
            has_text=re.compile(rf"^\s*{episode}\s*$")
        )

    def _active_episode_button(self) -> Locator:
        return self._episode_section().locator(
            'button[class*="ButtonEpisode"]:has('
            '[class*="SpanEpisodeActiveIcon"]'
            ")"
        )

    @staticmethod
    def _safe_inner_text(
        locator: Locator,
        default: str | None = "",
    ) -> str | None:
        try:
            if locator.count():
                return locator.inner_text().strip()
        except Exception:
            pass

        return default
