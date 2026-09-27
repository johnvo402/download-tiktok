from __future__ import annotations

import importlib.util
from pathlib import Path
from typing import Callable


Emitter = Callable[[dict[str, object]], None]
MODEL_SHA256 = "abcfccf65446752a057f4a302c941479a54b7560ebf8d7bca039d2ea98e64cfc"


def inspect_runtime() -> dict[str, object]:
    installed = importlib.util.find_spec("torch") is not None and importlib.util.find_spec("bandit_infer") is not None
    cuda = False
    if installed:
        try:
            import torch
            cuda = bool(torch.cuda.is_available())
        except Exception:
            installed = False
    return {"runtimeInstalled": installed, "cudaAvailable": cuda}


def separate_dialogue(request: dict[str, object], emit: Emitter) -> None:
    if not inspect_runtime()["runtimeInstalled"]:
        raise RuntimeError("Bộ xử lý tách thoại chưa được cài. Mở Cài đặt → Nhận dạng lời thoại và cài BandIt.")

    import numpy as np
    import soundfile as sf
    from bandit_infer import BanditSession

    input_path = Path(str(request["inputPath"])).resolve(strict=True)
    output_path = Path(str(request["outputPath"])).resolve()
    checkpoint = Path(str(request["modelPath"])).resolve(strict=True)
    if input_path.suffix.lower() != ".wav" or output_path.suffix.lower() != ".wav":
        raise ValueError("Audio tách thoại phải là WAV.")
    if not input_path.is_file() or not checkpoint.is_file():
        raise FileNotFoundError("Không tìm thấy audio hoặc model BandIt.")
    if output_path.exists():
        output_path.unlink()

    emit({"type": "progress", "job": "dialogue_separation", "message": "Đang nạp BandIt v2…"})
    audio, sample_rate = sf.read(str(input_path), dtype="float32", always_2d=True)
    if sample_rate != 48000:
        raise ValueError(f"BandIt v2 cần audio 48 kHz; nhận được {sample_rate} Hz.")
    if not np.isfinite(audio).all() or audio.shape[0] == 0:
        raise ValueError("Audio nguồn rỗng hoặc chứa dữ liệu không hợp lệ.")

    emit({"type": "progress", "job": "dialogue_separation", "message": "Đang tách stem thoại, nhạc và hiệu ứng…"})
    with BanditSession(
        "v2-multi",
        device=str(request.get("device", "auto")),
        checkpoint_path=checkpoint,
        checkpoint_sha256=MODEL_SHA256,
    ) as session:
        stems = session.infer(audio.T, sample_rate=sample_rate)

    speech = stems.get("speech")
    if speech is None:
        raise RuntimeError("BandIt không trả về stem thoại.")
    speech = np.asarray(speech, dtype=np.float32)
    if speech.ndim == 1:
        speech = speech[:, None]
    elif speech.ndim == 2:
        speech = speech.T
    if speech.shape[0] == 0 or not np.isfinite(speech).all():
        raise RuntimeError("BandIt trả về stem thoại không hợp lệ.")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(output_path), speech, sample_rate, subtype="PCM_16")
    if not output_path.is_file() or output_path.stat().st_size <= 44:
        raise RuntimeError("Không tạo được file stem thoại.")
    emit({"type": "completed", "job": "dialogue_separation", "output": str(output_path)})
