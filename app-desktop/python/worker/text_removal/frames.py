"""Timestamped, bounded ROI decode. No image files and no pixel IPC."""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator
import math
import time as clock

import numpy as np


@dataclass
class Frame:
    index: int
    pts: float
    end: float
    rgb: np.ndarray


class FrameSource:
    def __init__(self, path: str | Path, rect: dict):
        import av
        self.container = av.open(str(path))
        try:
            self.stream = self.container.streams.video[0]
            self.origin = float((self.stream.start_time or 0) * self.stream.time_base)
            self.rect = tuple(int(rect[k]) for k in ("x", "y", "width", "height"))
            x, y, w, h = self.rect
            if min(x, y) < 0 or min(w, h) <= 0 or x + w > self.stream.width or y + h > self.stream.height:
                raise ValueError("Vùng đọc video không hợp lệ.")
            self.period = 1 / float(self.stream.average_rate or 30)
            self.decoded = 0
            self.peak_buffer_bytes = 0
            self.read_seconds = 0.0
        except BaseException:
            self.close()
            raise

    def _seek(self, time: float):
        self.container.seek(int((time + self.origin) / self.stream.time_base), stream=self.stream, backward=True)

    def _image(self, frame) -> np.ndarray:
        self.decoded += 1
        x, y, w, h = self.rect
        return frame.to_ndarray(format="rgb24")[y:y+h, x:x+w].copy()

    def at(self, time: float) -> tuple[float, np.ndarray]:
        if not math.isfinite(time) or time < 0:
            raise ValueError("Thời điểm đọc video không hợp lệ.")
        before = clock.perf_counter()
        self._seek(time)
        for f in self.container.decode(self.stream):
            if f.pts is None:
                continue
            pts = float(f.pts * self.stream.time_base) - self.origin
            # First source frame at/after the requested sample, not an invented CFR timestamp.
            if pts + 1e-7 >= time:
                result = pts, self._image(f)
                self.read_seconds += clock.perf_counter()-before
                return result
        raise RuntimeError("Không đọc được khung hình video tại thời điểm đã chọn.")

    def frames(self, start: float, end: float) -> Iterator[Frame]:
        self._seek(start)
        previous = None
        index = 0
        for f in self.container.decode(self.stream):
            if f.pts is None:
                continue
            pts = float(f.pts * self.stream.time_base) - self.origin
            if previous is not None:
                if pts <= previous.pts:
                    raise RuntimeError("Timestamp khung hình không tăng.")
                previous.end = min(pts, end)
                yield previous
                previous = None
            if pts >= end:
                break
            if pts + 1e-7 < start:
                continue
            previous = Frame(index, pts, min(end, pts + self.period), self._image(f))
            index += 1
        if previous is not None:
            yield previous

    def windows(self, start: float, end: float, *, halo: int = 10):
        # A core and its temporal halo only. Never retain the complete clip.
        _, _, w, h = self.rect
        core_size = max(5, min(24, (64 * 1024 * 1024 // (w * h * 3)) - 2 * halo))
        buffer = deque()
        iterator = iter(self.frames(start, end))
        eof = False
        core_start = 0
        while True:
            wanted = core_start + core_size + halo
            while not eof and (not buffer or buffer[-1].index < wanted - 1):
                try:
                    before = clock.perf_counter()
                    buffer.append(next(iterator))
                    self.read_seconds += clock.perf_counter()-before
                except StopIteration:
                    eof = True
            core = [f for f in buffer if core_start <= f.index < core_start + core_size]
            if not core:
                break
            self.peak_buffer_bytes = max(self.peak_buffer_bytes, sum(f.rgb.nbytes for f in buffer))
            yield core, list(buffer)
            core_start += core_size
            while buffer and buffer[0].index < core_start - halo:
                buffer.popleft()

    def close(self):
        self.container.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
