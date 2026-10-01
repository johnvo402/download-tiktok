from __future__ import annotations

import json
import logging
import re
import time
import unicodedata
from difflib import SequenceMatcher
from threading import Lock
from pathlib import Path
from typing import Any, Callable

from .backends import AUTO_PRIORITY, resolve_ocr_backend, runtime_capabilities

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

_CUDA_PROBE_LOCK = Lock()
_CUDA_PROBE_SUCCEEDED = False


def _protobuf_varint(value: int) -> bytes:
    encoded = bytearray()
    while value > 0x7F:
        encoded.append((value & 0x7F) | 0x80)
        value >>= 7
    encoded.append(value)
    return bytes(encoded)


def _protobuf_int(field_number: int, value: int) -> bytes:
    return _protobuf_varint(field_number << 3) + _protobuf_varint(value)


def _protobuf_bytes(field_number: int, value: bytes) -> bytes:
    return (
        _protobuf_varint((field_number << 3) | 2)
        + _protobuf_varint(len(value))
        + value
    )


def _cuda_probe_model() -> bytes:
    """Build a tiny ONNX Add graph without adding the ONNX package as a dependency."""
    tensor_shape = _protobuf_bytes(1, _protobuf_int(1, 1))
    tensor_type = _protobuf_int(1, 1) + _protobuf_bytes(2, tensor_shape)
    value_type = _protobuf_bytes(1, tensor_type)

    def value_info(name: bytes) -> bytes:
        return _protobuf_bytes(1, name) + _protobuf_bytes(2, value_type)

    add_node = (
        _protobuf_bytes(1, b"left")
        + _protobuf_bytes(1, b"right")
        + _protobuf_bytes(2, b"sum")
        + _protobuf_bytes(3, b"cuda_probe_add")
        + _protobuf_bytes(4, b"Add")
    )
    graph = (
        _protobuf_bytes(1, add_node)
        + _protobuf_bytes(2, b"cuda_probe")
        + _protobuf_bytes(11, value_info(b"left"))
        + _protobuf_bytes(11, value_info(b"right"))
        + _protobuf_bytes(12, value_info(b"sum"))
    )
    opset = _protobuf_int(2, 13)
    return _protobuf_int(1, 8) + _protobuf_bytes(7, graph) + _protobuf_bytes(8, opset)


def _cuda_runtime_probe() -> tuple[bool, str | None]:
    """Confirm the CUDA provider can load and execute a CUDA kernel, not just advertise itself."""
    global _CUDA_PROBE_SUCCEEDED
    if _CUDA_PROBE_SUCCEEDED:
        return True, None

    with _CUDA_PROBE_LOCK:
        if _CUDA_PROBE_SUCCEEDED:
            return True, None
        try:
            import numpy as np
            import onnxruntime as ort

            providers = list(ort.get_available_providers())
            if "CUDAExecutionProvider" not in providers:
                return False, "ONNX Runtime không có CUDAExecutionProvider."
            if hasattr(ort, "preload_dlls"):
                ort.preload_dlls()

            options = ort.SessionOptions()
            options.add_session_config_entry("session.disable_cpu_ep_fallback", "1")
            session = ort.InferenceSession(
                _cuda_probe_model(),
                sess_options=options,
                providers=["CUDAExecutionProvider"],
            )
            active_providers = list(session.get_providers())
            if not active_providers or active_providers[0] != "CUDAExecutionProvider":
                return False, f"Phiên ONNX Runtime đang dùng provider: {active_providers}."

            output = session.run(
                ["sum"],
                {"left": np.array([1.0], dtype=np.float32), "right": np.array([2.0], dtype=np.float32)},
            )[0]
            if output.tolist() != [3.0]:
                return False, "Phép thử tính toán trên CUDA trả về kết quả không hợp lệ."
            _CUDA_PROBE_SUCCEEDED = True
            return True, None
        except Exception as error:
            return False, f"{type(error).__name__}: {error}"


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
    available, _ = _cuda_runtime_probe()
    return available


