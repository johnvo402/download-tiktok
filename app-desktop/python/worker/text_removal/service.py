from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Callable

import cv2
import numpy as np
from PIL import Image

from worker.ocr.service import _create_engine

MODEL_BYTES = 208_044_816


def inspect(cache_dir: str) -> dict[str, Any]:
    model = Path(cache_dir) / "text-removal" / "lama_fp32.onnx"
    installed = model.is_file() and model.stat().st_size == MODEL_BYTES
    try:
        import rapidocr  # noqa: F401
        ocr_installed = True
    except ImportError:
        ocr_installed = False
    try:
        import onnxruntime as ort
        runtime_installed = True
        cuda_available = "CUDAExecutionProvider" in ort.get_available_providers()
    except (ImportError, OSError, RuntimeError):
        runtime_installed = False
        cuda_available = False
    return {
        "modelInstalled": installed,
        "modelBytes": MODEL_BYTES if installed else 0,
        "estimatedDownloadBytes": MODEL_BYTES,
        "ocrInstalled": ocr_installed,
        "runtimeInstalled": runtime_installed,
        "cudaAvailable": cuda_available,
    }


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


def _line_groups(rects: list[tuple[int, int, int, int]]) -> list[tuple[int, int, int, int]]:
    lines: list[list[tuple[int, int, int, int]]] = []
    for rect in sorted(rects, key=lambda item: ((item[1] + item[3]) / 2, item[0])):
        center = (rect[1] + rect[3]) / 2
        target = next((line for line in reversed(lines)
                       if abs(center - sum((item[1] + item[3]) / 2 for item in line) / len(line)
                              ) <= max(12, rect[3] - rect[1])), None)
        if target is None:
            lines.append([rect])
        else:
            target.append(rect)
    return [
        (min(item[0] for item in line), min(item[1] for item in line),
         max(item[2] for item in line), max(item[3] for item in line))
        for line in lines
    ]


def _lama_session(model_path: Path):
    import onnxruntime as ort

    options = ort.SessionOptions()
    options.intra_op_num_threads = max(1, min(8, os.cpu_count() or 4))
    options.inter_op_num_threads = 1
    options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    providers = ort.get_available_providers()
    selected = (["CUDAExecutionProvider", "CPUExecutionProvider"]
                if "CUDAExecutionProvider" in providers else ["CPUExecutionProvider"])
    session = ort.InferenceSession(str(model_path), sess_options=options, providers=selected)
    inputs = {item.name for item in session.get_inputs()}
    if not {"image", "mask"}.issubset(inputs):
        raise RuntimeError("Model LaMa không đúng định dạng đầu vào dự kiến.")
    return session


def _infer_tiles(session: Any, frame: np.ndarray, mask: np.ndarray,
                 rects: list[tuple[int, int, int, int]]) -> np.ndarray:
    if not rects or not np.any(mask):
        return frame.copy()
    height, width = mask.shape
    pad_x = max(0, 512 - width)
    pad_y = max(0, 512 - height)
    left = pad_x // 2
    top = pad_y // 2
    padded_frame = cv2.copyMakeBorder(frame, top, pad_y - top, left, pad_x - left, cv2.BORDER_REFLECT_101)
    padded_mask = cv2.copyMakeBorder(mask, top, pad_y - top, left, pad_x - left, cv2.BORDER_CONSTANT, value=0)
    canvas_h, canvas_w = padded_mask.shape
    result = padded_frame.copy()
    touched = np.zeros_like(padded_mask)
    for x1, y1, x2, y2 in _line_groups(rects):
        center_y = (y1 + y2) // 2 + top
        y0 = max(0, min(canvas_h - 512, center_y - 256))
        start_x = max(0, x1 + left - 48)
        end_x = min(canvas_w, x2 + left + 48)
        starts = [max(0, min(canvas_w - 512, start_x))]
        while starts[-1] + 512 < end_x:
            next_x = min(canvas_w - 512, starts[-1] + 384)
            if next_x == starts[-1]:
                break
            starts.append(next_x)
        for x0 in starts:
            tile_mask = padded_mask[y0:y0 + 512, x0:x0 + 512]
            if not np.any(tile_mask):
                continue
            tile_image = padded_frame[y0:y0 + 512, x0:x0 + 512]
            image_input = (tile_image.astype(np.float32) / 255.0).transpose(2, 0, 1)[None, ...]
            mask_input = (tile_mask.astype(np.float32) / 255.0)[None, None, ...]
            filled = session.run(["output"], {"image": image_input, "mask": mask_input})[0][0]
            if filled.shape == (3, 512, 512):
                filled = filled.transpose(1, 2, 0)
            filled = np.clip(filled, 0, 255).astype(np.uint8)
            target = result[y0:y0 + 512, x0:x0 + 512]
            local_mask = tile_mask > 0
            new_pixels = local_mask & (touched[y0:y0 + 512, x0:x0 + 512] == 0)
            target[new_pixels] = filled[new_pixels]
            touched[y0:y0 + 512, x0:x0 + 512][local_mask] = 255
    return result[top:top + height, left:left + width]


