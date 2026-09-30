from __future__ import annotations

import logging
import time
from pathlib import Path
from typing import Any, Callable

import cv2
import numpy as np
from PIL import Image

from worker.ocr.service import _create_engine, _resolved_device, _runtime_diagnostics, verify_engine_device
from worker.text_removal.intervals import merge_detected_intervals

LOGGER = logging.getLogger(__name__)

STTN_MODEL_BYTES = 66_252_587


def _result_boxes(result: Any) -> list[np.ndarray]:
    if result is None:
        return []
    boxes = getattr(result, "boxes", None)
    if boxes is None and isinstance(result, (tuple, list)) and result:
        boxes = result[0]
    if boxes is None:
        return []
    output: list[np.ndarray] = []
    for box in boxes:
        try:
            points = np.asarray(box, dtype=np.float32).reshape(-1, 2)
            if len(points) >= 4 and np.isfinite(points).all():
                output.append(points)
        except (TypeError, ValueError):
            continue
    return output


def scan_subtitle_intervals(request: dict[str, Any], emit: Callable[[dict[str, object]], None]) -> None:
    """Sparse RapidOCR scan used only to find when subtitle text is present.

    This scan does not create or modify transcript data. Detailed per-frame OCR
    masks are produced later, only for the intervals returned here.
    """
    frames_dir = Path(str(request["framesDir"]))
    frames = sorted(frames_dir.glob("frame_*.jpg"))
    if not frames:
        raise RuntimeError("RapidOCR không nhận được frame để quét vùng chữ.")
    fps = float(request.get("scanFps", 2.0))
    start_seconds = float(request.get("startSeconds", 0.0))
    end_seconds = float(request.get("endSeconds", start_seconds + len(frames) / max(fps, 0.01)))
    if not np.isfinite(fps) or fps <= 0:
        raise RuntimeError("Tốc độ quét OCR không hợp lệ.")

    engine = _create_engine(str(request["cacheDir"]), str(request.get("ocrModel", "tiny")),
                            str(request.get("ocrDevice", "auto")))
    requested_device, _ = _resolved_device(str(request.get("ocrDevice", "auto")))
    ocr_rect = request.get("ocrRect") or {"x": 0, "y": 0, "width": 0, "height": 0}
    ocr_x = max(0, int(ocr_rect.get("x", 0)))
    ocr_y = max(0, int(ocr_rect.get("y", 0)))
    ocr_width = max(0, int(ocr_rect.get("width", 0)))
    ocr_height = max(0, int(ocr_rect.get("height", 0)))
    detected: list[int] = []

    for index, frame_path in enumerate(frames):
        with Image.open(frame_path) as frame_file:
            image = np.array(frame_file.convert("RGB"), dtype=np.uint8, copy=True)
        height, width = image.shape[:2]
        if ocr_width > 0 and ocr_height > 0:
            x1, y1 = min(ocr_x, width - 1), min(ocr_y, height - 1)
            x2, y2 = min(width, x1 + ocr_width), min(height, y1 + ocr_height)
            ocr_image = image[y1:y2, x1:x2]
        else:
            ocr_image = image
        result = engine(cv2.cvtColor(ocr_image, cv2.COLOR_RGB2BGR))
        if index == 0:
            verify_engine_device(engine, requested_device, "sparse text-removal scan")
        if _result_boxes(result):
            detected.append(index)
        if index % max(1, len(frames) // 100) == 0 or index + 1 == len(frames):
            emit({"type": "progress", "job": "text_removal_scan", "progress": (index + 1) / len(frames),
                  "message": f"Đang quét RapidOCR · {index + 1}/{len(frames)} mẫu · {fps:g} khung/giây"})

    intervals = merge_detected_intervals(detected, fps, start_seconds, end_seconds)
    emit({"type": "completed", "job": "text_removal_scan", "intervals": intervals,
          "detectedFrames": len(detected), "sampledFrames": len(frames)})


def _make_mask(boxes: list[np.ndarray], width: int, height: int) -> tuple[np.ndarray, list[tuple[int, int, int, int]]]:
    mask = np.zeros((height, width), dtype=np.uint8)
    rects: list[tuple[int, int, int, int]] = []
    for points in boxes:
        polygon = np.rint(points).astype(np.int32)
        cv2.fillPoly(mask, [polygon], 255)
        x, y, w, h = cv2.boundingRect(polygon)
        if w > 1 and h > 1:
            rects.append((x, y, x + w, y + h))
    if not rects:
        return mask, rects
    # OCR polygons usually include glyph strokes, but leave room for antialiasing,
    # outlines, and shadows so no faint subtitle edge survives the inpaint.
    max_height = max(bottom - top for _, top, _, bottom in rects)
    radius = max(2, min(10, round(max_height * 0.14)))
    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (radius * 2 + 1, radius * 2 + 1))
    mask = cv2.dilate(mask, kernel, iterations=1)
    return mask, rects