def _runtime_diagnostics() -> dict[str, Any]:
    try:
        import onnxruntime as ort
        version = str(getattr(ort, "__version__", "unknown"))
        providers = list(ort.get_available_providers())
        device = str(ort.get_device())
        cuda_available, cuda_error = _cuda_runtime_probe()
        return {
            "onnxRuntimeVersion": version,
            "onnxRuntimeDevice": device,
            "executionProviders": providers,
            "cudaAvailable": cuda_available,
            "runtimeError": cuda_error,
        }
    except (ImportError, OSError, RuntimeError) as error:
        return {
            "onnxRuntimeVersion": None,
            "onnxRuntimeDevice": None,
            "executionProviders": [],
            "cudaAvailable": False,
            "runtimeError": str(error),
        }


def _runtime_installed() -> bool:
    try:
        import onnxruntime  # noqa: F401
        return True
    except (ImportError, OSError, RuntimeError):
        return False


def _resolved_device(device: str | None) -> tuple[str, bool]:
    capability = resolve_ocr_backend(device)
    return capability.backend, capability.backend == "cuda"


def inspect_ocr(cache_dir: str, model: str = DEFAULT_MODEL) -> dict[str, Any]:
    selected_model, config = _profile(model)
    try:
        import rapidocr  # noqa: F401
        installed = True
    except ImportError:
        installed = False
    root = _model_root(cache_dir, selected_model)
    cached, model_bytes = _model_cached(root, selected_model)
    runtime = _runtime_diagnostics()
    capabilities = runtime_capabilities()
    recommended = resolve_ocr_backend("auto", capabilities) if any(c.available for c in capabilities.values()) else None
    cuda_available = capabilities["cuda"].available
    LOGGER.info(
        "RapidOCR environment: model=%s, runtime=%s, device=%s, providers=%s, cudaReady=%s",
        selected_model,
        runtime["onnxRuntimeVersion"],
        runtime["onnxRuntimeDevice"],
        runtime["executionProviders"],
        cuda_available,
    )
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
        "activeDevice": recommended.backend if recommended else "cpu",
        "backends": {key: cap.payload() for key, cap in capabilities.items()},
        "recommendedBackend": recommended.backend if recommended else "cpu",
        "resolvedLabel": recommended.label if recommended else "Chưa sẵn sàng",
        "onnxRuntimeVersion": runtime["onnxRuntimeVersion"],
        "onnxRuntimeDevice": runtime["onnxRuntimeDevice"],
        "executionProviders": runtime["executionProviders"],
        "runtimeError": runtime.get("runtimeError"),
    }


class _AutoOcrEngine:
    """Auto alone may recover from real-model initialization/inference failure."""
    def __init__(self, engine: Any, backend: str, fallback: Callable[[str], Any]):
        self.engine = engine
        self.resolved_backend = backend
        self.fallback = fallback
        self.verified = False
        self.ocr_auto_fallback = False
        self.lock = Lock()

    def __getattr__(self, name: str) -> Any:
        return getattr(self.engine, name)

    def __call__(self, image: Any, **kwargs: Any) -> Any:
        with self.lock:
            while True:
                try:
                    result = self.engine(image, **kwargs)
                    if not self.verified:
                        verify_engine_device(self.engine, self.resolved_backend, "auto-initialization")
                        self.verified = True
                    return result
                except Exception:
                    if self.resolved_backend == "cpu":
                        raise
                    LOGGER.exception("OCR Auto backend failed: %s", self.resolved_backend)
                    candidates = AUTO_PRIORITY[AUTO_PRIORITY.index(self.resolved_backend) + 1:]
                    for candidate in candidates:
                        try:
                            replacement = self.fallback(candidate)
                        except Exception:
                            LOGGER.info("OCR Auto fallback unavailable: %s", candidate, exc_info=True)
                            continue
                        self.engine = replacement
                        self.resolved_backend = candidate
                        self.ocr_auto_fallback = True
                        self.verified = False
                        LOGGER.warning("OCR Auto switched to backend=%s", candidate)
                        break
                    else:
                        raise RuntimeError("Không có thiết bị đọc chữ sẵn sàng. Kiểm tra Cài đặt rồi thử lại.")


def _auto_wrap(engine: Any, requested: str, backend: str, cache_dir: str, model: str) -> Any:
    if requested == "auto" and backend != "cpu":
        return _AutoOcrEngine(engine, backend, lambda candidate: _create_engine(cache_dir, model, candidate))
    return engine