def _temporal_fill(current: np.ndarray, current_mask: np.ndarray,
                   references: list[tuple[np.ndarray, np.ndarray]]) -> tuple[np.ndarray, np.ndarray]:
    if not references or not np.any(current_mask):
        return np.zeros_like(current), np.zeros_like(current_mask, dtype=bool)
    target_gray = cv2.cvtColor(current, cv2.COLOR_RGB2GRAY)
    height, width = current_mask.shape
    grid_x, grid_y = np.meshgrid(np.arange(width, dtype=np.float32), np.arange(height, dtype=np.float32))
    total = np.zeros_like(current, dtype=np.float32)
    count = np.zeros((height, width), dtype=np.float32)
    for reference, reference_mask in references:
        ref_gray = cv2.cvtColor(reference, cv2.COLOR_RGB2GRAY)
        flow = cv2.calcOpticalFlowFarneback(target_gray, ref_gray, None, 0.5, 2, 15, 2, 5, 1.1, 0)
        map_x = grid_x + flow[..., 0]
        map_y = grid_y + flow[..., 1]
        warped = cv2.remap(reference, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
        warped_mask = cv2.remap(reference_mask, map_x, map_y, cv2.INTER_NEAREST, borderMode=cv2.BORDER_CONSTANT, borderValue=255)
        valid = (current_mask > 0) & (warped_mask < 128)
        total[valid] += warped[valid].astype(np.float32)
        count[valid] += 1
    valid = count > 0
    output = np.zeros_like(current)
    if np.any(valid):
        output[valid] = np.clip(total[valid] / count[valid, None], 0, 255).astype(np.uint8)
    return output, valid


def process_frames(request: dict[str, Any], emit: Callable[[dict[str, object]], None]) -> None:
    frames_dir = Path(str(request["framesDir"]))
    masks_dir = Path(str(request["masksDir"]))
    output_dir = Path(str(request["outputDir"]))
    cache_dir = str(request["cacheDir"])
    model_path = Path(str(request["modelPath"]))
    model = str(request.get("ocrModel", "tiny"))
    device = str(request.get("ocrDevice", "auto"))
    frames = sorted(frames_dir.glob("frame_*.jpg"))
    if not frames:
        raise RuntimeError("Không có frame trong vùng đã chọn để xóa chữ.")
    if not model_path.is_file() or model_path.stat().st_size != MODEL_BYTES:
        raise RuntimeError("Chưa cài đủ mô hình xóa chữ bằng AI. Hãy tải mô hình trong thuộc tính vùng.")
    masks_dir.mkdir(parents=True, exist_ok=True)
    output_dir.mkdir(parents=True, exist_ok=True)
    engine = _create_engine(cache_dir, model, device)
    total = len(frames)
    detections: list[list[tuple[int, int, int, int]]] = []
    mask_paths: list[Path] = []
    for index, frame_path in enumerate(frames):
        image = np.asarray(Image.open(frame_path).convert("RGB"))
        h, w = image.shape[:2]
        result = engine(cv2.cvtColor(image, cv2.COLOR_RGB2BGR))
        mask, rects = _make_mask(_result_boxes(result), w, h)
        mask_path = masks_dir / f"mask_{index + 1:08d}.png"
        Image.fromarray(mask).save(mask_path, optimize=False)
        detections.append(rects)
        mask_paths.append(mask_path)
        if index % max(1, total // 100) == 0 or index + 1 == total:
            emit({"type": "progress", "job": "text_removal", "stage": "detect", "progress": 0.35 * (index + 1) / total,
                  "message": f"Đang nhận diện nét chữ · {index + 1}/{total} khung hình"})

    import onnxruntime as ort
    session = _lama_session(model_path)
    completed_mask_pixels = 0
    for index, frame_path in enumerate(frames):
        current = np.asarray(Image.open(frame_path).convert("RGB"))
        mask = np.asarray(Image.open(mask_paths[index]).convert("L"))
        if np.any(mask):
            lama = _infer_tiles(session, current, mask, detections[index])
            reference_indices = [candidate for candidate in (index - 3, index - 1, index + 1, index + 3)
                                 if 0 <= candidate < total]
            references = []
            for candidate in reference_indices:
                ref_image = np.asarray(Image.open(frames[candidate]).convert("RGB"))
                ref_mask = np.asarray(Image.open(mask_paths[candidate]).convert("L"))
                references.append((ref_image, ref_mask))
            temporal, temporal_valid = _temporal_fill(current, mask, references)
            result = lama
            result[temporal_valid] = temporal[temporal_valid]
            completed_mask_pixels += int(np.count_nonzero(mask))
            # The extracted crop is a lossy JPEG for reasonable temporary disk use.
            # Export only pixels covered by the expanded text mask; keeping the rest
            # transparent preserves the source video exactly outside the repaired glyphs.
            alpha = cv2.GaussianBlur(mask, (0, 0), sigmaX=0.75)
        else:
            result = current
            alpha = np.zeros(mask.shape, dtype=np.uint8)
        output_path = output_dir / f"clean_{index + 1:08d}.png"
        Image.fromarray(np.dstack((result, alpha)), mode="RGBA").save(output_path, format="PNG", compress_level=1)
        if index % max(1, total // 100) == 0 or index + 1 == total:
            emit({"type": "progress", "job": "text_removal", "stage": "inpaint", "progress": 0.35 + 0.65 * (index + 1) / total,
                  "message": f"Đang tái tạo nền bằng AI · {index + 1}/{total} khung hình"})
    if completed_mask_pixels == 0:
        raise RuntimeError("Không nhận diện được chữ trong vùng này. Hãy kiểm tra vị trí vùng hoặc cài model OCR.")
    emit({"type": "completed", "job": "text_removal", "frames": total,
          "maskedPixels": completed_mask_pixels, "device": "cuda" if "CUDAExecutionProvider" in ort.get_available_providers() else "cpu"})
