from __future__ import annotations

import json
import logging
import re
import unicodedata
from difflib import SequenceMatcher
from pathlib import Path
from typing import Any, Callable

LOGGER = logging.getLogger(__name__)
MODEL_DIR = "rapidocr"
DEFAULT_MODEL = "tiny"
MODEL_PROFILES: dict[str, dict[str, Any]] = {
    "tiny": {
        "label": "PP-OCRv6 Tiny", "enum": "TINY", "estimatedBytes": 7 * 1024 * 1024,
        "requiredFiles": ("PP-OCRv6_det_tiny.onnx", "PP-OCRv6_rec_tiny.onnx", "ch_ppocr_mobile_v2.0_cls_mobile.onnx"),
        "optionalFiles": ("ppocrv6_tiny_dict.txt",),
    },
    "small": {
        "label": "PP-OCRv6 Small", "enum": "SMALL", "estimatedBytes": 31 * 1024 * 1024,
        "requiredFiles": ("PP-OCRv6_det_small.onnx", "PP-OCRv6_rec_small.onnx", "ch_ppocr_mobile_v2.0_cls_mobile.onnx"),
        "optionalFiles": ("ppocrv6_dict.txt",),
    },
    "medium": {
        "label": "PP-OCRv6 Medium", "enum": "MEDIUM", "estimatedBytes": 136 * 1024 * 1024,
        "requiredFiles": ("PP-OCRv6_det_medium.onnx", "PP-OCRv6_rec_medium.onnx", "ch_ppocr_mobile_v2.0_cls_mobile.onnx"),
        "optionalFiles": ("ppocrv6_dict.txt",),
    },
}


def _model_files(root: Path) -> dict[str, Path]:
    if not root.is_dir() or root.is_symlink():
        return {}
    found: dict[str, Path] = {}
    for item in root.rglob("*"):
        if item.is_file() and not item.is_symlink():
            found.setdefault(item.name, item)
    return found


def _profile(model: str | None) -> tuple[str, dict[str, Any]]:
    selected = model or DEFAULT_MODEL
    config = MODEL_PROFILES.get(selected)
    if config is None:
        raise ValueError("Model OCR phải là tiny, small hoặc medium.")
    return selected, config


def _model_cached(root: Path, model: str) -> tuple[bool, int]:
    _, config = _profile(model)
    files = _model_files(root)
    required = config["requiredFiles"]
    cached = all(name in files and files[name].stat().st_size > 1024 for name in required)
    if not cached:
        return False, 0
    return True, sum(files[name].stat().st_size for name in (*required, *config["optionalFiles"]) if name in files)


def _model_root(cache_dir: str, model: str) -> Path:
    base = Path(cache_dir) / MODEL_DIR
    selected_root = base / model
    cached, _ = _model_cached(selected_root, model)
    if cached:
        return selected_root
    # Existing versions stored PP-OCRv6 Tiny directly under models/rapidocr.
    if model == "tiny":
        legacy_cached, _ = _model_cached(base, model)
        if legacy_cached:
            return base
    return selected_root


def _cuda_available() -> bool:
    try:
        import onnxruntime
        return "CUDAExecutionProvider" in onnxruntime.get_available_providers()
    except (ImportError, OSError, RuntimeError):
        return False


def _runtime_installed() -> bool:
    try:
        import onnxruntime  # noqa: F401
        return True
    except (ImportError, OSError, RuntimeError):
        return False


def _resolved_device(device: str | None) -> tuple[str, bool]:
    selected = device or "auto"
    if selected not in ("auto", "cpu", "cuda"):
        raise ValueError("Thiết bị OCR phải là auto, cpu hoặc cuda.")
    available = _cuda_available()
    if selected == "cuda" and not available:
        raise RuntimeError("Tăng tốc NVIDIA chưa sẵn sàng cho đọc chữ. Chọn Tự động hoặc CPU, hoặc cài bản NVIDIA trong Cài đặt.")
    return ("cuda" if selected == "cuda" or (selected == "auto" and available) else "cpu", available)


def inspect_ocr(cache_dir: str, model: str = DEFAULT_MODEL) -> dict[str, Any]:
    selected_model, config = _profile(model)
    try:
        import rapidocr  # noqa: F401
        installed = True
    except ImportError:
        installed = False
    root = _model_root(cache_dir, selected_model)
    cached, model_bytes = _model_cached(root, selected_model)
    cuda_available = _cuda_available()
    return {
        "workerInstalled": True,
        "ocrInstalled": installed,
        "runtimeInstalled": _runtime_installed(),
        "modelCached": cached,
        "engine": "RapidOCR · ONNX Runtime",
        "model": config["label"],
        "modelProfile": selected_model,
        "modelBytes": model_bytes if cached else 0,
        "estimatedDownloadBytes": config["estimatedBytes"],
        "cudaAvailable": cuda_available,
        "activeDevice": "cuda" if cuda_available else "cpu",
    }


