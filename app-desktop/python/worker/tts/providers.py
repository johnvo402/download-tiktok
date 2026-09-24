from __future__ import annotations

from pathlib import Path
from typing import Any, Protocol


class TTSProvider(Protocol):
    """Small engine boundary so hosted or alternative local TTS can be added later."""

    @property
    def sample_rate(self) -> int: ...

    def synthesize(self, text: str, voice: str, reference_audio: str | None = None) -> Any: ...

    def save(self, audio: Any, path: str) -> None: ...


class VieNeuTTSProvider:
    def __init__(self, device: str = "auto") -> None:
        from vieneu import Vieneu

        if device == "cpu":
            self._engine = Vieneu(device="cpu", backend="onnx")
        elif device == "cuda":
            self._engine = Vieneu(device="cuda", backend="pytorch")
        else:
            self._engine = Vieneu()

    @property
    def sample_rate(self) -> int:
        return int(getattr(self._engine, "sample_rate", 48_000))

    def synthesize(self, text: str, voice: str, reference_audio: str | None = None) -> Any:
        if reference_audio:
            return self._engine.infer(text, ref_audio=str(Path(reference_audio).resolve()), denoise=True)
        return self._engine.infer(text, voice=voice)

    def save(self, audio: Any, path: str) -> None:
        self._engine.save(audio, path)
