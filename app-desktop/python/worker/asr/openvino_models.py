"""Backend-aware, pinned IR artifacts; only explicit installation uses network."""
from __future__ import annotations

import json
import logging
from pathlib import Path
import shutil

from worker.model_manifest import create_manifest, verify_manifest, _sha256

MANIFEST = "integrity-manifest.json"
ARTIFACTS = json.loads(Path(__file__).with_name("openvino-models.json").read_text(encoding="utf-8-sig"))
LOGGER = logging.getLogger(__name__)


def artifact(model: str) -> dict:
    if model not in ARTIFACTS:
        raise ValueError("Whisper Intel hiện hỗ trợ tiny, base và small. Chọn model tương ứng hoặc dùng faster-whisper.")
    return ARTIFACTS[model]


def model_directory(cache_dir: str | Path, model: str) -> Path:
    artifact(model)
    # Existing models/fw/CT2 paths are untouched. IR is an app-level sibling.
    return Path(cache_dir).parent / "whisper" / model / "openvino"


def model_ready(cache_dir: str | Path, model: str) -> bool:
    if model not in ARTIFACTS:
        return False
    root = model_directory(cache_dir, model)
    info = artifact(model)
    try:
        marker = json.loads((root / "gpu-verified.json").read_text(encoding="utf-8"))
        manifest = json.loads((root / MANIFEST).read_text(encoding="utf-8"))
        return (not root.is_symlink() and marker.get("revision") == info["revision"]
                and marker.get("schemaVersion") == 1 and marker.get("device", "").startswith("GPU")
                and manifest.get("version") == info["revision"]
                and all((root / file["name"]).is_file() and (root / file["name"]).stat().st_size == file["size"] for file in info["files"])
                and verify_manifest(root, root / MANIFEST, info["repository"]))
    except (OSError, ValueError, TypeError):
        return False


def install_model(model: str, cache_dir: str, emit) -> None:
    from worker.asr.openvino_backend import OpenVinoWhisperBackend
    from worker.asr.service import _replace_directory
    from huggingface_hub import hf_hub_download
    info = artifact(model)
    root = model_directory(cache_dir, model)
    if model_ready(cache_dir, model):
        emit({"type": "completed", "job": "asr_install", "alreadyInstalled": True})
        return
    staging = root.with_name(".openvino.installing")
    if staging.is_symlink() or root.is_symlink():
        raise ValueError("Thư mục model không hợp lệ.")
    staging.mkdir(parents=True, exist_ok=True)
    downloaded = 0
    for file in info["files"]:
        emit({"type": "progress", "job": "asr_install", "progress": downloaded / info["estimatedBytes"],
              "message": f"Whisper Intel {model} · {downloaded // 1048576}/{info['estimatedBytes'] // 1048576} MB · {file['name']}"})
        target = Path(hf_hub_download(info["repository"], file["name"], revision=info["revision"], local_dir=str(staging)))
        if target.stat().st_size != file["size"] or file.get("sha256") and _sha256(target) != file["sha256"]:
            raise RuntimeError("Model Whisper Intel tải về chưa hoàn chỉnh. Kiểm tra mạng và tải lại.")
        downloaded += file["size"]
    emit({"type": "progress", "job": "asr_install", "message": "Đang nạp và kiểm tra Whisper bằng GPU Intel…"})
    backend = OpenVinoWhisperBackend(staging, Path(cache_dir).parent.parent / "runtimes/advanced/compiled-whisper")
    try:
        backend.load()
        backend.verify()
        (staging / "gpu-verified.json").write_text(json.dumps({"schemaVersion": 1, "revision": info["revision"], "device": backend.device}), encoding="utf-8")
    finally:
        backend.close()
    create_manifest(staging, staging / MANIFEST, info["repository"],
                    [staging / file["name"] for file in info["files"]] + [staging / "gpu-verified.json"],
                    model_id=f"whisper-{model}-openvino", version=info["revision"],
                    download=f"https://huggingface.co/{info['repository']}")
    # Download metadata isn't used at runtime; avoid carrying redundant hf cache.
    shutil.rmtree(staging / ".cache", ignore_errors=True)
    _replace_directory(staging, root)
    emit({"type": "completed", "job": "asr_install", "alreadyInstalled": False})


def inspect_model(model: str, cache_dir: str) -> dict:
    from worker.asr.backends import runtime_capabilities
    import importlib.util
    runtime = importlib.util.find_spec("openvino_genai") is not None
    capabilities = runtime_capabilities()
    ready = model_ready(cache_dir, model)
    info = ARTIFACTS.get(model)
    return {"intelRuntimeInstalled": runtime, "intelGpuAvailable": capabilities["openvino"],
            "intelModelSupported": info is not None, "intelModelCached": ready,
            "intelEstimatedDownloadBytes": info["estimatedBytes"] if info else 0,
            "intelModelPath": str(model_directory(cache_dir, model)) if info else None,
            "wordTimestamps": False, "backends": capabilities}
