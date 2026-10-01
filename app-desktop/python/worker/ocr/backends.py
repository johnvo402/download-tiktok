"""OCR capability checks and selection. No optional imports at module load time."""
from __future__ import annotations

import logging
from dataclasses import asdict, dataclass
from enum import StrEnum
from threading import Lock
from typing import Callable, Mapping, Sequence

LOGGER = logging.getLogger(__name__)
OPENVINO_VERSION = "2026.4.0"
AUTO_PRIORITY = ("cuda", "openvino", "cpu")


class OcrBackend(StrEnum):
    AUTO = "auto"
    CPU = "cpu"
    CUDA = "cuda"
    OPENVINO = "openvino"


@dataclass(frozen=True)
class OcrRuntimeCapability:
    backend: str
    available: bool
    device: str | None = None
    label: str = ""
    message: str | None = None
    installed: bool = False
    version: str | None = None

    def payload(self) -> dict:
        return asdict(self)


_OV_LOCK = Lock()
_OV_SUCCESS: OcrRuntimeCapability | None = None


def probe_openvino_gpu(core_factory: Callable | None = None) -> OcrRuntimeCapability:
    """Listing devices is insufficient: compile ONNX Add, infer, verify output AND device.

    A supplied factory bypasses the successful-probe cache for unit tests.
    Never use AUTO/HETERO here, since these can execute on the CPU.
    """
    global _OV_SUCCESS
    with _OV_LOCK:
        if core_factory is None and _OV_SUCCESS is not None:
            return _OV_SUCCESS
        version = None
        installed = False
        try:
            import numpy as np
            if core_factory is None:
                import openvino as ov
                installed = True
                version = ov.__version__
                core_factory = ov.Core
                cache_success = True
            else:
                installed = True
                cache_success = False
            from .service import _cuda_probe_model
            core = core_factory()
            devices = sorted(d for d in core.available_devices if d == "GPU" or d.startswith("GPU."))
            if not devices:
                return OcrRuntimeCapability("openvino", False, message="Không tìm thấy GPU Intel. Kiểm tra driver Intel hoặc chọn Tự động/CPU.", installed=True, version=version)
            model = core.read_model(_cuda_probe_model())
            for device in devices:
                try:
                    compiled = core.compile_model(model, device, {"PERFORMANCE_HINT": "LATENCY"})
                    actual = list(compiled.get_property("EXECUTION_DEVICES"))
                    if not actual or any(not d.startswith("GPU") for d in actual):
                        raise RuntimeError(f"Unexpected execution devices: {actual}")
                    output = compiled({"left": np.array([1.0], dtype=np.float32), "right": np.array([2.0], dtype=np.float32)})
                    if not np.array_equal(next(iter(output.values())), np.array([3.0], dtype=np.float32)):
                        raise RuntimeError("Invalid Add inference result")
                    capability = OcrRuntimeCapability("openvino", True, device=device, label="Intel GPU · OpenVINO", installed=True, version=version)
                    LOGGER.info("OpenVINO inference probe passed: device=%s actual=%s version=%s", device, actual, version)
                    if cache_success:
                        _OV_SUCCESS = capability
                    return capability
                except Exception:
                    LOGGER.exception("OpenVINO GPU probe failed: device=%s version=%s", device, version)
        except ImportError:
            LOGGER.info("OpenVINO acceleration not installed")
        except Exception:
            LOGGER.exception("OpenVINO initialization failed")
        return OcrRuntimeCapability("openvino", False, message="Không thể khởi tạo tăng tốc Intel. Cài tăng tốc Intel hoặc cập nhật driver, rồi chọn Tự động/CPU nếu cần.", installed=installed, version=version)


def cpu_capability() -> OcrRuntimeCapability:
    try:
        import onnxruntime as ort
        # CPU provider is built into both CPU and CUDA distributions.
        ready = "CPUExecutionProvider" in ort.get_available_providers()
        return OcrRuntimeCapability("cpu", ready, "CPU", "CPU", installed=True, version=ort.__version__)
    except Exception:
        return OcrRuntimeCapability("cpu", False, message="Bộ đọc chữ chưa sẵn sàng. Hãy cập nhật hoặc sửa chữa JC AI.")


def runtime_capabilities() -> dict[str, OcrRuntimeCapability]:
    from .service import _cuda_runtime_probe
    ready, detail = _cuda_runtime_probe()
    cpu = cpu_capability()
    try:
        import onnxruntime as ort
        cuda_installed = "CUDAExecutionProvider" in ort.get_available_providers()
    except Exception:
        cuda_installed = False
    if detail:
        LOGGER.info("OCR CUDA probe unavailable: %s", detail)
    return {
        "cpu": cpu,
        "cuda": OcrRuntimeCapability("cuda", ready, "CUDA" if ready else None, "NVIDIA · CUDA", message=None if ready else "Tăng tốc NVIDIA chưa sẵn sàng. Kiểm tra driver và thành phần tăng tốc trong Cài đặt.", installed=cuda_installed, version=cpu.version),
        "openvino": probe_openvino_gpu(),
    }


def resolve_ocr_backend(requested: str | None, capabilities: Mapping[str, OcrRuntimeCapability] | None = None,
                        priority: Sequence[str] = AUTO_PRIORITY) -> OcrRuntimeCapability:
    selected = OcrBackend(requested or "auto")
    if capabilities is None:
        capabilities = {"cpu": cpu_capability()} if selected == OcrBackend.CPU else runtime_capabilities()
    if selected != OcrBackend.AUTO:
        capability = capabilities.get(selected)
        if capability is not None and capability.available:
            return capability
        raise RuntimeError(capability.message if capability and capability.message else f"Thiết bị đọc chữ {selected} chưa sẵn sàng. Chọn Tự động hoặc CPU.")
    for backend in priority:
        capability = capabilities.get(backend)
        if capability and capability.available:
            return capability
    raise RuntimeError("Không có thiết bị đọc chữ sẵn sàng. Hãy kiểm tra Cài đặt ứng dụng.")
