from __future__ import annotations

import hashlib
import json
import os
import uuid
from pathlib import Path, PurePosixPath
from typing import Iterable


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _inside(root: Path, path: Path) -> bool:
    try:
        path.resolve(strict=True).relative_to(root.resolve(strict=True))
        return True
    except (OSError, ValueError):
        return False


def _manifest_digest(entries: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for item in entries:
        digest.update(str(item["path"]).encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(item["size"]).encode("ascii"))
        digest.update(b"\0")
        digest.update(str(item["sha256"]).encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def create_manifest(
    root: Path,
    manifest_path: Path,
    identity: str,
    files: Iterable[Path],
    *,
    model_id: str,
    version: str,
    download: str,
) -> None:
    root = root.resolve(strict=True)
    entries: list[dict[str, object]] = []
    # Canonicalize both roots and files before deriving relative paths. On
    # Windows, absolute() can preserve a short-name/junction path while
    # root.resolve() returns its canonical form, making safe in-root files
    # appear outside the root on CI runners.
    normalized = {file_path.resolve(strict=True) for file_path in files}
    for file_path in sorted(normalized, key=lambda item: item.as_posix()):
        if not _inside(root, file_path) or not file_path.is_file():
            raise ValueError("Model file resolves outside its managed cache or is not a file")
        stat = file_path.stat()
        if stat.st_size <= 0:
            raise ValueError(f"Model file is empty: {file_path.name}")
        relative = file_path.relative_to(root).as_posix()
        entries.append({
            "path": relative,
            "size": stat.st_size,
            "mtimeNs": stat.st_mtime_ns,
            "sha256": _sha256(file_path),
        })
    if not entries:
        raise ValueError("Model integrity manifest cannot be empty")

    payload = {
        "schemaVersion": 1,
        "identity": identity,
        "id": model_id,
        "version": version,
        "download": download,
        "files": entries,
        "size": sum(int(item["size"]) for item in entries),
        "sha256": _manifest_digest(entries),
    }
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = manifest_path.with_name(f".{manifest_path.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temporary.open("x", encoding="utf-8", newline="\n") as output:
            json.dump(payload, output, ensure_ascii=False, sort_keys=True, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        temporary.replace(manifest_path)
    finally:
        temporary.unlink(missing_ok=True)


def verify_manifest(root: Path, manifest_path: Path, identity: str) -> bool:
    try:
        root = root.resolve(strict=True)
        if manifest_path.is_symlink() or not manifest_path.is_file() or not _inside(root, manifest_path):
            return False
        payload = json.loads(manifest_path.read_text(encoding="utf-8"))
        entries = payload.get("files")
        if payload.get("schemaVersion") != 1 or payload.get("identity") != identity:
            return False
        if not isinstance(entries, list) or not entries:
            return False
        seen: set[str] = set()
        total_size = 0
        for item in entries:
            if not isinstance(item, dict):
                return False
            relative = item.get("path")
            if not isinstance(relative, str):
                return False
            parsed = PurePosixPath(relative)
            if parsed.is_absolute() or ".." in parsed.parts or "\\" in relative or relative in seen:
                return False
            seen.add(relative)
            path = root.joinpath(*parsed.parts)
            if not _inside(root, path) or not path.is_file():
                return False
            stat = path.stat()
            if stat.st_size != item.get("size") or stat.st_size <= 0:
                return False
            total_size += stat.st_size
            digest = item.get("sha256")
            if not isinstance(digest, str) or len(digest) != 64:
                return False
            # Hash during install. On later checks, size + mtime are a cheap
            # integrity fast path; re-hash whenever metadata changed.
            if stat.st_mtime_ns != item.get("mtimeNs") and _sha256(path) != digest:
                return False
        return (
            isinstance(payload.get("id"), str)
            and isinstance(payload.get("version"), str)
            and isinstance(payload.get("download"), str)
            and payload.get("size") == total_size
            and payload.get("sha256") == _manifest_digest(entries)
        )
    except (OSError, ValueError, TypeError, json.JSONDecodeError):
        return False
