from __future__ import annotations

import importlib.util
import json
import logging
import os
from pathlib import Path
import time
from typing import Any, Callable

from worker.model_manifest import _sha256, create_manifest, verify_manifest
from worker.tts.onnx_cache import materialize_onnx_files
from worker.tts.providers import VieNeuTTSProvider

log = logging.getLogger(__name__)

MODEL_REPO = "pnnbao-ump/VieNeu-TTS-v3-Turbo"
CODEC_REPO = "OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX"
MANIFEST_NAME = "vieneu-model-manifest.json"


def _onnx_files(model_path: Path, codec_path: Path) -> list[Path]:
    return sorted(
        [path for path in model_path.rglob("*.onnx") if path.is_file()]
        + [path for path in codec_path.rglob("*.onnx") if path.is_file()],
        key=lambda path: path.as_posix(),
    )


def _validate_and_manifest(cache_root: Path, model_path: Path, codec_path: Path) -> tuple[bool, str | None]:
    required_model = model_path / "onnx_update" / "vieneu_prefill.onnx"
    required_codec = codec_path / "moss_audio_tokenizer_decode_full.onnx"
    if not required_model.is_file() or not required_codec.is_file():
        return False, None
    files = _onnx_files(model_path, codec_path)
    if required_model not in files or required_codec not in files:
        return False, None

    identity = f"{MODEL_REPO}@{model_path.name}|{CODEC_REPO}@{codec_path.name}"
    manifest = cache_root / MANIFEST_NAME
    if not manifest.exists():
        # Migrate complete pre-manifest model snapshots. Incomplete downloads
        # cannot pass the required-file checks above and are never marked ready.
        try:
            create_manifest(
                cache_root,
                manifest,
                identity,
                files,
                model_id="vieneu-v3-turbo+moss-audio-tokenizer-onnx",
                version=f"{model_path.name}+{codec_path.name}",
                download=f"https://huggingface.co/{MODEL_REPO} ; https://huggingface.co/{CODEC_REPO}",
            )
        except (OSError, ValueError):
            return False, None
    if not verify_manifest(cache_root, manifest, identity):
        return False, None
    try:
        payload = json.loads(manifest.read_text(encoding="utf-8"))
        recorded = {entry["path"] for entry in payload["files"]}
        cache_root_resolved = cache_root.resolve(strict=True)
        current = {
            path.resolve(strict=True).relative_to(cache_root_resolved).as_posix()
            for path in files
        }
        if current != recorded:
            # Recover a layout migration interrupted after replacing graph
            # symlinks. The old blob manifest must still be valid, and every
            # graph must have identical content before we replace its manifest.
            expected = {(entry["size"], entry["sha256"]) for entry in payload["files"]}
            actual = {(path.stat().st_size, _sha256(path)) for path in files}
            if actual != expected:
                return False, None
        changed = materialize_onnx_files(cache_root, model_path, codec_path)
        if changed or current != recorded:
            create_manifest(
                cache_root, manifest, identity, files,
                model_id="vieneu-v3-turbo+moss-audio-tokenizer-onnx",
                version=f"{model_path.name}+{codec_path.name}",
                download=f"https://huggingface.co/{MODEL_REPO} ; https://huggingface.co/{CODEC_REPO}",
            )
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError):
        return False, None
    return True, f"{model_path.name}+{codec_path.name}"
def emit_progress(emit: Callable[[dict[str, Any]], None], progress: float, message: str) -> None:
    emit({"type": "progress", "job": "tts", "progress": max(0.0, min(1.0, progress)), "message": message})


def _local_model_info(cache_dir: str) -> tuple[bool, str | None]:
    try:
        from huggingface_hub import snapshot_download

        base = Path(cache_dir).resolve()
        os.environ["HF_HOME"] = str(base)
        hub_cache = base / "hub"
        os.environ["HF_HUB_CACHE"] = str(hub_cache)
        model_path = Path(snapshot_download(MODEL_REPO, cache_dir=hub_cache, local_files_only=True))
        codec_path = Path(snapshot_download(CODEC_REPO, cache_dir=hub_cache, local_files_only=True))
        return _validate_and_manifest(base, model_path, codec_path)
    except Exception:
        return False, None


def inspect_tts(cache_dir: str) -> dict[str, Any]:
    installed = importlib.util.find_spec("vieneu") is not None
    voices: list[str] = []
    if installed:
        try:
            import vieneu
            voice_file = Path(vieneu.__file__).parent / "assets" / "voices_v3_turbo.json"
            data = __import__("json").loads(voice_file.read_text(encoding="utf-8"))
            catalog = data.get("presets", {})
            if catalog:
                voices = list(catalog)
        except Exception:
            log.exception("Could not read the installed VieNeu voice catalog")
    model_cached, model_revision = _local_model_info(cache_dir) if installed else (False, None)
    return {
        # Legacy wire name retained for compatibility; this checks the VieNeu SDK,
        # not whether the JSONL worker process itself is running.
        "workerInstalled": installed,
        "modelCached": installed and model_cached,
        "modelRevision": model_revision,
        "cudaAvailable": _cuda_available(),
        "model": "v3-turbo",
        "voices": voices,
        "estimatedDownloadBytes": 1_800_000_000,
        "pythonPackage": "vieneu",
    }