def _create_engine(cache_dir: str, model: str = DEFAULT_MODEL, device: str = "auto"):
    try:
        from rapidocr import ModelType, OCRVersion, RapidOCR
    except ImportError as error:
        raise RuntimeError("Bộ đọc chữ chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.") from error
    selected_model, config = _profile(model)
    if not _runtime_installed():
        raise RuntimeError("Bộ đọc chữ chưa đầy đủ. Hãy cập nhật hoặc cài lại JC AI.")
    active_device, _ = _resolved_device(device)
    root = _model_root(cache_dir, selected_model)
    root.mkdir(parents=True, exist_ok=True)
    # RapidOCR 3.x supports separate PP-OCRv6 Tiny, Small, and Medium profiles.
    # Auto uses CUDA only when the packaged ONNX Runtime exposes its provider.
    return RapidOCR(params={
        "Global.model_root_dir": str(root),
        "EngineConfig.onnxruntime.use_cuda": active_device == "cuda",
        # RapidOCR 3.9 validates these settings as enum members rather than
        # accepting their serialized string values.
        "Det.ocr_version": OCRVersion.PPOCRV6,
        "Det.model_type": getattr(ModelType, config["enum"]),
        "Rec.ocr_version": OCRVersion.PPOCRV6,
        "Rec.model_type": getattr(ModelType, config["enum"]),
    })


def install_ocr(cache_dir: str, emit: Callable[[dict[str, object]], None], model: str = DEFAULT_MODEL) -> None:
    selected_model, config = _profile(model)
    state = inspect_ocr(cache_dir, selected_model)
    if state["modelCached"]:
        emit({"type": "completed", "job": "ocr_install", "model": selected_model, "alreadyInstalled": True})
        return
    estimate = int(config["estimatedBytes"])
    emit({"type": "progress", "job": "ocr_install", "progress": 0.0,
          "downloadedBytes": 0, "totalBytes": estimate,
          "message": f"Đang tải {config['label']} · khoảng {estimate / (1024 * 1024):.0f} MB…"})
    try:
        root = _model_root(cache_dir, selected_model)
        import threading
        import numpy as np
        from PIL import Image, ImageDraw, ImageFont
        engine = _create_engine(cache_dir, selected_model, "cpu")
        # Use visible text so lazy initialization loads detector, angle
        # classifier, and recognizer weights. Transcript generation itself
        # never downloads models.
        sample = Image.new("RGB", (640, 160), "white")
        ImageDraw.Draw(sample).text((18, 44), "RAPIDOCR 12345", fill="black", font=ImageFont.load_default(size=72))
        error: list[Exception] = []

        def load_models() -> None:
            try:
                engine(np.asarray(sample))
            except Exception as exc:  # propagate worker-thread model errors
                error.append(exc)

        before = {str(path): path.stat().st_size for path in _model_files(root).values()}
        loader = threading.Thread(target=load_models, name="rapidocr-model-download", daemon=True)
        loader.start()
        while loader.is_alive():
            current = _model_files(root)
            downloaded = 0
            for path in current.values():
                key = str(path)
                size = path.stat().st_size
                downloaded += max(0, size - before.get(key, 0))
            emit({
                "type": "progress",
                "job": "ocr_install",
                "progress": min(downloaded / estimate, 0.95),
                "downloadedBytes": downloaded,
                "totalBytes": estimate,
                "message": f"Đang tải trọng số {config['label']}…",
            })
            loader.join(timeout=0.5)
        if error:
            raise error[0]
    except Exception as error:
        raise RuntimeError(f"Không thể cài model RapidOCR: {error}") from error
    state = inspect_ocr(cache_dir, selected_model)
    if not state["modelCached"]:
        raise RuntimeError(f"RapidOCR chạy xong nhưng chưa tìm thấy đủ file {config['label']} trong thư mục model.")
    emit({"type": "progress", "job": "ocr_install", "progress": 1.0,
          "downloadedBytes": state["modelBytes"], "totalBytes": state["modelBytes"],
          "message": f"Đã tải xong {config['label']}"})
    emit({"type": "completed", "job": "ocr_install", "model": selected_model, "alreadyInstalled": False})


def _extract_text(result: Any) -> str:
    if result is None:
        return ""
    texts = getattr(result, "txts", None)
    boxes = getattr(result, "boxes", None)
    if texts is None and isinstance(result, (tuple, list)) and len(result) >= 2:
        boxes, texts = result[0], result[1]
    if texts is None or len(texts) == 0:
        return ""
    rows: list[tuple[float, float, str]] = []
    for index, text in enumerate(texts):
        clean = " ".join(str(text).split()).strip()
        if not clean:
            continue
        y = float(index)
        x = float(index)
        if boxes is not None and index < len(boxes):
            try:
                points = boxes[index]
                x = min(float(point[0]) for point in points)
                y = sum(float(point[1]) for point in points) / len(points)
            except (TypeError, ValueError, IndexError):
                pass
        rows.append((y, x, clean))
    rows.sort(key=lambda row: (round(row[0] / 18), row[1]))
    return "\n".join(row[2] for row in rows)