def _create_engine(cache_dir: str, model: str = DEFAULT_MODEL, device: str = "auto"):
    try:
        from rapidocr import ModelType, OCRVersion, RapidOCR
    except ImportError as error:
        raise RuntimeError("Bộ đọc chữ chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI.") from error
    selected_model, config = _profile(model)
    if not _runtime_installed():
        raise RuntimeError("Bộ đọc chữ chưa đầy đủ. Hãy cập nhật hoặc cài lại JC AI.")
    capability = resolve_ocr_backend(device)
    active_device = capability.backend
    runtime = _runtime_diagnostics()
    LOGGER.info(
        "RapidOCR engine requested: device=%s, resolved=%s, model=%s, runtime=%s, providers=%s",
        device or "auto",
        active_device,
        selected_model,
        runtime["onnxRuntimeVersion"],
        runtime["executionProviders"],
    )
    root = _model_root(cache_dir, selected_model)
    root.mkdir(parents=True, exist_ok=True)
    # RapidOCR 3.x supports separate PP-OCRv6 Tiny, Small, and Medium profiles.
    # Auto uses CUDA only when the packaged ONNX Runtime exposes its provider.
    params = {
        "Global.model_root_dir": str(root),
        "EngineConfig.onnxruntime.use_cuda": active_device == "cuda",
        # RapidOCR 3.9 validates these settings as enum members rather than
        # accepting their serialized string values.
        "Det.ocr_version": OCRVersion.PPOCRV6,
        "Det.model_type": getattr(ModelType, config["enum"]),
        "Rec.ocr_version": OCRVersion.PPOCRV6,
        "Rec.model_type": getattr(ModelType, config["enum"]),
    }
    if active_device == "openvino":
        if not _model_cached(root, selected_model)[0]:
            raise RuntimeError("Model OCR chưa được cài. Cài model trong Cài đặt trước khi đọc chữ.")
        try:
            from .openvino_engine import RapidOCROpenVinoBackend
            engine = RapidOCROpenVinoBackend(params, _model_files(root), config["requiredFiles"], capability.device,
                                              Path(cache_dir).parent / "runtimes" / "advanced" / "compiled-ocr")
            return _auto_wrap(engine, device, active_device, cache_dir, selected_model)
        except Exception as error:
            LOGGER.exception("OpenVINO model initialization failed: model=%s device=%s", selected_model, capability.device)
            if device == "auto":
                LOGGER.warning("OCR Auto falling back to CPU after OpenVINO model failure")
                fallback = RapidOCR(params=params)
                fallback.ocr_auto_fallback = True
                return fallback
            raise RuntimeError("Không thể khởi tạo model đọc chữ trên GPU Intel. Kiểm tra driver hoặc chọn Tự động/CPU.") from error
    try:
        engine = RapidOCR(params=params)
    except Exception as error:
        LOGGER.exception("OCR initialization failed: backend=%s model=%s", active_device, selected_model)
        if device == "auto" and active_device == "cuda":
            for candidate in AUTO_PRIORITY[AUTO_PRIORITY.index("cuda") + 1:]:
                try:
                    fallback = _create_engine(cache_dir, selected_model, candidate)
                    LOGGER.warning("OCR Auto initialization switched to %s", candidate)
                    return _auto_wrap(fallback, "auto", candidate, cache_dir, selected_model)
                except Exception:
                    LOGGER.info("OCR fallback unavailable: %s", candidate, exc_info=True)
        raise RuntimeError("Không thể khởi tạo bộ đọc chữ. Kiểm tra thiết bị và model trong Cài đặt.") from error
    return _auto_wrap(engine, device, active_device, cache_dir, selected_model)


def _engine_session_providers(engine: Any) -> dict[str, list[str]]:
    """Return providers registered on RapidOCR's lazily initialized ORT sessions."""
    sessions: dict[str, list[str]] = {}
    for component_name, label in (("text_det", "det"), ("text_cls", "cls"), ("text_rec", "rec")):
        component = getattr(engine, component_name, None)
        wrapper = getattr(component, "session", None)
        session = getattr(wrapper, "session", wrapper)
        get_providers = getattr(session, "get_providers", None)
        if callable(get_providers):
            try:
                sessions[label] = list(get_providers())
            except Exception as error:  # diagnostics must not mask OCR errors
                LOGGER.warning("Could not inspect RapidOCR %s session providers: %s", label, error)
    return sessions


