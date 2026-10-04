import re
from time import monotonic

from playwright.sync_api import Locator, Page, expect

from .config import (
    EPISODE_CHANGE_TIMEOUT_MS,
    EPISODE_CLICK_SETTLE_MS,
    EPISODE_LIST_LOAD_TIMEOUT_MS,
    EPISODE_LIST_POLL_MS,
    EPISODE_LIST_STABLE_MS,
    CINEMA_MODE_SETTLE_MS,
    EPISODE_SCROLL_QUIET_MS,
    PAGE_LOAD_TIMEOUT_MS,
    PAGE_CONTENT_TIMEOUT_MS,
    SHOW_ALL_BUTTON_TIMEOUT_MS,
    TAB_SWITCH_WAIT_MS,
)
from .metadata import DramaInfo
from .urls import is_video_url, normalize_video_href

MAX_EPISODES = 10_000


class TikTokNavigator:
    """Handle TikTok series metadata and episode navigation."""

    def __init__(self, page: Page) -> None:
        self.page = page
        self._current_tab_index: int | None = None
        self._expected_episode_count: int | None = None

    def open(self, url: str) -> None:
        navigation_response: dict[str, int] = {}

        def capture_navigation_response(response) -> None:
            try:
                request = response.request
                if request.is_navigation_request() and request.frame == self.page.main_frame:
                    navigation_response["status"] = response.status
            except Exception:
                pass

        self.page.on("response", capture_navigation_response)
        try:
            response = self.page.goto(
                url,
                wait_until="commit",
                timeout=PAGE_LOAD_TIMEOUT_MS,
            )
            status = response.status if response else navigation_response.get("status")
            if status is not None and not 200 <= status < 400:
                raise RuntimeError(self._navigation_failure_message(status=status))
            self.page.wait_for_load_state(
                "domcontentloaded",
                timeout=PAGE_LOAD_TIMEOUT_MS,
            )
        except RuntimeError:
            raise
        except Exception as error:
            raise RuntimeError(
                self._navigation_failure_message(
                    error=error,
                    status=navigation_response.get("status"),
                )
            ) from error
        finally:
            self.page.remove_listener("response", capture_navigation_response)

        try:
            self.page.wait_for_function(
                """() => {
                    const title = document.querySelector(
                        'section[class*="DivTopInfoSection"] h2[class*="HDramaTitle"]'
                    )?.textContent?.trim();
                    const cinemaEntry = document.querySelector(
                        '[data-testid="short-drama-cinema-mode-entry"]'
                    );
                    const episode = document.querySelector(
                        'button[class*="ButtonEpisode"]'
                    );
                    return Boolean(title || cinemaEntry || episode);
                }""",
                timeout=PAGE_CONTENT_TIMEOUT_MS,
                polling=EPISODE_LIST_POLL_MS,
            )
        except Exception as error:
            raise RuntimeError(
                "TikTok chưa tải được nội dung series sau 60 giây. Kiểm tra kết nối mạng rồi thử lại."
            ) from error

    @staticmethod
    def _navigation_failure_message(
        error: Exception | None = None,
        status: int | None = None,
    ) -> str:
        if status == 429:
            return "TikTok đang giới hạn yêu cầu (HTTP 429). Chờ một lúc rồi thử lại."
        if status in {401, 403}:
            return (
                f"TikTok từ chối truy cập (HTTP {status}). Kiểm tra cookies/đăng nhập "
                "hoặc thử lại sau."
            )
        if status is not None and status >= 500:
            return f"TikTok đang lỗi máy chủ (HTTP {status}). Chờ một lúc rồi thử lại."
        if status is not None:
            return f"TikTok trả lỗi HTTP {status} khi mở trang phim. Kiểm tra link rồi thử lại."
        detail = str(error or "")
        code = re.search(r"net::(ERR_[A-Z0-9_]+)", detail)
        if code and code.group(1) == "ERR_HTTP_RESPONSE_CODE_FAILURE":
            return (
                "TikTok không hoàn tất phản hồi trang (ERR_HTTP_RESPONSE_CODE_FAILURE). "
                "Có thể máy chủ hoặc mạng đang từ chối yêu cầu; kiểm tra cookies/kết nối "
                "rồi thử lại sau."
            )
        if code:
            return f"Không thể tải trang TikTok ({code.group(1)}). Kiểm tra kết nối mạng rồi thử lại."
        if "Timeout" in detail or "timed out" in detail.lower():
            return "TikTok phản hồi quá chậm, hết thời gian chờ tải trang. Kiểm tra mạng rồi thử lại."
        return "Không thể mở trang TikTok. Kiểm tra đường dẫn, cookies và kết nối mạng rồi thử lại."

    def try_open_episode_list(self) -> None:
        """Enter TikTok's cinema mode, then wait for its episode list to load."""
        try:
            self.page.wait_for_function(
                """() => Boolean(
                    document.querySelector('[data-testid="short-drama-cinema-mode-entry"]') ||
                    /Xem tất cả .* tập/i.test(document.body?.innerText || '') ||
                    document.querySelector('button[class*="ButtonEpisode"]')
                )""",
                timeout=SHOW_ALL_BUTTON_TIMEOUT_MS,
                polling=EPISODE_LIST_POLL_MS,
            )
            cinema_entry = self.page.locator(
                '[data-testid="short-drama-cinema-mode-entry"]'
            ).first
            if cinema_entry.count() and cinema_entry.is_visible():
                self._remember_episode_count(cinema_entry.inner_text())
                previous_url = self.page.url
                cinema_entry.click()
                self._current_tab_index = None
                self.page.wait_for_timeout(CINEMA_MODE_SETTLE_MS)
                self._wait_for_cinema_mode_content(previous_url)
                return

            show_all = self.page.get_by_text(
                re.compile(r"Xem tất cả .* tập", re.I)
            )
            if show_all.count() and show_all.first.is_visible():
                label = show_all.first.inner_text().strip()
                self._remember_episode_count(label)
                previous_url = self.page.url
                show_all.first.click()
                self._current_tab_index = None
                self._wait_for_cinema_mode_content(previous_url)
        except Exception as error:
            if isinstance(error, RuntimeError):
                raise
            return

    def _remember_episode_count(self, label: str) -> None:
        match = re.search(r"([\d,.]+)\s*tập", label, re.IGNORECASE)
        if not match:
            return
        count = int(re.sub(r"\D", "", match.group(1)))
        if 0 < count <= MAX_EPISODES:
            self._expected_episode_count = count

    def _wait_for_cinema_mode_content(self, previous_url: str) -> None:
        """Wait for the cinema-mode transition, document load, and episode data."""
        try:
            self.page.wait_for_function(
                """(oldUrl) => {
                    const entry = document.querySelector(
                        '[data-testid="short-drama-cinema-mode-entry"]'
                    );
                    const rect = entry?.getBoundingClientRect();
                    const entryVisible = Boolean(
                        rect && rect.width > 0 && rect.height > 0 &&
                        getComputedStyle(entry).visibility !== 'hidden'
                    );
                    const visibleEpisode = Array.from(document.querySelectorAll(
                        'button[class*="ButtonEpisode"]'
                    )).some((button) => {
                        const buttonRect = button.getBoundingClientRect();
                        return buttonRect.width > 0 && buttonRect.height > 0 &&
                            getComputedStyle(button).visibility !== 'hidden';
                    });
                    return location.href !== oldUrl || !entryVisible || visibleEpisode;
                }""",
                arg=previous_url,
                timeout=PAGE_CONTENT_TIMEOUT_MS,
                polling=EPISODE_LIST_POLL_MS,
            )
        except Exception as error:
            raise RuntimeError(
                "Đã nhấn mở chế độ phim ngắn nhưng trang không chuyển sang danh sách tập. "
                "Kiểm tra kết nối mạng rồi thử lại."
            ) from error

        if self.page.url != previous_url:
            try:
                self.page.wait_for_load_state(
                    "load",
                    timeout=PAGE_CONTENT_TIMEOUT_MS,
                )
            except Exception as error:
                raise RuntimeError(
                    "TikTok đã chuyển trang nhưng chưa tải xong. "
                    "Kiểm tra kết nối mạng rồi thử lại."
                ) from error

        if not self.wait_for_episode_list():
            raise RuntimeError(
                "Trang phim ngắn đã mở nhưng danh sách tập chưa tải xong sau 60 giây. "
                "Kiểm tra kết nối mạng rồi thử lại."
            )

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

        Read every displayed tab and scroll its episode list so virtualized
        rows are included. TikTok's “Xem tất cả N tập” label and tab ranges
        provide a second source of truth when only part of a list is rendered.
        """
        if not self.wait_for_episode_list():
            return []

        active_episode = self.get_active_episode()
        numbers: set[int] = set()
        tab_count = self.get_episode_tab_count()

        if tab_count == 0:
            numbers = self._collect_current_tab_episode_numbers()
            if self._expected_episode_count and numbers and min(numbers) == 1:
                if max(numbers) <= self._expected_episode_count:
                    numbers.update(range(1, self._expected_episode_count + 1))
            return sorted(numbers)

        # TikTok can already have tab 0 selected. Detect it before
        # scanning to avoid clicking the selected tab unnecessarily.
        self._current_tab_index = self._detect_selected_tab_index()

        for index in range(tab_count):
            self._switch_to_tab(index)
            numbers.update(self._collect_current_tab_episode_numbers())

        for label in self._episode_tab_labels():
            for first, last in re.findall(r"(\d+)\s*[-–—]\s*(\d+)", label):
                start, end = int(first), int(last)
                if 0 < start <= end <= MAX_EPISODES:
                    numbers.update(range(start, end + 1))

        if self._expected_episode_count and numbers and min(numbers) == 1:
            if max(numbers) <= self._expected_episode_count:
                numbers.update(range(1, self._expected_episode_count + 1))

        if active_episode is not None:
            self._ensure_episode_visible(active_episode)

        return sorted(numbers)

    def wait_for_episode_list(self) -> bool:
        """Wait for delayed TikTok SPA content instead of sampling the DOM once."""
        deadline = monotonic() + EPISODE_LIST_LOAD_TIMEOUT_MS / 1000
        previous: tuple[int, ...] | None = None
        stable_since: float | None = None
        while monotonic() < deadline:
            try:
                current = tuple(self.get_visible_episode_numbers())
            except Exception:
                current = ()

            if current:
                now = monotonic()
                if current == previous:
                    if stable_since is not None and (now - stable_since) * 1000 >= EPISODE_LIST_STABLE_MS:
                        return True
                else:
                    stable_since = now
                previous = current
            else:
                previous = None
                stable_since = None
            self.page.wait_for_timeout(EPISODE_LIST_POLL_MS)
        return False

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

        button.first.scroll_into_view_if_needed(timeout=EPISODE_CHANGE_TIMEOUT_MS)
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
        if self._seek_episode_in_current_tab(episode):
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

            if self._seek_episode_in_current_tab(episode):
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

        previous_tab_index = self._current_tab_index
        previous_numbers = tuple(self.get_visible_episode_numbers())

        # Use a normal click so Playwright still checks whether a real
        # overlay is blocking the target. Do not bypass it with force=True.
        tabs.nth(index).click()
        try:
            self.page.wait_for_function(
                """(index) => {
                    const sections = Array.from(document.querySelectorAll('section'));
                    const section = sections.filter((candidate) => {
                        const rect = candidate.getBoundingClientRect();
                        return rect.width > 0 && rect.height > 0 &&
                            Array.from(candidate.children).some((child) =>
                                child.tagName === 'H3' && child.innerText.trim() === 'Tập'
                            );
                    }).at(-1);
                    const tab = section?.querySelectorAll('[data-testid="tux-segment-item"]')[index];
                    if (!tab) return false;
                    if (tab.getAttribute('aria-selected') === 'true' ||
                        tab.getAttribute('aria-checked') === 'true' ||
                        tab.getAttribute('data-state') === 'active') return true;
                    const control = tab.closest('[data-testid="tux-segmented-control"]');
                    const indicator = control?.querySelector('[class*="tux-segmented-control__indicator"]');
                    if (!indicator) return false;
                    const mark = indicator.getBoundingClientRect();
                    const target = tab.getBoundingClientRect();
                    const center = mark.left + mark.width / 2;
                    return mark.width > 0 && center >= target.left && center <= target.right;
                }""",
                arg=index,
                timeout=max(TAB_SWITCH_WAIT_MS, 5_000),
            )
        except Exception:
            # Some TikTok builds expose neither selected attributes nor an
            # indicator. The stable-content wait below is the fallback.
            pass
        self._wait_for_episode_list_to_settle(
            previous_numbers if previous_tab_index is not None else None
        )
        self._current_tab_index = index

    def _wait_for_episode_list_to_settle(
        self,
        previous_numbers: tuple[int, ...] | None = None,
    ) -> None:
        deadline = monotonic() + min(EPISODE_LIST_LOAD_TIMEOUT_MS, 15_000) / 1000
        previous: tuple[int, ...] | None = None
        stable_since: float | None = None
        while monotonic() < deadline:
            current = tuple(self.get_visible_episode_numbers())
            if current and (previous_numbers is None or current != previous_numbers):
                now = monotonic()
                if current == previous:
                    if stable_since is not None and (now - stable_since) * 1000 >= EPISODE_LIST_STABLE_MS:
                        return
                else:
                    stable_since = now
                previous = current
            else:
                previous = None
                stable_since = None
            self.page.wait_for_timeout(EPISODE_LIST_POLL_MS)

    def _episode_tab_labels(self) -> list[str]:
        try:
            return [label.strip() for label in self._episode_tabs().all_inner_texts()]
        except Exception:
            return []

    def _collect_current_tab_episode_numbers(self) -> set[int]:
        numbers = set(self.get_visible_episode_numbers())
        self._scroll_episode_list("top")
        self.page.wait_for_timeout(EPISODE_LIST_POLL_MS)
        deadline = monotonic() + EPISODE_LIST_LOAD_TIMEOUT_MS / 1000
        quiet_since: float | None = None
        while monotonic() < deadline:
            state = self._scroll_episode_list("next")
            previous_count = len(numbers)
            numbers.update(state["numbers"])
            if state["done"]:
                if len(numbers) > previous_count:
                    quiet_since = None
                elif quiet_since is None:
                    quiet_since = monotonic()
                elif (monotonic() - quiet_since) * 1000 >= EPISODE_SCROLL_QUIET_MS:
                    break
                self.page.wait_for_timeout(EPISODE_LIST_POLL_MS)
            else:
                quiet_since = None
                self.page.wait_for_timeout(180)
        numbers.update(self.get_visible_episode_numbers())
        self._scroll_episode_list("top")
        return numbers

    def _seek_episode_in_current_tab(self, episode: int) -> bool:
        button = self._episode_button(episode)
        if button.count():
            button.first.scroll_into_view_if_needed(timeout=EPISODE_CHANGE_TIMEOUT_MS)
            return True

        self._scroll_episode_list("top")
        for _ in range(100):
            state = self._scroll_episode_list("next")
            button = self._episode_button(episode)
            if button.count():
                button.first.scroll_into_view_if_needed(timeout=EPISODE_CHANGE_TIMEOUT_MS)
                return True
            if state["done"]:
                break
            self.page.wait_for_timeout(180)
        return False

    def _scroll_episode_list(self, action: str) -> dict[str, object]:
        return self._episode_section().evaluate(
            """(root, action) => {
                const numberList = () => Array.from(
                    root.querySelectorAll('button[class*="ButtonEpisode"]')
                ).map((button) => button.innerText.trim())
                    .filter((text) => /^\\d+$/.test(text))
                    .map((text) => Number(text));
                const nodes = [root, ...root.querySelectorAll('*')];
                const scrollable = nodes.filter((node) => {
                    const style = getComputedStyle(node);
                    return /(auto|scroll)/.test(style.overflowY) &&
                        node.scrollHeight > node.clientHeight + 8;
                }).sort((left, right) =>
                    (right.scrollHeight - right.clientHeight) -
                    (left.scrollHeight - left.clientHeight)
                )[0];
                if (!scrollable) return { numbers: numberList(), done: true };
                if (action === 'top') {
                    scrollable.scrollTop = 0;
                    return { numbers: numberList(), done: false };
                }
                const max = scrollable.scrollHeight - scrollable.clientHeight;
                const step = Math.max(240, Math.floor(scrollable.clientHeight * 0.8));
                scrollable.scrollTop = Math.min(max, scrollable.scrollTop + step);
                return {
                    numbers: numberList(),
                    done: scrollable.scrollTop >= max - 2,
                };
            }""",
            action,
        )

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
