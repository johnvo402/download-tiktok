from __future__ import annotations

import os
from pathlib import Path
import shutil
import uuid


def materialize_onnx_files(cache_root: Path, *snapshots: Path) -> bool:
    """Keep ONNX graphs and external weights beside one another, not symlinks.

    HF's shared blob cache can resolve each file into a different directory.
    ORT validates external weights against the graph's canonical directory.
    Hard links preserve the cache bytes without duplicating large weights.
    """
    root = cache_root.resolve(strict=True)
    links: list[tuple[Path, Path]] = []
    for snapshot in snapshots:
        snapshot.resolve(strict=True).relative_to(root)
        for file_path in sorted(snapshot.rglob("*")):
            if file_path.suffix not in {".onnx", ".data"}:
                continue
            file_path.parent.resolve(strict=True).relative_to(root)
            source = file_path.resolve(strict=True)
            source.relative_to(root)
            if not source.is_file() or source.stat().st_size == 0:
                raise ValueError(f"Model artifact is empty or not a file: {file_path.name}")
            if file_path.is_symlink():
                links.append((file_path, source))

    for file_path, source in links:
        temporary = file_path.with_name(f".{file_path.name}.{uuid.uuid4().hex}.tmp")
        try:
            try:
                os.link(source, temporary)
            except OSError:
                shutil.copy2(source, temporary)
            temporary.replace(file_path)
        finally:
            temporary.unlink(missing_ok=True)
    return bool(links)
