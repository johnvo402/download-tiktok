from __future__ import annotations

import json
import logging
import math
import subprocess
import os
import re
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable
from urllib.parse import urlsplit, urljoin

from .urls import get_video_id

log = logging.getLogger(__name__)

_VIDEO_MIMES = {
    "video/mp4",
    "video/webm",
    "video/quicktime",
    "video/x-m4v",
    "application/mp4",
    "application/octet-stream",
    "binary/octet-stream",
}
_ALLOWED_HOST_SUFFIXES = (
    "tiktok.com",
    "tiktokcdn.com",
    "tiktokcdn-us.com",
    "tiktokcdn-eu.com",
    "tiktokv.com",
    "byteoversea.com",
    "ibytedtos.com",
    "muscdn.com",
)
_NON_VIDEO_EXTENSIONS = re.compile(
    r"\.(?:jpe?g|png|webp|avif|vtt|srt|ttml|ass|ssa|xml|json|m3u8|mpd|m4s|ts|cmf[av])$",
    re.IGNORECASE,
)
_SUBTITLE_HINTS = re.compile(
    r"(?:subtitle|caption|timedtext|texttrack|closedcaption|mime_type=text)",
    re.IGNORECASE,
)
_VIDEO_ID_KEYS = {"id", "itemid", "item_id", "aweme_id"}
_SKIP_VIDEO_KEYS = re.compile(
    r"cover|avatar|thumb|music|author|subtitle|caption|texttrack|lyric|vtt|srt",
    re.IGNORECASE,
)
_MAX_EMBEDDED_SCRIPT_CHARS = 12_000_000
_MAX_JSON_NODES = 60_000


@dataclass(frozen=True, slots=True)
class VideoCandidate:
    url: str
    mime: str = ""
    bytes: int = 0
    width: int = 0
    height: int = 0
    bitrate: int = 0
    source: str = "browser"
    video_id: str | None = None


def normalize_video_candidate(
    value: object,
    *,
    mime: str = "",
    byte_count: int = 0,
    width: int = 0,
    height: int = 0,
    bitrate: int = 0,
    source: str = "browser",
    video_id: str | None = None,
) -> VideoCandidate | None:
    """Accept only HTTPS media URLs on TikTok/CDN domains; reject HLS and text tracks."""
    if not isinstance(value, str) or len(value) > 16_384:
        return None
    try:
        parsed = urlsplit(value.strip())
        host = (parsed.hostname or "").lower().rstrip(".")
        if parsed.scheme.lower() != "https" or not host or parsed.username or parsed.password:
            return None
        if not any(host == suffix or host.endswith(f".{suffix}") for suffix in _ALLOWED_HOST_SUFFIXES):
            return None
        if parsed.port not in (None, 443):
            return None
    except ValueError:
        return None

    path_and_query = f"{parsed.path}?{parsed.query}"
    if _NON_VIDEO_EXTENSIONS.search(parsed.path) or _SUBTITLE_HINTS.search(path_and_query):
        return None
    normalized_mime = mime.split(";", 1)[0].strip().lower()
    if normalized_mime and normalized_mime not in _VIDEO_MIMES:
        return None
    clean_url = parsed._replace(fragment="").geturl()
    return VideoCandidate(
        url=clean_url,
        mime=normalized_mime,
        bytes=max(0, _safe_int(byte_count)),
        width=max(0, _safe_int(width)),
        height=max(0, _safe_int(height)),
        bitrate=max(0, _safe_int(bitrate)),
        source=source,
        video_id=video_id,
    )


def rank_candidates(candidates: Iterable[VideoCandidate]) -> list[VideoCandidate]:
    """Deduplicate captured sources and prefer the highest quality rendition."""
    unique: dict[str, VideoCandidate] = {}
    for candidate in candidates:
        previous = unique.get(candidate.url)
        if previous is None:
            unique[candidate.url] = candidate
            continue
        unique[candidate.url] = VideoCandidate(
            url=candidate.url,
            mime=candidate.mime or previous.mime,
            bytes=max(candidate.bytes, previous.bytes),
            width=max(candidate.width, previous.width),
            height=max(candidate.height, previous.height),
            bitrate=max(candidate.bitrate, previous.bitrate),
            source=previous.source if previous.source != "network" else candidate.source,
            video_id=candidate.video_id or previous.video_id,
        )
    return sorted(
        unique.values(),
        key=lambda item: (item.bitrate, item.bytes, item.width * item.height),
        reverse=True,
    )


