from __future__ import annotations

from typing import Any, Callable

Emitter = Callable[[dict[str, Any]], None]


def progress(emit: Emitter, stage: str, message: str, value: float | None = None) -> None:
    event: dict[str, Any] = {"type": "progress", "job": "transcribe", "stage": stage, "message": message}
    if value is not None:
        event["progress"] = max(0.0, min(1.0, value))
    emit(event)
