"""ASR selection and normalized results, independent of inference libraries."""
from __future__ import annotations

from dataclasses import dataclass
import importlib.util
import math
from typing import Any


@dataclass(frozen=True)
class AsrSelection:
    engine: str
    device: str

    @property
    def label(self) -> str:
        return {"cuda": "NVIDIA · CUDA", "openvino": "Intel GPU · OpenVINO", "cpu": "CPU · INT8"}[self.device]


def resolve_asr_backend(engine: str, device: str, capabilities: dict[str, bool]) -> AsrSelection:
    if engine not in {"auto", "fasterWhisper", "openVinoWhisper", "whisperX"}:
        raise ValueError("Engine nhận dạng lời thoại không được hỗ trợ.")
    if device not in {"auto", "cpu", "cuda", "openvino"}:
        raise ValueError("Thiết bị nhận dạng lời thoại không được hỗ trợ.")
    if engine in {"fasterWhisper", "whisperX"} and device == "openvino":
        raise ValueError("Intel GPU cần chọn engine Tự động hoặc Whisper Intel; WhisperX không hỗ trợ Intel.")
    if engine == "openVinoWhisper" and device not in {"auto", "openvino"}:
        raise ValueError("Whisper Intel chỉ hỗ trợ thiết bị Intel GPU.")
    candidates = [device] if device != "auto" else (
        ["openvino"] if engine == "openVinoWhisper" else
        ["cuda", "cpu"] if engine in {"fasterWhisper", "whisperX"} else
        ["cuda", "openvino", "cpu"]
    )
    for candidate in candidates:
        if capabilities.get(candidate, False):
            selected_engine = "openVinoWhisper" if candidate == "openvino" else "whisperX" if engine == "whisperX" else "fasterWhisper"
            return AsrSelection(selected_engine, candidate)
    raise RuntimeError({"openvino": "Whisper Intel chưa sẵn sàng. Cài tăng tốc Intel và model trong Cài đặt → Nhận dạng lời thoại.",
                        "cuda": "Tăng tốc NVIDIA chưa sẵn sàng. Cài tăng tốc hoặc chọn CPU.",
                        "cpu": "Bộ nhận dạng CPU chưa sẵn sàng. Hãy cập nhật hoặc cài lại JC AI."}.get(device, "Không có bộ nhận dạng lời thoại sẵn sàng."))


def runtime_capabilities() -> dict[str, bool]:
    cpu = importlib.util.find_spec("faster_whisper") is not None
    cuda = False
    if cpu:
        try:
            import ctranslate2
            cuda = ctranslate2.get_cuda_device_count() > 0
        except Exception:
            pass
    intel = False
    if importlib.util.find_spec("openvino_genai") is not None:
        from worker.ocr.backends import probe_openvino_gpu
        intel = probe_openvino_gpu().available
    return {"cpu": cpu, "cuda": cuda, "openvino": intel}


def normalize_segments(segments: list[dict[str, Any]], duration: float) -> list[dict[str, Any]]:
    if not math.isfinite(duration) or duration <= 0:
        raise ValueError("Thời lượng audio không hợp lệ.")
    result = []
    for item in segments:
        start, end = float(item["start"]), float(item["end"])
        text = str(item["text"]).strip()
        if not text:
            continue
        if not math.isfinite(start) or not math.isfinite(end) or start < 0 or end <= start or end > duration + 0.25:
            raise ValueError("Model nhận dạng trả timestamp không hợp lệ. Không lưu transcript.")
        end = min(end, duration)
        if end <= start:
            raise ValueError("Timestamp nằm ngoài thời lượng audio.")
        result.append({**item, "start": start, "end": end, "text": text})
    return sorted(result, key=lambda item: (item["start"], item["end"]))


class FasterWhisperBackend:
    def __init__(self, model_name, model_dir, device, emit):
        self.args = model_name, model_dir, device, emit
        self.model = None

    def load(self):
        from worker.asr.service import _install_pyav_metadata_errors_compat, _load_whisper_model_with_progress, _cpu_thread_budget
        _install_pyav_metadata_errors_compat()
        name, path, device, emit = self.args
        self.model = _load_whisper_model_with_progress(name, path, device,
            "int8" if device == "cpu" else "float16", _cpu_thread_budget() if device == "cpu" else 4, emit)

    def transcribe(self, audio_path, language, on_progress, cancelled=lambda: False):
        from worker.asr.service import _transcribe_audio
        return _transcribe_audio(self.model, str(audio_path), language, on_progress)

    def close(self):
        self.model = None