class BrowserMediaObserver:
    """Collect direct video responses made by the TikTok page in Playwright."""

    def __init__(self, page: Any) -> None:
        self.page = page
        self._started_at = time.monotonic()
        self._candidates: dict[str, VideoCandidate] = {}
        page.on("response", self._on_response)

    def start_episode(self) -> None:
        self._started_at = time.monotonic()
        self._candidates.clear()

    def close(self) -> None:
        try:
            self.page.remove_listener("response", self._on_response)
        except Exception:
            pass

    def candidates(self, video_id: str | None) -> list[VideoCandidate]:
        if not video_id:
            return []
        return rank_candidates(
            candidate for candidate in self._candidates.values()
            if candidate.video_id == video_id
        )

    def _on_response(self, response: Any) -> None:
        if time.monotonic() < self._started_at:
            return
        try:
            if response.status not in (200, 206):
                return
            headers = response.headers
            mime = str(headers.get("content-type", "")).split(";", 1)[0].strip().lower()
            resource_type = response.request.resource_type
            if mime not in _VIDEO_MIMES and resource_type != "media":
                return
            byte_count = _response_total_bytes(headers)
            candidate = normalize_video_candidate(
                response.url,
                mime=mime,
                byte_count=byte_count,
                source="network",
                video_id=get_video_id(self.page.url),
            )
            if candidate:
                self._candidates[candidate.url] = candidate
                if len(self._candidates) > 100:
                    self._candidates.pop(next(iter(self._candidates)))
        except Exception:
            # A failed/expired response must not interrupt episode navigation.
            return


def browser_video_element_candidates(page: Any, video_id: str | None = None) -> list[VideoCandidate]:
    try:
        urls = page.locator("video").evaluate_all(
            "videos => videos.flatMap(video => [video.currentSrc, video.src, "
            "...Array.from(video.querySelectorAll('source'), source => source.src)])"
        )
    except Exception:
        return []
    return [
        candidate
        for url in urls
        if (candidate := normalize_video_candidate(url, source="video", video_id=video_id))
    ]


def embedded_video_candidates(page: Any, video_id: str) -> list[VideoCandidate]:
    """Read matching TikTok hydration JSON in Python and extract that item's video URLs."""
    selectors = (
        "script#__UNIVERSAL_DATA_FOR_REHYDRATION__,"
        "script#SIGI_STATE, script[type='application/json']"
    )
    try:
        scripts = page.locator(selectors).all()
    except Exception:
        return []

    found: list[VideoCandidate] = []
    visited: set[int] = set()
    budget = [0]

    def collect_video(value: Any, quality: dict[str, int] | None = None) -> None:
        if budget[0] >= _MAX_JSON_NODES or len(found) >= 80:
            return
        budget[0] += 1
        inherited = quality or {}
        if isinstance(value, str):
            if value.startswith("https://"):
                candidate = normalize_video_candidate(
                    value,
                    width=inherited.get("width", 0),
                    height=inherited.get("height", 0),
                    bitrate=inherited.get("bitrate", 0),
                    source="embedded",
                    video_id=video_id,
                )
                if candidate:
                    found.append(candidate)
            return
        if isinstance(value, list):
            for item in value:
                collect_video(item, inherited)
            return
        if not isinstance(value, dict):
            return

        current = {
            "width": _safe_int(value.get("width", value.get("Width", inherited.get("width", 0)))),
            "height": _safe_int(value.get("height", value.get("Height", inherited.get("height", 0)))),
            "bitrate": _safe_int(value.get("bitrate", value.get("bitRate", value.get("Bitrate", inherited.get("bitrate", 0))))),
        }
        for key, item in value.items():
            if not _SKIP_VIDEO_KEYS.search(str(key)):
                collect_video(item, current)

    def find_matching_items(value: Any) -> None:
        if budget[0] >= _MAX_JSON_NODES or not isinstance(value, (dict, list)):
            return
        identity = id(value)
        if identity in visited:
            return
        visited.add(identity)
        budget[0] += 1
        if isinstance(value, list):
            for item in value:
                find_matching_items(item)
            return
        item_id = next(
            (str(value[key]) for key in value if str(key).lower() in _VIDEO_ID_KEYS and value[key] is not None),
            None,
        )
        if item_id == video_id and isinstance(value.get("video"), (dict, list)):
            collect_video(value["video"])
        for child in value.values():
            find_matching_items(child)

    for script in scripts:
        try:
            source = script.text_content(timeout=1_000) or ""
            if not source or len(source) > _MAX_EMBEDDED_SCRIPT_CHARS:
                continue
            find_matching_items(json.loads(source))
        except Exception:
            continue
        if len(found) >= 80 or budget[0] >= _MAX_JSON_NODES:
            break
    return rank_candidates(found)