def verify_engine_device(engine: Any, resolved_device: str, operation: str = "transcription") -> dict[str, list[str]]:
    if getattr(engine, "backend", None) == "openvino":
        sessions = {label: list(getattr(engine, name).session.devices)
                    for name, label in (("text_det", "det"), ("text_cls", "cls"), ("text_rec", "rec"))}
        if len(sessions) != 3 or any(not devices or any(not d.startswith("GPU") for d in devices) for devices in sessions.values()):
            raise RuntimeError("Không xác minh được model đọc chữ chạy trên GPU Intel.")
        LOGGER.info("RapidOCR %s OpenVINO actual execution devices: %s", operation, sessions)
        return sessions
    # Auto can fall back when a real model operator is unsupported.
    if resolved_device == "openvino" and not getattr(engine, "ocr_auto_fallback", False):
        raise RuntimeError("Không xác minh được RapidOCR chạy trên GPU Intel; đã dừng để tránh dùng CPU ngoài ý muốn.")
    if resolved_device == "openvino":
        LOGGER.info("RapidOCR %s: Auto model fallback to ORT CPU", operation)
    if getattr(engine, "ocr_auto_fallback", False):
        resolved_device = "cpu"
    sessions = _engine_session_providers(engine)
    LOGGER.info("RapidOCR %s session providers: %s", operation, sessions or "not exposed")
    if resolved_device == "cuda":
        if not sessions:
            raise RuntimeError("Không xác minh được RapidOCR đã khởi tạo phiên CUDA. Hãy cập nhật runtime OCR rồi thử lại.")
        cpu_only = [name for name, providers in sessions.items()
                    if not providers or providers[0] != "CUDAExecutionProvider"]
        if cpu_only:
            raise RuntimeError(
                "RapidOCR được chọn chạy NVIDIA nhưng phiên model đang dùng CPU "
                f"({', '.join(cpu_only)}). Hãy mở Cài đặt → Nhận dạng lời thoại, kiểm tra lại tăng tốc NVIDIA."
            )
    return sessions


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
    requested_device = str(request.get("device", "auto"))
    resolved_device, _ = _resolved_device(requested_device)
    emit({"type": "progress", "stage": "ocr-model", "progress": 0.0,
          "message": "Đang khởi tạo model đọc chữ" + (" trên GPU Intel…" if resolved_device == "openvino" else "…")})
    engine = _create_engine(
        str(request["cacheDir"]),
        str(request.get("model", DEFAULT_MODEL)),
        requested_device,
    )
    groups: list[dict[str, Any]] = []
    total = len(frames)
    interval = 1.0 / fps
    # Allow a missed OCR sample (for example, a subtitle animation frame) to
    # bridge the same caption, while keeping separate captions apart.
    repeat_gap = min(1.5, interval * 2.5)
    started = time.perf_counter()
    detected_frames = 0
    for index, path in enumerate(frames):
        match = re.search(r"frame_(\d+)\.jpg$", path.name)
        timestamp = max(0.0, (int(match.group(1)) - 1) / fps) if match else index / fps
        result = engine(str(path))
        if index == 0:
            verify_engine_device(engine, resolved_device, "transcript")
        text = _extract_text(result)
        normalized = _normalized(text)
        if normalized:
            detected_frames += 1
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
    elapsed = time.perf_counter() - started
    LOGGER.info(
        "RapidOCR transcript complete: device=%s, model=%s, frames=%d, detectedFrames=%d, segments=%d, elapsed=%.2fs, avg=%.1fms/frame",
        getattr(engine, "resolved_backend", resolved_device),
        request.get("model", DEFAULT_MODEL),
        total,
        detected_frames,
        len(merged),
        elapsed,
        elapsed * 1000 / max(total, 1),
    )
    emit({"type": "completed", "job": "ocr_transcribe", "output": str(output_path), "segments": len(merged)})
