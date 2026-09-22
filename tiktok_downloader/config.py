from dataclasses import dataclass
from pathlib import Path


DEFAULT_COOKIE_FILE = Path("cookies.txt")
DEFAULT_OUTPUT_DIR = Path("downloads")

TIKTOK_BASE_URL = "https://www.tiktok.com"
BROWSER_LOCALE = "vi-VN"

PAGE_LOAD_TIMEOUT_MS = 120_000
INITIAL_PAGE_WAIT_MS = 5_000
SHOW_ALL_WAIT_MS = 2_000
TAB_SWITCH_WAIT_MS = 700
EPISODE_CLICK_SETTLE_MS = 1_500
EPISODE_CHANGE_TIMEOUT_MS = 10_000


@dataclass(frozen=True, slots=True)
class AppConfig:
    start_url: str
    total_episodes: int | None
    cookie_file: Path
    output_dir: Path
