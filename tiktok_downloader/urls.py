import re

from .config import TIKTOK_BASE_URL


_VIDEO_ID_PATTERN = re.compile(r"/video/(\d+)")
_MARKDOWN_URL_PATTERN = re.compile(r"^\[.*?\]\((https?://.+?)\)$")


def normalize_url(url: str) -> str:
    """Normalize a plain TikTok URL or a URL copied in Markdown form."""
    value = url.strip()
    markdown_match = _MARKDOWN_URL_PATTERN.match(value)

    if markdown_match:
        return markdown_match.group(1)

    return value


def get_video_id(url: str) -> str | None:
    """Extract a TikTok video id from a video URL."""
    match = _VIDEO_ID_PATTERN.search(url)
    return match.group(1) if match else None


def normalize_video_href(href: str) -> str:
    """Convert a TikTok video href into an absolute URL without query params."""
    if href.startswith("/"):
        href = f"{TIKTOK_BASE_URL}{href}"

    return href.split("?", 1)[0]


def is_video_url(url: str) -> bool:
    return bool(_VIDEO_ID_PATTERN.search(url))
