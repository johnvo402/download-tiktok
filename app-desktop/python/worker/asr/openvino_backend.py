"""Native OpenVINO GenAI Whisper: local IR, GPU-only, bounded PCM windows."""
from __future__ import annotations

import logging
from pathlib import Path
import wave

from worker.asr.backends import normalize_segments

LOGGER = logging.getLogger(__name__)


def wav_duration(path: Path) -> float:
    with wave.open(str(path), "rb") as audio:
        if audio.getframerate() != 16000 or audio.getnchannels() != 1 or audio.getsampwidth() != 2 or audio.getcomptype() != "NONE":
            raise ValueError("Audio nhận dạng phải là WAV 16 kHz mono PCM 16-bit.")
        return audio.getnframes() / 16000


class OpenVinoWhisperBackend:
    word_timestamps = False

    def __init__(self, model_dir: Path, compiled_cache: Path | None = None, pipeline_factory=None, pcm_decoder=None):
        self.model_dir = model_dir
        self.compiled_cache = compiled_cache
        self.pipeline_factory = pipeline_factory
        self.pcm_decoder = pcm_decoder
        self.pipeline = None
        self.device = "GPU"

    def load(self):
        from worker.ocr.backends import probe_openvino_gpu
        capability = probe_openvino_gpu()
        if not capability.available:
            raise RuntimeError(capability.message)
        self.device = capability.device or "GPU"
        try:
            if self.pipeline_factory is None:
                import openvino_genai
                factory = openvino_genai.ASRPipeline
            else:
                factory = self.pipeline_factory
            options = {"PERFORMANCE_HINT": "LATENCY"}
            if self.compiled_cache:
                self.compiled_cache.mkdir(parents=True, exist_ok=True)
                options["CACHE_DIR"] = str(self.compiled_cache)
            # No AUTO/HETERO/CPU fallback. Encoder and decoder explicitly compile
            # for the device that passed the real GPU arithmetic probe.
            self.pipeline = factory(str(self.model_dir), self.device, **options)
            LOGGER.info("Whisper OpenVINO loaded: device=%s; model=%s; FP16 IR", self.device, self.model_dir.name)
        except Exception as error:
            LOGGER.exception("OpenVINO Whisper model compilation failed")
            raise RuntimeError("Không thể nạp Whisper vào GPU Intel. Kiểm tra driver Intel hoặc chọn Tự động/CPU.") from error

    def verify(self):
        import numpy as np
        # Model-specific inference after download; not merely an import/probe.
        result = self.pipeline.generate(np.zeros(16000, dtype=np.float32),
            task="transcribe", language="<|en|>", return_timestamps=True, max_new_tokens=16)
        if result is None:
            raise RuntimeError("Whisper Intel không trả kết quả kiểm tra model.")

    def transcribe(self, audio_path, language, on_progress, cancelled=lambda: False):
        duration = wav_duration(Path(audio_path))
        results = []
        detected_language = language
        with wave.open(str(audio_path), "rb") as audio:
            offset = 0.0
            while offset < duration:
                if cancelled():
                    raise InterruptedError("Đã hủy nhận dạng lời thoại.")
                audio.setpos(round(offset * 16000))
                frames = audio.readframes(30 * 16000)
                if self.pcm_decoder:
                    pcm = self.pcm_decoder(frames)
                else:
                    import numpy as np
                    pcm = np.frombuffer(frames, dtype="<i2").astype(np.float32) / 32768.0
                chunk_duration = len(pcm) / 16000
                options = {"return_timestamps": True, "task": "transcribe"}
                if detected_language:
                    options["language"] = f"<|{detected_language}|>"
                try:
                    result = self.pipeline.generate(pcm, **options)
                except InterruptedError:
                    raise
                except Exception as error:
                    LOGGER.exception("Whisper Intel inference failed at audio offset %.2fs", offset)
                    raise RuntimeError("Whisper Intel không nhận dạng được đoạn âm thanh này. Thử lại hoặc chọn Tự động/CPU.") from error
                detected_language = (getattr(result, "languages", None) or [getattr(result, "language", None)])[0] or detected_language
                chunks = result.chunks
                # 2026.4 ASRPipeline returns per-input lists (not the legacy flat API).
                chunks = chunks[0] if chunks and isinstance(chunks[0], (list, tuple)) else chunks or []
                is_last = offset + chunk_duration >= duration - 0.001
                for chunk in chunks:
                    if not str(chunk.text).strip():
                        continue
                    start, end = float(chunk.start_ts), float(chunk.end_ts)
                    # A missing end (-1) must never be invented by splitting raw
                    # text. Reject it rather than saving an inaccurate subtitle.
                    if end < 0:
                        raise RuntimeError("Whisper Intel chưa xác định được thời điểm kết thúc một câu. Thử model lớn hơn hoặc dùng faster-whisper.")
                    local = normalize_segments([{"start": start, "end": end, "text": str(chunk.text)}], chunk_duration)[0]
                    start, end = local["start"], local["end"]
                    midpoint = (start + end) / 2
                    # Each 30s window owns 28s of midpoint space; the 2s overlap
                    # supplies context at the boundary without repeating segments.
                    if offset > 0 and midpoint < 1 or not is_last and midpoint >= 29:
                        continue
                    item = {"start": offset + start, "end": min(offset + end, duration), "text": str(chunk.text)}
                    if results and results[-1]["text"].strip() == item["text"].strip() and item["start"] <= results[-1]["end"] + 0.1:
                        results[-1]["end"] = max(results[-1]["end"], item["end"])
                    else:
                        results.append(item)
                on_progress(min((offset + chunk_duration) / duration, 0.99))
                if is_last:
                    break
                offset += 28
        return normalize_segments(results, duration), detected_language

    def close(self):
        self.pipeline = None