def _cuda_available() -> bool:
    try:
        import torch

        return bool(torch.cuda.is_available())
    except Exception:
        return False


def install_tts_model(request: dict[str, Any], emit: Callable[[dict[str, Any]], None]) -> None:
    base = Path(str(request["cacheDir"])).resolve()
    cache_dir = str(base / "hub")
    os.environ["HF_HOME"] = str(base)
    os.environ["HF_HUB_CACHE"] = cache_dir
    try:
        from huggingface_hub import snapshot_download
    except ImportError as error:
        raise RuntimeError("Thiếu huggingface_hub trong Python runtime nên không thể tải model VieNeu-TTS.") from error

    from tqdm.auto import tqdm as hub_tqdm

    class ProgressTqdm(hub_tqdm):
        def update(self, amount: int = 1) -> None:
            super().update(amount)
            total = self.total or 0
            if total > 0:
                emit_progress(emit, min(0.99, (self.n / total) * 0.99), "Đang tải model VieNeu-TTS v3 Turbo…")

    model_snapshot = snapshot_download(MODEL_REPO, cache_dir=cache_dir, tqdm_class=ProgressTqdm)
    emit_progress(emit, 0.99, "Đang tải bộ giải mã audio…")
    codec_snapshot = snapshot_download(CODEC_REPO, cache_dir=cache_dir, tqdm_class=ProgressTqdm)
    complete, _revision = _validate_and_manifest(base, Path(model_snapshot), Path(codec_snapshot))
    if not complete:
        raise RuntimeError("Các file model VieNeu/MOSS chưa hoàn chỉnh hoặc không vượt qua kiểm tra integrity.")
    emit_progress(emit, 1.0, "Model VieNeu-TTS đã sẵn sàng.")
    emit({"type": "completed", "job": "tts_install", "model": "v3-turbo", "modelRevision": f"{Path(model_snapshot).name}+{Path(codec_snapshot).name}"})


def synthesize(request: dict[str, Any], emit: Callable[[dict[str, Any]], None]) -> None:
    if importlib.util.find_spec("vieneu") is None:
        raise RuntimeError("Chưa cài VieNeu-TTS SDK trong Python runtime.")
    os.environ["HF_HOME"] = str(Path(str(request["cacheDir"])).resolve())
    os.environ["HF_HUB_CACHE"] = str(Path(str(request["cacheDir"])).resolve() / "hub")
    device = str(request.get("device", "auto"))
    if device == "cuda" and not _cuda_available():
        raise RuntimeError("CUDA chưa khả dụng hoặc chưa cài PyTorch CUDA. Chọn Auto hoặc CPU.")
    if not _local_model_info(str(request["cacheDir"]))[0]:
        raise RuntimeError("Model VieNeu-TTS chưa sẵn sàng hoặc không vượt qua kiểm tra integrity. Hãy tải model trong Cài đặt giọng đọc.")
    tts = VieNeuTTSProvider(device)
    items = request.get("segments", [])
    if not isinstance(items, list) or not items:
        raise ValueError("Không có segment cần tạo giọng đọc.")
    total = len(items)
    for index, item in enumerate(items):
        segment_id = str(item["id"])
        text = str(item["text"]).strip()
        if not text:
            emit({"type": "segment_failed", "id": segment_id, "message": "Bản dịch đang trống."})
            continue
        output = Path(str(item["outputPath"])).resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        started = time.monotonic()
        emit({"type": "segment_started", "id": segment_id, "index": index + 1, "total": total})
        try:
            reference = item.get("referenceAudio")
            audio = tts.synthesize(text, str(item.get("voice") or "Ngọc Huyền"), str(reference) if reference else None)
            tts.save(audio, str(output))
            duration = len(audio) / float(tts.sample_rate)
            emit({
                "type": "segment_completed", "id": segment_id,
                "outputPath": str(output), "duration": duration,
                "elapsedSeconds": time.monotonic() - started,
            })
        except Exception as error:
            log.exception("VieNeu synthesis failed for segment %s", segment_id)
            output.unlink(missing_ok=True)
            emit({"type": "segment_failed", "id": segment_id, "message": str(error)[:500]})
        emit_progress(emit, (index + 1) / total, f"Đã xử lý {index + 1}/{total} segment")
    emit({"type": "completed", "job": "tts_generate"})