def _mask_pixels(path: Path) -> int:
    with Image.open(path) as image:
        return int(np.count_nonzero(np.asarray(image.convert("L"))))


def _process_frames(
    request: dict[str, Any],
    emit: Callable[[dict[str, object]], None],
    engine: Any | None = None,
    runtime_cache: dict[str, Any] | None = None,
) -> dict[str, Any]:
    frames_dir = Path(str(request["framesDir"]))
    masks_dir = Path(str(request["masksDir"]))
    output_dir = Path(str(request["outputDir"]))
    cache_dir = str(request["cacheDir"])
    model_path = Path(str(request["modelPath"]))
    engine_name = str(request.get("engine", "sttn"))
    if engine_name != "sttn":
        raise RuntimeError("Chỉ hỗ trợ xử lý xóa chữ bằng STTN.")
    mask_mode = str(request.get("maskMode", "ocrGuided"))
    if mask_mode not in {"ocrGuided", "fullRegion"}:
        raise RuntimeError("Chế độ phục hồi vùng chữ không hợp lệ.")
    model = str(request.get("ocrModel", "tiny"))
    device = str(request.get("ocrDevice", "auto"))
    ocr_device, _ = _resolved_device(device)
    frames = sorted(frames_dir.glob("frame_*.jpg"))
    if not frames:
        raise RuntimeError("Không có frame trong vùng đã chọn để xóa chữ.")
    if not model_path.is_file() or model_path.stat().st_size != STTN_MODEL_BYTES:
        raise RuntimeError("Chưa cài đủ model STTN. Hãy mở Cài đặt ứng dụng → Nhận dạng lời thoại để cài model.")
    masks_dir.mkdir(parents=True, exist_ok=True)
    output_dir.mkdir(parents=True, exist_ok=True)
    if mask_mode == "ocrGuided":
        engine = engine or _create_engine(cache_dir, model, device)
    runtime_cache = runtime_cache if runtime_cache is not None else {}
    total = len(frames)
    LOGGER.info(
        "Text removal mask pass started: engine=%s, OCR model=%s, requestedDevice=%s, resolvedOCRDevice=%s, frames=%d",
        engine_name, model, device, ocr_device, total,
    )
    stage_started = time.perf_counter()
    detections: list[list[tuple[int, int, int, int]]] = []
    mask_paths: list[Path] = []
    ocr_rect = request.get("ocrRect") or {"x": 0, "y": 0, "width": 0, "height": 0}
    ocr_x = max(0, int(ocr_rect.get("x", 0)))
    ocr_y = max(0, int(ocr_rect.get("y", 0)))
    ocr_width = max(0, int(ocr_rect.get("width", 0)))
    ocr_height = max(0, int(ocr_rect.get("height", 0)))
    for index, frame_path in enumerate(frames):
        image = np.asarray(Image.open(frame_path).convert("RGB"))
        h, w = image.shape[:2]
        if ocr_width > 0 and ocr_height > 0:
            x1, y1 = min(ocr_x, w - 1), min(ocr_y, h - 1)
            x2, y2 = min(w, x1 + ocr_width), min(h, y1 + ocr_height)
            ocr_image = image[y1:y2, x1:x2]
        else:
            x1, y1, x2, y2 = 0, 0, w, h
            ocr_image = image
        if mask_mode == "fullRegion":
            mask = np.zeros((h, w), dtype=np.uint8)
            if ocr_width > 0 and ocr_height > 0:
                x1, y1 = min(ocr_x, w), min(ocr_y, h)
                x2, y2 = min(w, x1 + ocr_width), min(h, y1 + ocr_height)
            else:
                x1, y1, x2, y2 = 0, 0, w, h
            if x2 > x1 and y2 > y1:
                mask[y1:y2, x1:x2] = 255
                rects = [(x1, y1, x2, y2)]
            else:
                rects = []
        else:
            result = engine(cv2.cvtColor(ocr_image, cv2.COLOR_RGB2BGR))
            if index == 0:
                verify_engine_device(engine, ocr_device, "text-removal mask")
            boxes = [box + np.array([x1, y1], dtype=np.float32) for box in _result_boxes(result)]
            mask, rects = _make_mask(boxes, w, h)
        mask_path = masks_dir / f"mask_{index + 1:08d}.png"
        Image.fromarray(mask).save(mask_path, optimize=False)
        detections.append(rects)
        mask_paths.append(mask_path)
        if index % max(1, total // 100) == 0 or index + 1 == total:
            emit({"type": "progress", "job": "text_removal", "stage": "detect", "progress": 0.35 * (index + 1) / total,
                  "message": f"{'Đang tạo mask toàn vùng' if mask_mode == 'fullRegion' else 'Đang tạo mask theo nét chữ'} · {index + 1}/{total} khung"})

    completed_mask_pixels = sum(_mask_pixels(path) for path in mask_paths)
    LOGGER.info(
        "Text removal mask pass complete: mode=%s, frames=%d, maskedPixels=%d, detectedFrames=%d, elapsed=%.2fs, avg=%.1fms/frame",
        mask_mode, total, completed_mask_pixels, sum(bool(rects) for rects in detections),
        time.perf_counter() - stage_started, (time.perf_counter() - stage_started) * 1000 / max(total, 1),
    )
    if completed_mask_pixels == 0:
        # A coarse scan can include a false positive. Report it so the caller can
        # skip this span instead of failing an otherwise valid export.
        return {"frames": total, "maskedPixels": 0, "device": ocr_device}

    from worker.text_removal.sttn_engine import process_sttn_frames

    result = process_sttn_frames(frames_dir, masks_dir, output_dir, model_path, emit, runtime_cache)
    return {"frames": total, "maskedPixels": completed_mask_pixels, "device": result.get("device", "cpu")}


def process_frames(request: dict[str, Any], emit: Callable[[dict[str, object]], None]) -> None:
    result = _process_frames(request, emit)
    emit({"type": "completed", "job": "text_removal", **result})


def process_intervals(request: dict[str, Any], emit: Callable[[dict[str, object]], None]) -> None:
    """Process detected spans with shared OCR and inpainting runtimes."""
    intervals = request.get("intervals")
    if not isinstance(intervals, list) or not intervals:
        raise RuntimeError("Không có đoạn subtitle đã nhận diện để xử lý.")
    cache_dir = str(request["cacheDir"])
    model = str(request.get("ocrModel", "tiny"))
    device = str(request.get("ocrDevice", "auto"))
    mask_mode = str(request.get("maskMode", "ocrGuided"))
    engine = _create_engine(cache_dir, model, device) if mask_mode == "ocrGuided" else None
    runtime_cache: dict[str, Any] = {}
    results: list[dict[str, Any]] = []
    total_intervals = len(intervals)

    for index, interval in enumerate(intervals):
        if not isinstance(interval, dict):
            raise RuntimeError("Danh sách đoạn subtitle không hợp lệ.")
        def emit_interval(event: dict[str, object]) -> None:
            if event.get("type") == "progress":
                local = float(event.get("progress", 0.0))
                emit({**event, "progress": (index + max(0.0, min(1.0, local))) / total_intervals,
                      "message": f"Đoạn {index + 1}/{total_intervals} · {event.get('message', 'Đang xóa chữ') }"})

        interval_request = {**request, **interval}
        result = _process_frames(interval_request, emit_interval, engine, runtime_cache)
        results.append(result)

    total_masked_pixels = sum(int(result.get("maskedPixels", 0)) for result in results)
    emit({"type": "completed", "job": "text_removal", "frames": sum(int(r.get("frames", 0)) for r in results),
          "maskedPixels": total_masked_pixels,
          "device": next((str(result.get("device")) for result in results if result.get("maskedPixels", 0)), "cpu"),
          "intervalResults": results})
