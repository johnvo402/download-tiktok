from dataclasses import dataclass
from pathlib import Path


DEFAULT_COOKIE_FILE = Path("cookies.txt")
DEFAULT_OUTPUT_DIR = Path("downloads")

TIKTOK_BASE_URL = "https://www.tiktok.com"
BROWSER_LOCALE = "vi-VN"

PAGE_LOAD_TIMEOUT_MS = 120_000
PAGE_CONTENT_TIMEOUT_MS = 60_000
EPISODE_LIST_LOAD_TIMEOUT_MS = 60_000
EPISODE_LIST_POLL_MS = 400
EPISODE_LIST_STABLE_MS = 1_600
EPISODE_SCROLL_QUIET_MS = 2_200
SHOW_ALL_BUTTON_TIMEOUT_MS = 30_000
CINEMA_MODE_SETTLE_MS = 5_000
TAB_SWITCH_WAIT_MS = 700
EPISODE_CLICK_SETTLE_MS = 1_500
EPISODE_CHANGE_TIMEOUT_MS = 10_000


@dataclass(frozen=True, slots=True)
class AppConfig:
    start_url: str
    total_episodes: int | None
    cookie_file: Path
    output_dir: Path