def download_video_candidate(
    candidate: VideoCandidate,
    cookies: list[dict[str, Any]],
    episode: int,
    output_dir: Path,
    on_progress: Callable[[int, int | None], None],
) -> Path:
    """Stream a captured browser media URL to disk, validate it, then atomically publish it."""
    try:
        from curl_cffi import requests as curl_requests
    except ImportError as error:
        raise RuntimeError("Tải video trực tiếp chưa sẵn sàng. Hãy cập nhật hoặc cài lại ứng dụng.") from error

    if not normalize_video_candidate(candidate.url, source=candidate.source):
        raise RuntimeError("Nguồn video không hợp lệ.")
    for extension in ("mp4", "webm"):
        existing = output_dir / f"EP{episode:04d}.{extension}"
        if existing.is_file() and _valid_video_file(existing, extension, 0):
            on_progress(existing.stat().st_size, existing.stat().st_size)
            return existing

    # A stable, opaque filename permits Range continuation after a crash.
    # Neither cookies nor signed CDN URLs are written into resume metadata.
    identity = uuid.uuid5(uuid.NAMESPACE_URL, candidate.url.split("?", 1)[0]).hex
    partial = output_dir / f".EP{episode:04d}.{identity}.partial"
    metadata_path = partial.with_suffix(".partial.json")
    session = curl_requests.Session(impersonate="chrome")
    _add_browser_cookies(session, cookies)
    last_report = 0.0
    downloaded = 0
    expected = candidate.bytes or None
    output_dir.mkdir(parents=True, exist_ok=True)
    if partial.is_symlink() or metadata_path.is_symlink():
        session.close()
        raise RuntimeError("Tệp tải tạm không an toàn.")
    try:
        for attempt in range(3):
            resume = {}
            try:
                if metadata_path.stat().st_size <= 4096:
                    resume = json.loads(metadata_path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                pass
            etag = str(resume.get("etag", ""))
            offset = partial.stat().st_size if partial.is_file() and etag.startswith('"') and etag.endswith('"') and len(etag) < 512 and not any(c in etag for c in '\r\n') else 0
            headers = {"Referer": "https://www.tiktok.com/", "Accept": "video/mp4,video/webm,application/octet-stream;q=0.9,*/*;q=0.8", "Accept-Encoding": "identity"}
            if offset:
                headers.update({"Range": f"bytes={offset}-", "If-Range": etag})
            response = None
            retry = False
            try:
                response = _get_safe_media(session, candidate.url, headers)
                if not normalize_video_candidate(response.url, source=candidate.source):
                    raise RuntimeError("Máy chủ chuyển hướng tới một địa chỉ video không được hỗ trợ.")
                if response.status_code not in (200, 206):
                    if response.status_code in (401, 403):
                        raise RuntimeError("TikTok/CDN từ chối nguồn video. Hãy kết nối lại phiên TikTok rồi thử lại.")
                    if response.status_code == 429:
                        raise RuntimeError("TikTok/CDN đang giới hạn lượt tải. Hãy chờ một lúc rồi thử lại.")
                    retry = response.status_code >= 500
                    raise RuntimeError(f"Máy chủ video trả HTTP {response.status_code}.")
                mime = str(response.headers.get("content-type", "")).split(";", 1)[0].strip().lower()
                if mime and mime not in _VIDEO_MIMES:
                    raise RuntimeError("Máy chủ trả về nội dung không phải video.")
                if re.search(r"(?:subtitle|caption|timedtext|texttrack|closedcaption)|\.(?:m3u8|mpd|m4s|ts|cmf[av])(?:\?|$)", response.url, re.IGNORECASE):
                    raise RuntimeError("TikTok chỉ cung cấp phụ đề hoặc luồng video phân mảnh cho tập này.")
                if response.status_code == 206:
                    match = re.fullmatch(r"bytes\s+(\d+)-(\d+)/(\d+)", str(response.headers.get("content-range", "")), re.IGNORECASE)
                    if not match or int(match[1]) != offset or int(match[2]) + 1 != int(match[3]):
                        raise RuntimeError("Máy chủ trả phạm vi video không hợp lệ; chưa lưu tập này.")
                    if offset and str(response.headers.get("etag", "")) != etag:
                        raise RuntimeError("Nguồn video đã thay đổi; hãy tải lại tập này.")
                    expected = int(match[3])
                else:
                    # If-Range mismatch or no Range support: restart safely.
                    offset = 0
                    expected = _safe_int(response.headers.get("content-length")) or candidate.bytes or None
                current_etag = str(response.headers.get("etag", ""))
                if len(current_etag) > 512 or any(c in current_etag for c in "\r\n"):
                    current_etag = ""
                resume_bytes = json.dumps({"etag": current_etag, "expected": expected}, ensure_ascii=True).encode("utf-8")
                with metadata_path.open("wb") as meta:
                    meta.write(resume_bytes); meta.flush(); os.fsync(meta.fileno())
                downloaded = offset
                with partial.open("ab" if offset else "wb") as target:
                    try:
                        for chunk in response.iter_content(chunk_size=1024 * 1024):
                            if not chunk: continue
                            target.write(chunk); downloaded += len(chunk)
                            now = time.monotonic()
                            if now - last_report >= 0.25:
                                on_progress(downloaded, expected); last_report = now
                    except Exception:
                        retry = True
                        raise
                    finally:
                        target.flush(); os.fsync(target.fileno())
                if expected and downloaded != expected:
                    retry = True
                    raise RuntimeError(f"Video tải chưa đủ dữ liệu ({downloaded}/{expected} byte).")
                break
            except Exception as error:
                # Transport errors can reuse a strong ETag. Invalid content,
                # authentication and unsafe redirects must not be retried.
                if response is None: retry = not isinstance(error, RuntimeError)
                if not retry or attempt == 2:
                    if isinstance(error, RuntimeError): raise
                    raise RuntimeError("Không tải xong video TikTok. Hãy kiểm tra mạng và thử lại.") from error
                time.sleep(0.5 * (attempt + 1))
            finally:
                if response is not None: response.close()
        extension = _sniff_video_extension(partial)
        if not extension or not _valid_video_file(partial, extension, expected or 0):
            raise RuntimeError("Video tải về thiếu dữ liệu phát hoặc không có luồng video hợp lệ; chưa lưu vào danh sách tập.")
        final_path = output_dir / f"EP{episode:04d}.{extension}"
        if final_path.is_symlink(): raise RuntimeError("Tệp đích không an toàn.")
        os.replace(partial, final_path)
        on_progress(downloaded, expected or downloaded)
        return final_path
    finally:
        session.close()
        for temporary in (partial, metadata_path):
            try: temporary.unlink(missing_ok=True)
            except OSError: log.warning("Could not remove incomplete TikTok download")


def _get_safe_media(session: Any, url: str, headers: dict[str, str]) -> Any:
    """Validate every redirect before connecting; never send cookies to arbitrary hosts."""
    for _ in range(6):
        if not normalize_video_candidate(url):
            raise RuntimeError("Máy chủ chuyển hướng tới một địa chỉ video không được hỗ trợ.")
        response = session.get(url, stream=True, timeout=(20, 90), allow_redirects=False, headers=headers)
        if response.status_code not in (301, 302, 303, 307, 308):
            return response
        location = str(response.headers.get("location", ""))
        response.close()
        if not location or any(c in location for c in "\r\n"):
            raise RuntimeError("Địa chỉ chuyển hướng video không hợp lệ.")
        url = urljoin(url, location)
    raise RuntimeError("Máy chủ chuyển hướng quá nhiều lần; hãy thử lại nguồn video.")


def _add_browser_cookies(session: Any, cookies: list[dict[str, Any]]) -> None:
    for cookie in cookies:
        name = str(cookie.get("name", ""))
        value = str(cookie.get("value", ""))
        domain = str(cookie.get("domain", ""))
        path = str(cookie.get("path", "/")) or "/"
        if not name or not domain or any(char in name + value + domain + path for char in "\r\n"):
            continue
        try:
            session.cookies.set(name, value, domain=domain, path=path)
        except Exception:
            continue


def _response_total_bytes(headers: dict[str, str]) -> int:
    content_range = str(headers.get("content-range", ""))
    match = re.search(r"/(\d+)\s*$", content_range)
    return _safe_int(match.group(1) if match else headers.get("content-length"))


def _safe_int(value: object) -> int:
    try:
        return int(float(value))
    except (TypeError, ValueError, OverflowError):
        return 0


def _sniff_video_extension(path: Path) -> str | None:
    try:
        with path.open("rb") as source:
            lead = source.read(32)
            if len(lead) >= 8 and lead[4:8] == b"ftyp":
                return "mp4"
            if lead.startswith(bytes.fromhex("1a45dfa3")):
                return "webm"
            lowercase = lead[:16].lstrip().lower()
            if lowercase.startswith((b"webvtt", b"<!doctype", b"<html", b"#extm3u")) or lead[4:8] == b"moof":
                return None
    except OSError:
        return None
    return None


def _probe_playable(path: Path) -> bool:
    executable = os.environ.get("JC_FFPROBE_PATH")
    if not executable:
        # The desktop always supplies its bundled FFprobe. Standalone header
        # inspection remains usable without depending on a system executable.
        return True
    try:
        result = subprocess.run([executable, "-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height", "-of", "json", str(path)],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=30,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        if result.returncode or len(result.stdout) > 1024 * 1024: return False
        metadata = json.loads(result.stdout)
        duration = float(metadata.get("format", {}).get("duration", 0))
        return math.isfinite(duration) and duration > 0 and any(stream.get("codec_type") == "video" and stream.get("width", 0) > 0 and stream.get("height", 0) > 0 for stream in metadata.get("streams", []))
    except (OSError, ValueError, TypeError, subprocess.TimeoutExpired):
        return False


def _valid_video_file(path: Path, extension: str, expected_bytes: int) -> bool:
    try:
        if path.is_symlink(): return False
        size = path.stat().st_size
        if size < 128 * 1024 or (expected_bytes > 0 and size < expected_bytes):
            return False
        if extension == "webm":
            with path.open("rb") as source:
                header = source.read(min(size, 8 * 1024 * 1024))
            return (
                header.startswith(bytes.fromhex("1a45dfa3"))
                and bytes.fromhex("18538067") in header
                and bytes.fromhex("1f43b675") in header
                and _probe_playable(path)
            )

        offset = 0
        box_count = 0
        has_ftyp = has_moov = has_media = False
        with path.open("rb") as source:
            while offset + 8 <= size and box_count < 10_000:
                source.seek(offset)
                header = source.read(16)
                if len(header) < 8:
                    return False
                box_size = int.from_bytes(header[:4], "big")
                box_type = header[4:8]
                header_size = 8
                if box_size == 1:
                    if len(header) < 16:
                        return False
                    box_size = int.from_bytes(header[8:16], "big")
                    header_size = 16
                elif box_size == 0:
                    box_size = size - offset
                if box_size < header_size or offset + box_size > size:
                    return False
                has_ftyp |= box_type == b"ftyp"
                has_moov |= box_type == b"moov"
                has_media |= box_type == b"mdat" and box_size - header_size >= 64 * 1024
                offset += box_size
                box_count += 1
        return has_ftyp and has_moov and has_media and _probe_playable(path)
    except OSError:
        return False
