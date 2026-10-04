"""Sparse detection -> bounded frame/mask windows -> checkpointed clean patches."""
from __future__ import annotations

import json
import hashlib
import logging
from pathlib import Path
import time
import cv2
import numpy as np

from .frames import FrameSource
from .masks import MaskProfile, make_mask, plan_windows, scene_ids, context_masks
from .patch_cache import PatchWriter, fingerprint, load
from .detection import valid_detection

LOGGER = logging.getLogger(__name__)


def process(request: dict, emit, *, backend_factory=None, cancel=lambda: False) -> dict:
    detection_bytes = Path(request["detectionPath"]).read_bytes()
    detection = json.loads(detection_bytes.decode('utf-8'))
    if not valid_detection(detection, request["detectionKey"], 0, float(request["videoDuration"])):
        raise RuntimeError("Kết quả nhận diện đã thay đổi hoặc bị hỏng. Hãy nhận diện lại phụ đề.")
    intervals = detection["intervals"]
    starts = [i['startSeconds'] for i in intervals]
    context_key = hashlib.sha256(detection_bytes).hexdigest()
    results, backend = [], None
    rect = request["processingRect"]
    width, height = int(rect["width"]), int(rect["height"])
    local = request["ocrRect"]
    profile = MaskProfile(**request.get("maskProfile", {}))
    mode = request.get("maskMode", "ocrGuided")
    if mode not in {"ocrGuided", "fullRegion"}:
        raise ValueError("Chế độ phục hồi không hợp lệ.")
    output_dir = Path(request["outputDir"])
    output_dir.mkdir(parents=True, exist_ok=True)
    # The Rust project job lock guarantees no concurrent writer in this directory.
    # Remove only uncommitted fragments left by a terminated worker.
    for orphan in output_dir.glob('*.partial'):
        if orphan.is_file() and not orphan.is_symlink():
            orphan.unlink(missing_ok=True)
    started = time.perf_counter()
    stats = {"temporaryImageFiles": 0, "decodedRoiFrames": 0, "activeFrames": 0,
             "inpaintSeconds": 0.0, "writeSeconds": 0.0, "readSeconds": 0.0,
             "maskSeconds": 0.0, "peakFrameBufferBytes": 0, "reusedSpans": 0}
    try:
        for index, interval in enumerate(intervals):
            if cancel():
                raise InterruptedError("Đã hủy phục hồi phụ đề.")
            key = fingerprint({"engine": "sttn-v2", "model": request["modelHash"], "source": request["sourceKey"],
                               "detection": request["detectionKey"], "context": context_key, "interval": interval, "rect": rect,
                               "maskMode": mode, "maskProfile": request.get("maskProfile", {}),
                               "resolution": [432, 240], "radius": 5, "stride": 5, "references": 8})
            path = output_dir / f"{key}.pack"
            cached = load(path, key)
            if cached is not None:
                stats["reusedSpans"] += 1
                results.append({"path": str(path), "frames": len(cached["frames"]), "maskedPixels": cached["metrics"]["maskedPixels"]})
                emit({"type": "progress", "stage": "cached", "progress": (index+1)/max(1, len(intervals)),
                      "message": f"Đã dùng lại đoạn phục hồi {index+1}/{len(intervals)}"})
                continue
            start, end = float(interval["startSeconds"]), float(interval["endSeconds"])
            if not np.isfinite([start, end]).all() or not 0 <= start < end:
                raise ValueError("Timestamp nhận diện không hợp lệ.")
            boxes = [(np.asarray(b, np.float32) + np.array([local["x"], local["y"]])).tolist() for b in interval["boxes"]]
            before = time.perf_counter()
            binary, alpha = make_mask(width, height, boxes, int(request["videoHeight"]), profile,
                                      full_region=local if mode == "fullRegion" else None)
            stats["maskSeconds"] += time.perf_counter()-before
            masked_pixels = int(np.count_nonzero(binary))
            if not masked_pixels:
                continue
            if backend is None:
                emit({"type": "progress", "stage": "loading", "progress": index/max(1, len(intervals)), "message": "Đang nạp STTN…"})
                if backend_factory is None:
                    from .inpainting import SttnInpainter
                    backend = SttnInpainter(Path(request["modelPath"]), request.get("inpaintDevice", "auto"))
                else:
                    backend = backend_factory()
                backend.prepare()
            writer = PatchWriter(path, width, height, key)
            active = 0
            try:
                with FrameSource(request["sourcePath"], rect) as source:
                    context_start = max(0, start - 10 * source.period)
                    context_end = min(float(request["videoDuration"]), end + 10 * source.period)
                    for core, context in source.windows(context_start, context_end):
                        if cancel():
                            raise InterruptedError("Đã hủy phục hồi phụ đề.")
                        before = time.perf_counter()
                        masks = context_masks(context, intervals, starts, index, binary, width, height,
                                              local, int(request['videoHeight']), profile, mode)
                        stats['maskSeconds'] += time.perf_counter()-before
                        core_set = {f.index for f in core if start <= f.pts < end}
                        affected = [i for i, f in enumerate(context) if f.index in core_set]
                        if not affected:
                            continue
                        sums, counts = {}, {}
                        before = time.perf_counter()
                        for neighbors, refs in plan_windows(len(context), affected, scene_ids(context), masks, indices=[f.index for f in context]):
                            if cancel():
                                raise InterruptedError("Đã hủy phục hồi phụ đề.")
                            predictions = backend.process(context, masks, neighbors, refs)
                            for k, i in enumerate(neighbors):
                                if i not in affected:
                                    continue
                                if i not in sums:
                                    sums[i] = predictions[k].astype(np.uint16)
                                    counts[i] = 1
                                else:
                                    sums[i] += predictions[k].astype(np.uint16)
                                    counts[i] += 1
                        stats["inpaintSeconds"] += time.perf_counter()-before
                        before = time.perf_counter()
                        for i in affected:
                            if i not in sums:
                                raise RuntimeError("STTN chưa phục hồi đủ frame đang có phụ đề.")
                            f = context[i]
                            prediction = cv2.resize(np.rint(sums[i]/counts[i]).astype(np.uint8), (width, height), interpolation=cv2.INTER_CUBIC)
                            rgb = f.rgb.copy()
                            rgb[binary > 0] = prediction[binary > 0]
                            writer.append(f.pts, min(f.end, end), np.dstack((rgb, alpha)))
                            active += 1
                        stats["writeSeconds"] += time.perf_counter()-before
                        emit({"type": "progress", "stage": "inpaint", "progress": (index + min(1, (core[-1].end-start)/(end-start)))/max(1, len(intervals)),
                              "message": f"Đoạn {index+1}/{len(intervals)} · đã phục hồi {active} khung có phụ đề"})
                    stats["decodedRoiFrames"] += source.decoded
                    stats["readSeconds"] += source.read_seconds
                    stats["peakFrameBufferBytes"] = max(stats["peakFrameBufferBytes"], source.peak_buffer_bytes)
                if not active:
                    raise RuntimeError("Không đọc được frame trong đoạn phụ đề.")
                writer.commit({"maskedPixels": masked_pixels, "device": str(backend.device), "activeFrames": active})
            finally:
                writer.close()
            stats["activeFrames"] += active
            results.append({"path": str(path), "frames": active, "maskedPixels": masked_pixels})
        stats["elapsedSeconds"] = time.perf_counter()-started
        stats["inpaintingBackend"] = f"STTN {backend.device}" if backend is not None else "cache"
        stats["persistentBytes"] = sum(Path(r["path"]).stat().st_size for r in results)
        if backend is not None and str(backend.device).startswith('cuda'):
            stats["peakGpuBytes"] = int(backend.torch.cuda.max_memory_allocated())
        if backend is not None:
            stats['encodedFrameCacheHits'] = getattr(backend,'feature_hits',0)
            stats['encodedFrameCacheMisses'] = getattr(backend,'feature_misses',0)
        LOGGER.info("Text removal v2: %s", json.dumps(stats))
        atomic_path = output_dir / "report.json"
        from .patch_cache import atomic_json
        atomic_json(atomic_path, stats)
        atomic_json(output_dir / "result.json", {"complete": True, "version": 2, "detectionKey": request["detectionKey"], "detectionFingerprint": context_key,
                                               "files": [Path(r["path"]).name for r in results]})
        return {"intervalResults": results, "metrics": stats, "frames": sum(r["frames"] for r in results)}
    finally:
        if backend is not None:
            backend.close()
