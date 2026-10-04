"""Atomic compressed RGBA patch spans with source PTS. No frame PNG files."""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import tempfile
import zlib

import numpy as np

VERSION = 2
MAGIC = b"JCPATCH2"


def atomic_json(path: Path, value: dict):
    temp = path.with_suffix(path.suffix + ".tmp")
    with temp.open("w", encoding="utf-8") as out:
        json.dump(value, out, ensure_ascii=False, allow_nan=False)
        out.flush()
        os.fsync(out.fileno())
    os.replace(temp, path)


def fingerprint(value: dict) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


class PatchWriter:
    def __init__(self, path: Path, width: int, height: int, key: str):
        self.path, self.width, self.height, self.key = path, width, height, key
        self.file = tempfile.NamedTemporaryFile(dir=path.parent, suffix=".partial", delete=False)
        self.file.write(MAGIC)
        self.frames = []
        self.closed = False

    def append(self, pts: float, end: float, rgba: np.ndarray):
        if rgba.shape != (self.height, self.width, 4) or rgba.dtype != np.uint8:
            raise ValueError("Patch pixels không đúng kích thước/định dạng.")
        if not math.isfinite(pts) or not math.isfinite(end) or pts < 0 or end <= pts or (self.frames and pts < self.frames[-1]["end"] - 1e-7):
            raise ValueError("Patch timestamp không hợp lệ.")
        data = zlib.compress(rgba.tobytes(), level=1)
        offset = self.file.tell()
        self.file.write(data)
        self.frames.append({"pts": pts, "end": end, "offset": offset, "length": len(data)})

    def commit(self, metrics: dict):
        if not self.frames:
            raise ValueError("Không thể lưu đoạn phục hồi không có khung hình.")
        self.file.flush()
        os.fsync(self.file.fileno())
        self.file.close()
        # Invalidate the old completion marker before swapping the data file.
        self.path.with_suffix(".json").unlink(missing_ok=True)
        self.path.with_suffix(".nut").unlink(missing_ok=True)
        os.replace(self.file.name, self.path)
        metadata = {"version": VERSION, "key": self.key, "width": self.width, "height": self.height,
                    "bytes": self.path.stat().st_size, "frames": self.frames, "metrics": metrics, "complete": True}
        atomic_json(self.path.with_suffix(".json"), metadata)
        self.closed = True
        return metadata

    def close(self):
        if not self.closed:
            self.file.close()
            Path(self.file.name).unlink(missing_ok=True)


def load(path: Path, key: str | None = None) -> dict | None:
    try:
        m = json.loads(path.with_suffix(".json").read_text(encoding="utf-8"))
        if m["version"] != VERSION or not m["complete"] or (key and key != m["key"]) or path.stat().st_size != m["bytes"]:
            return None
        if path.is_symlink() or path.with_suffix(".json").is_symlink():
            return None
        previous = 8
        last_end = -1.0
        expected = int(m["width"]) * int(m["height"]) * 4
        if min(int(m["width"]), int(m["height"])) <= 0 or not 0 < expected <= 256 * 1024 * 1024 or not m["frames"]:
            return None
        with path.open("rb") as data:
            if data.read(8) != MAGIC:
                return None
            for f in m["frames"]:
                if (not math.isfinite(f["pts"]) or not math.isfinite(f["end"]) or f["pts"] < 0
                    or f["offset"] != previous or not 0 < f["length"] <= expected+65536
                    or f["end"] <= f["pts"] or f["pts"] < last_end - 1e-7):
                    return None
                previous = f["offset"] + f["length"]
                if previous > m["bytes"]:
                    return None
                data.seek(f["offset"])
                decoder = zlib.decompressobj()
                raw = decoder.decompress(data.read(f["length"]), expected+1)
                if len(raw) != expected or not decoder.eof or decoder.unused_data:
                    return None
                last_end = f["end"]
        return m if previous == m["bytes"] else None
    except (OSError, ValueError, KeyError, TypeError, zlib.error):
        return None


def patch_video(path: Path) -> Path:
    """Compatibility compositor only: lossless alpha NUT, preserving each source PTS."""
    import av
    from fractions import Fraction
    metadata = load(path)
    if metadata is None or not metadata["frames"]:
        raise RuntimeError("Cache phục hồi bị thiếu hoặc hỏng.")
    dest = path.with_suffix(".nut")
    if dest.is_file():
        try:
            with av.open(str(dest)) as existing:
                s = existing.streams.video[0]
                if s.width == metadata["width"] and s.height == metadata["height"] and s.codec_context.name == "ffv1":
                    for _ in existing.decode(s):
                        pass
                    return dest
        except (OSError, ValueError, IndexError):
            pass
    temp = dest.with_suffix(".partial.nut")
    origin = metadata["frames"][0]["pts"]
    try:
        with av.open(str(temp), "w", format="nut") as container, path.open("rb") as data:
            stream = container.add_stream("ffv1", rate=30)
            stream.width, stream.height = metadata["width"], metadata["height"]
            stream.pix_fmt = "bgra"
            stream.time_base = Fraction(1, 1_000_000)
            stream.codec_context.time_base = stream.time_base
            for f in metadata["frames"]:
                data.seek(f["offset"])
                rgb = np.frombuffer(zlib.decompress(data.read(f["length"])), np.uint8).reshape(metadata["height"], metadata["width"], 4)
                frame = av.VideoFrame.from_ndarray(rgb, format="rgba")
                frame.pts, frame.time_base = round((f["pts"] - origin) * 1e6), stream.time_base
                for packet in stream.encode(frame):
                    packet.duration = round((f["end"] - f["pts"]) / packet.time_base)
                    container.mux(packet)
            for packet in stream.encode():
                container.mux(packet)
        os.replace(temp, dest)
    finally:
        temp.unlink(missing_ok=True)
    return dest