def _normalized(text: str) -> str:
    # OCR commonly alternates punctuation, whitespace, and a single accented
    # character from one sampled frame to the next. Those cosmetic changes
    # should not split a subtitle that stayed on screen continuously.
    return "".join(
        char for char in unicodedata.normalize("NFKC", text).casefold()
        if char.isalnum()
    )


def _repeat_similarity(left: str, right: str) -> float:
    if not left or not right:
        return 0.0
    if left == right:
        return 1.0
    # Disable SequenceMatcher's autojunk heuristic: captions are short, and
    # repeated common characters are meaningful in many scripts.
    ratio = SequenceMatcher(None, left, right, autojunk=False).ratio()
    shorter, longer = sorted((left, right), key=len)
    if len(shorter) >= 4 and shorter in longer:
        ratio = max(ratio, len(shorter) / len(longer))
    return ratio


def _add_variant(group: dict[str, Any], text: str, normalized: str) -> None:
    variants: list[dict[str, Any]] = group.setdefault("variants", [])
    variants.append({"text": text, "normalized": normalized})
    # Pick a representative OCR result that is most similar to the rest of
    # this subtitle's frames; on a tie prefer the variant seen most often and
    # then the fuller transcription.
    candidates: dict[str, dict[str, Any]] = {}
    for item in variants:
        key = item["text"]
        if key not in candidates:
            candidates[key] = {"text": key, "normalized": item["normalized"], "count": 0}
        candidates[key]["count"] += 1
    best = max(
        candidates.values(),
        key=lambda candidate: (
            sum(_repeat_similarity(candidate["normalized"], item["normalized"]) for item in variants),
            candidate["count"],
            len(candidate["text"]),
        ),
    )
    group["text"] = best["text"]
    group["normalized"] = best["normalized"]


def transcribe_frames(request: dict[str, Any], emit: Callable[[dict[str, object]], None]) -> None:
    frames_dir = Path(str(request["framesDir"]))
    output_path = Path(str(request["outputPath"]))
    fps = float(request["fps"])
    if fps not in (1.0, 2.0, 4.0):
        raise ValueError("OCR chỉ hỗ trợ lấy mẫu 1, 2 hoặc 4 khung hình/giây.")
    frames = sorted(frames_dir.glob("frame_*.jpg"))
    if not frames:
        raise RuntimeError("FFmpeg không trích xuất được frame trong vùng OCR đã chọn.")
    engine = _create_engine(
        str(request["cacheDir"]),
        str(request.get("model", DEFAULT_MODEL)),
        str(request.get("device", "auto")),
    )
    groups: list[dict[str, Any]] = []
    total = len(frames)
    interval = 1.0 / fps
    # Allow a missed OCR sample (for example, a subtitle animation frame) to
    # bridge the same caption, while keeping separate captions apart.
    repeat_gap = min(1.5, interval * 2.5)
    for index, path in enumerate(frames):
        match = re.search(r"frame_(\d+)\.jpg$", path.name)
        timestamp = max(0.0, (int(match.group(1)) - 1) / fps) if match else index / fps
        text = _extract_text(engine(str(path)))
        normalized = _normalized(text)
        if normalized:
            if groups and timestamp - groups[-1]["end"] <= repeat_gap and _repeat_similarity(groups[-1]["normalized"], normalized) >= 0.78:
                groups[-1]["end"] = max(groups[-1]["end"], timestamp + interval)
                _add_variant(groups[-1], text, normalized)
            else:
                group = {"start": timestamp, "end": timestamp + interval, "text": text, "normalized": normalized, "variants": []}
                _add_variant(group, text, normalized)
                groups.append(group)
        if index % max(1, int(fps)) == 0 or index + 1 == total:
            emit({"type": "progress", "stage": "ocr-frames", "progress": (index + 1) / total, "message": f"Đang nhận dạng subtitle trên hình · {index + 1}/{total} frame"})

    # Make one final adjacent pass after representative-text selection so a
    # noisy frame cannot leave duplicate caption entries in the transcript.
    merged: list[dict[str, Any]] = []
    for item in groups:
        if merged and item["start"] - merged[-1]["end"] <= repeat_gap and _repeat_similarity(merged[-1]["normalized"], item["normalized"]) >= 0.78:
            merged[-1]["end"] = max(merged[-1]["end"], item["end"])
            for variant in item.get("variants", [{"text": item["text"], "normalized": item["normalized"]}]):
                _add_variant(merged[-1], variant["text"], variant["normalized"])
        else:
            merged.append(item)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    payload = {"language": request.get("language", "zh"), "segments": [{"start": item["start"], "end": item["end"], "text": item["text"]} for item in merged]}
    output_path.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
    emit({"type": "completed", "job": "ocr_transcribe", "output": str(output_path), "segments": len(merged)})
