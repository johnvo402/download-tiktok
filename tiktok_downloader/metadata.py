import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path

from playwright.sync_api import APIRequestContext


_INVALID_WINDOWS_CHARS = re.compile(r'[<>:"/\\|?*\x00-\x1f]')
_RESERVED_WINDOWS_NAMES = {
    "CON",
    "PRN",
    "AUX",
    "NUL",
    *(f"COM{index}" for index in range(1, 10)),
    *(f"LPT{index}" for index in range(1, 10)),
}


@dataclass(frozen=True, slots=True)
class DramaInfo:
    title: str
    tags: list[str]
    description: str
    play_count: str | None
    poster_url: str | None


def sanitize_folder_name(value: str) -> str:
    """Return a Windows-safe folder name."""
    name = _INVALID_WINDOWS_CHARS.sub("_", value).strip().rstrip(". ")

    if not name:
        name = "untitled"

    if name.upper() in _RESERVED_WINDOWS_NAMES:
        name = f"_{name}"

    return name[:120].rstrip(". ")


def create_series_directory(base_dir: Path, title: str) -> Path:
    series_dir = base_dir / sanitize_folder_name(title)
    series_dir.mkdir(parents=True, exist_ok=True)
    return series_dir


def _poster_extension(content_type: str) -> str:
    mime = content_type.split(";", 1)[0].strip().lower()

    return {
        "image/avif": ".avif",
        "image/jpeg": ".jpg",
        "image/jpg": ".jpg",
        "image/png": ".png",
        "image/webp": ".webp",
    }.get(mime, ".jpg")


def download_poster(
    request: APIRequestContext,
    poster_url: str | None,
    series_dir: Path,
) -> str | None:
    """Download the poster with the same browser context used for TikTok."""
    if not poster_url:
        return None

    try:
        response = request.get(
            poster_url,
            timeout=120_000,
        )
    except Exception as exc:
        print(f"[!] Không tải được poster: {exc}")
        return None

    if not response.ok:
        print(f"[!] Poster HTTP {response.status}")
        return None

    extension = _poster_extension(response.headers.get("content-type", ""))
    poster_path = series_dir / f"poster{extension}"
    poster_path.write_bytes(response.body())

    return poster_path.name


def write_metadata(
    info: DramaInfo,
    series_dir: Path,
    source_url: str,
    episode_numbers: list[int],
    poster_file: str | None,
) -> Path:
    metadata = {
        **asdict(info),
        "source_url": source_url,
        "poster_file": poster_file,
        "total_episodes": len(episode_numbers),
        "episode_numbers": episode_numbers,
    }

    metadata_path = series_dir / "metadata.json"
    metadata_path.write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )

    return metadata_path
