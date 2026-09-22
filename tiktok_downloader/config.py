from dataclasses import dataclass
from pathlib import Path


DEFAULT_COOKIE_FILE = Path("cookies.txt")
DEFAULT_OUTPUT_DIR = Path("downloads")
DEFAULT_TOTAL_EPISODES = 82

TIKTOK_BASE_URL = "https://www.tiktok.com"
BROWSER_LOCALE = "vi-VN"
PAGE_LOAD_TIMEOUT_MS = 120_000
INITIAL_PAGE_WAIT_MS = 5_000
NEXT_EPISODE_WAIT_MS = 2_800
SHOW_ALL_WAIT_MS = 2_000


@dataclass(frozen=True, slots=True)
class AppConfig:
    start_url: str
    total_episodes: int
    cookie_file: Path
    output_dir: Path
