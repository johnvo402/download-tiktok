"""Sparse detection persists polygons/times; removal does not rerun OCR."""
from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Callable
import cv2
import numpy as np

from .frames import FrameSource
from .masks import same_subtitle, stabilize
from .patch_cache import atomic_json


def valid_detection(value: dict, key: str, start: float, end: float) -> bool:
    """Treat a matching key as insufficient: interrupted/edited indexes must rescan."""
    try:
        if value["version"] != 2 or value["key"] != key or value["complete"] is not True:
            return False
        if not isinstance(value["intervals"], list):
            return False
        previous_end = start
        for interval in value["intervals"]:
            a, b = interval["startSeconds"], interval["endSeconds"]
            if not np.isfinite([a, b]).all() or not start <= a < b <= end + 1e-7 or a < previous_end - 1e-7:
                return False
            if not isinstance(interval["boxes"], list) or not interval["boxes"]:
                return False
            for box in interval["boxes"]:
                points = np.asarray(box, dtype=float)
                if points.ndim != 2 or points.shape[1] != 2 or len(points) < 4 or not np.isfinite(points).all():
                    return False
            previous_end = b
        return True
    except (ValueError, KeyError, TypeError, IndexError):
        return False


def detect(request: dict, emit: Callable, *, engine_factory=None) -> dict:
    path = Path(request["detectionPath"])
    key = request["detectionKey"]
    start, end = float(request["startSeconds"]), float(request["endSeconds"])
    fps = int(request.get("scanFps", 2))
    if fps not in (1, 2) or not np.isfinite([start, end]).all() or not 0 <= start < end:
        raise ValueError("Khoảng thời gian hoặc tốc độ quét chữ không hợp lệ.")
    if path.is_file() and not path.is_symlink():
        try:
            previous = json.loads(path.read_text(encoding="utf-8"))
            if valid_detection(previous, key, start, end):
                return previous
        except (OSError, ValueError, KeyError):
            pass
    from worker.ocr.service import _create_engine, _resolved_device, verify_engine_device
    factory = engine_factory or _create_engine
    engine = factory(str(request["cacheDir"]), request.get("ocrModel", "tiny"), request.get("ocrDevice", "auto"))
    requested, _ = _resolved_device(request.get("ocrDevice", "auto")) if engine_factory is None else ("cpu", None)
    started = time.perf_counter()
    samples, calls = [], 0
    with FrameSource(request["sourcePath"], request["scanRect"]) as source:
        def sample(ts):
            nonlocal calls
            pts, rgb = source.at(ts)
            result = engine(cv2.cvtColor(rgb, cv2.COLOR_RGB2BGR))
            if calls == 0 and engine_factory is None:
                verify_engine_device(engine, requested, "text removal detection v2")
            calls += 1
            boxes = getattr(result, "boxes", None)
            if boxes is None and isinstance(result, (tuple, list)) and result:
                boxes = result[0]
            polygons = []
            for box in boxes if boxes is not None else []:
                p = np.asarray(box, np.float32).reshape(-1, 2)
                if len(p) >= 4 and np.isfinite(p).all():
                    polygons.append(p.tolist())
            texts = getattr(result, "txts", None)
            scores = getattr(result, "scores", None)
            confidence = float(np.mean(scores)) if scores is not None and len(scores) else None
            text = " ".join(str(t) for t in texts) if texts is not None else ""
            return {"pts": min(pts, end), "boxes": polygons, "text": text, "confidence": confidence}
        count = max(1, int(np.ceil((end-start)*fps)))
        for i in range(count):
            s = sample(start + i / fps)
            if samples and bool(samples[-1]["boxes"]) != bool(s["boxes"]):
                # Refine only transitions, to about 1/16 sampling period.
                lo, hi = samples[-1]["pts"], s["pts"]
                before = bool(samples[-1]["boxes"])
                for _ in range(4):
                    if hi-lo < source.period:
                        break
                    mid = sample((lo+hi)/2)
                    samples.append(mid)
                    if bool(mid["boxes"]) == before:
                        lo = mid["pts"]
                    else:
                        hi = mid["pts"]
            samples.append(s)
            emit({"type": "progress", "stage": "detect", "progress": (i+1)/count,
                  "message": f"Đang nhận diện phụ đề · {i+1}/{count} mẫu · {fps} khung/giây"})
        samples.sort(key=lambda s: s["pts"])
        intervals = []
        for i, s in enumerate(samples):
            if not s["boxes"]:
                continue
            until = samples[i+1]["pts"] if i+1 < len(samples) else end
            a, b = max(start, s["pts"]-0.08), min(end, until+0.08)
            if b <= a:
                continue
            if intervals and a <= intervals[-1]["endSeconds"] + 0.12 and same_subtitle(intervals[-1], s):
                intervals[-1]["endSeconds"] = b
                intervals[-1]["boxes"] = stabilize(intervals[-1]["boxes"], s["boxes"])
            else:
                if intervals:
                    intervals[-1]["endSeconds"] = min(intervals[-1]["endSeconds"], a)
                intervals.append({"startSeconds": a, "endSeconds": b, "boxes": s["boxes"], "text": s["text"], "confidence": s["confidence"]})
        result = {"version": 2, "key": key, "complete": True, "intervals": intervals,
                  "sampledFrames": count, "detectedFrames": sum(bool(s["boxes"]) for s in samples),
                  "metrics": {"ocrCalls": calls, "decodedRoiFrames": source.decoded, "ocrSeconds": time.perf_counter()-started,
                              "ocrBackend": requested, "temporaryImageFiles": 0}}
    path.parent.mkdir(parents=True, exist_ok=True)
    atomic_json(path, result)
    return result
