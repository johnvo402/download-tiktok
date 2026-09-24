from __future__ import annotations

import json
import importlib.util
import logging
import os
import shutil
from difflib import SequenceMatcher
from pathlib import Path
from typing import Any, Callable
import unicodedata
import uuid
import wave

from worker.model_manifest import create_manifest, verify_manifest
from worker.protocol import Emitter, progress

LOGGER = logging.getLogger(__name__)
SUPPORTED_MODELS = {"tiny", "base", "small", "medium", "large-v3"}
SUPPORTED_ALIGNMENT_LANGUAGES = {"en", "vi", "ja", "ko", "zh", "fr", "de", "es"}
MODEL_REPOSITORIES = {model: f"Systran/faster-whisper-{model}" for model in SUPPORTED_MODELS}
MODEL_FILES = ("config.json", "model.bin", "tokenizer.json")
OPTIONAL_MODEL_FILES = ("preprocessor_config.json",)
MANIFEST_NAME = "integrity-manifest.json"


def _model_directory(cache_dir: str | Path, model_name: str) -> Path:
    if model_name not in SUPPORTED_MODELS:
        raise ValueError(f"Unsupported faster-whisper model: {model_name}")
    return Path(cache_dir) / model_name


def _required_files_complete(model_dir: Path) -> bool:
    try:
        return model_dir.is_dir() and not model_dir.is_symlink() and all(
            (model_dir / filename).is_file() and (model_dir / filename).stat().st_size > 0
            for filename in MODEL_FILES
        ) and any(
            path.is_file() and path.stat().st_size > 0
            for path in model_dir.glob("vocabulary.*")
        )
    except OSError:
        return False


def _model_files(model_dir: Path) -> list[Path]:
    files = [model_dir / filename for filename in MODEL_FILES]
    files.extend(
        model_dir / filename
        for filename in OPTIONAL_MODEL_FILES
        if (model_dir / filename).is_file()
    )
    files.extend(path for path in model_dir.glob("vocabulary.*") if path.is_file())
    return files


def _model_is_complete(model_dir: Path, model_name: str | None = None) -> bool:
    if not _required_files_complete(model_dir):
        return False
    manifest = model_dir / MANIFEST_NAME
    files = _model_files(model_dir)
    identity = MODEL_REPOSITORIES.get(model_name or model_dir.name)
    if not identity:
        return False
    # Migrate pre-manifest installations only when all required model files
    # are present. Never overwrite an existing invalid manifest silently.
    if not manifest.exists():
        try:
            create_manifest(
                model_dir,
                manifest,
                identity,
                files,
                model_id=f"faster-whisper-{model_name or model_dir.name}",
                version="main",
                download=f"https://huggingface.co/{identity}",
            )
        except (OSError, ValueError):
            return False
    return verify_manifest(model_dir, manifest, identity)


def _recover_staging_install(model_name: str, cache_dir: Path) -> bool:
    """Promote a complete interrupted download without downloading it again."""
    staging_dir = cache_dir / f".{model_name}.installing"
    model_dir = _model_directory(cache_dir, model_name)
    if staging_dir.is_symlink() or not staging_dir.is_dir():
        return False
    if not _model_is_complete(staging_dir, model_name):
        return False
    _replace_directory(staging_dir, model_dir)
    return _model_is_complete(model_dir, model_name)


def _replace_directory(staging_dir: Path, model_dir: Path) -> None:
    if model_dir.is_symlink():
        raise RuntimeError("Model cache path is a symlink; refusing to replace it")
    backup = model_dir.with_name(f".{model_dir.name}.previous-{uuid.uuid4().hex}")
    moved_old = False
    try:
        if model_dir.exists():
            model_dir.replace(backup)
            moved_old = True
        staging_dir.replace(model_dir)
    except Exception:
        if moved_old and backup.exists() and not model_dir.exists():
            backup.replace(model_dir)
        raise
    if moved_old:
        shutil.rmtree(backup, ignore_errors=True)


def _migrate_legacy_cache(model_name: str, legacy_cache_dir: str | None, cache_dir: Path) -> bool:
    if not legacy_cache_dir or not Path(legacy_cache_dir).is_dir():
        return False
    model_dir = _model_directory(cache_dir, model_name)
    staging_dir = cache_dir / f".{model_name}.installing"
    try:
        from huggingface_hub import snapshot_download

        snapshot = Path(snapshot_download(
            repo_id=MODEL_REPOSITORIES[model_name],
            cache_dir=legacy_cache_dir,
            local_files_only=True,
        ))
        if not _required_files_complete(snapshot):
            return False
        staging_dir.mkdir(parents=True, exist_ok=True)
        for source in snapshot.iterdir():
            if source.is_file() and (
                source.name in MODEL_FILES
                or source.name in OPTIONAL_MODEL_FILES
                or source.name.startswith("vocabulary.")
            ):
                shutil.copy2(source, staging_dir / source.name)
        if not _required_files_complete(staging_dir):
            shutil.rmtree(staging_dir, ignore_errors=True)
            return False
        create_manifest(
            staging_dir,
            staging_dir / MANIFEST_NAME,
            MODEL_REPOSITORIES[model_name],
            _model_files(staging_dir),
            model_id=f"faster-whisper-{model_name}",
            version="main",
            download=f"https://huggingface.co/{MODEL_REPOSITORIES[model_name]}",
        )
        _replace_directory(staging_dir, model_dir)
        return True
    except Exception:
        # A legacy cache miss or an interrupted .incomplete download is not a
        # fatal inspect error. Settings can install a clean copy explicitly.
        return False


def _alignment_language_directory(cache_dir: str | Path, language: str) -> Path:
    if language not in SUPPORTED_ALIGNMENT_LANGUAGES:
        raise ValueError("WhisperX alignment language is not supported")
    return Path(cache_dir) / language


def _alignment_cache_is_complete(cache_dir: str | Path, language: str) -> bool:
    try:
        language_dir = _alignment_language_directory(cache_dir, language)
        marker = language_dir / "alignment-installed.json"
        if language_dir.is_symlink() or marker.is_symlink() or not marker.is_file():
            return False
        payload = json.loads(marker.read_text(encoding="utf-8"))
        if payload.get("schemaVersion") != 1 or payload.get("language") != language:
            return False
        # WhisperX uses Hugging Face snapshots for most aligners and Torch Hub
        # checkpoints for a few. Both are persisted below model_dir.
        has_alignment_weights = any(
            path.is_file() and path.suffix.lower() in {".bin", ".safetensors", ".pt", ".pth", ".ckpt"}
            for path in language_dir.rglob("*")
        )
        has_nltk_tokenizer = (Path(cache_dir) / "nltk_data" / "tokenizers" / "punkt_tab").is_dir()
        return has_alignment_weights and has_nltk_tokenizer
    except (OSError, ValueError, TypeError, json.JSONDecodeError):
        return False


def inspect_environment(
    model_name: str,
    cache_dir: str,
    legacy_cache_dir: str | None = None,
    alignment_language: str | None = None,
) -> dict[str, Any]:
    if model_name not in SUPPORTED_MODELS:
        raise ValueError(f"Unsupported faster-whisper model: {model_name}")
    root = Path(cache_dir)
    root.mkdir(parents=True, exist_ok=True)
    installed = False
    try:
        import faster_whisper  # noqa: F401
        installed = True
    except ImportError:
        pass

    model_dir = _model_directory(root, model_name)
    cached = _model_is_complete(model_dir)
    if installed and not cached:
        cached = _recover_staging_install(model_name, root)
    if installed and not cached:
        # snapshot_download may have completed its shared Hugging Face cache
        # while the app-local staging directory was interrupted.
        cached = _migrate_legacy_cache(model_name, str(root), root)
    if installed and not cached and legacy_cache_dir != str(root):
        cached = _migrate_legacy_cache(model_name, legacy_cache_dir, root)

    cuda_available = False
    if installed:
        try:
            import ctranslate2

            cuda_available = ctranslate2.get_cuda_device_count() > 0
        except Exception:
            cuda_available = False

    whisperx_installed = importlib.util.find_spec("whisperx") is not None
    alignment_cached = bool(
        whisperx_installed
        and alignment_language
        and alignment_language in SUPPORTED_ALIGNMENT_LANGUAGES
        and _alignment_cache_is_complete(Path(cache_dir) / "whisperx", alignment_language)
    )

    return {
        "workerInstalled": installed,
        "modelCached": cached,
        "cudaAvailable": cuda_available,
        "whisperXInstalled": whisperx_installed,
        "alignmentModelCached": alignment_cached,
        "alignmentLanguage": alignment_language,
        "modelPath": str(model_dir),
    }


def install_whisperx_alignment(language: str, cache_dir: str, emit: Emitter) -> None:
    language_dir = _alignment_language_directory(cache_dir, language)
    if _alignment_cache_is_complete(cache_dir, language):
        emit({"type": "completed", "job": "whisperx_install", "language": language, "alreadyInstalled": True})
        return
    try:
        import nltk
        import whisperx
    except ImportError as error:
        raise RuntimeError(
            "WhisperX chưa được cài trong Python Worker. Trong development, chạy scripts/setup-whisperx-dev.ps1; bản release cần worker có WhisperX."
        ) from error

    language_dir.mkdir(parents=True, exist_ok=True)
    nltk_dir = Path(cache_dir) / "nltk_data"
    nltk_dir.mkdir(parents=True, exist_ok=True)
    emit({"type": "progress", "job": "whisperx_install", "message": f"Đang tải aligner WhisperX cho ngôn ngữ {language}…"})
    try:
        # Setup is the only operation allowed to fetch these weights. Runtime
        # transcription uses model_cache_only=True and offline mode.
        align_model, _metadata = whisperx.load_align_model(
            language_code=language,
            device="cpu",
            model_dir=str(language_dir),
            model_cache_only=False,
        )
        del align_model
        import gc
        gc.collect()
        nltk.data.path.insert(0, str(nltk_dir))
        punkt = nltk_dir / "tokenizers" / "punkt_tab"
        if not punkt.exists() and not nltk.download("punkt_tab", download_dir=str(nltk_dir), quiet=True):
            raise RuntimeError("Không tải được dữ liệu NLTK punkt_tab cần cho WhisperX.")
        if not any(
            path.is_file() and path.suffix.lower() in {".bin", ".safetensors", ".pt", ".pth", ".ckpt"}
            for path in language_dir.rglob("*")
        ):
            raise RuntimeError("WhisperX không tạo được file trọng số aligner hợp lệ.")
        marker = language_dir / "alignment-installed.json"
        temporary = marker.with_suffix(".json.tmp")
        temporary.write_text(
            json.dumps({"schemaVersion": 1, "language": language}, ensure_ascii=False),
            encoding="utf-8",
        )
        temporary.replace(marker)
    except Exception:
        # Leave downloaded cache files so Settings can resume/retry, but never
        # claim the model is ready until both weights and required NLTK data exist.
        raise
    emit({"type": "completed", "job": "whisperx_install", "language": language, "alreadyInstalled": False})


def install_model(model_name: str, cache_dir: str, emit: Emitter) -> None:
    if model_name not in SUPPORTED_MODELS:
        raise ValueError(f"Unsupported faster-whisper model: {model_name}")
    try:
        from huggingface_hub import snapshot_download
    except ImportError as error:
        raise RuntimeError("AI Worker không có thành phần faster-whisper cần thiết. Hãy sửa chữa hoặc cài lại ứng dụng.") from error

    root = Path(cache_dir)
    root.mkdir(parents=True, exist_ok=True)
    model_dir = _model_directory(root, model_name)
    if _model_is_complete(model_dir):
        emit({"type": "completed", "job": "asr_install", "model": model_name, "alreadyInstalled": True})
        return

    # Keep an incomplete staging directory so Hugging Face can resume a failed
    # setup from Settings. Generate never reads or downloads from this folder.
    staging_dir = root / f".{model_name}.installing"
    staging_dir.mkdir(parents=True, exist_ok=True)
    emit({"type": "progress", "job": "asr_install", "message": f"Downloading faster-whisper {model_name}"})
    snapshot_download(
        repo_id=MODEL_REPOSITORIES[model_name],
        local_dir=str(staging_dir),
        cache_dir=str(root),
        allow_patterns=[
            "config.json",
            "preprocessor_config.json",
            "model.bin",
            "tokenizer.json",
            "vocabulary.*",
        ],
    )
    if not _required_files_complete(staging_dir):
        raise RuntimeError("Model tải về chưa hoàn chỉnh. Hãy kiểm tra dung lượng trống và thử lại trong Settings.")

    create_manifest(
        staging_dir,
        staging_dir / MANIFEST_NAME,
        MODEL_REPOSITORIES[model_name],
        _model_files(staging_dir),
        model_id=f"faster-whisper-{model_name}",
        version="main",
        download=f"https://huggingface.co/{MODEL_REPOSITORIES[model_name]}",
    )
    _replace_directory(staging_dir, model_dir)
    emit({"type": "completed", "job": "asr_install", "model": model_name, "alreadyInstalled": False})


def _audio_channel_plan(audio_path: Path) -> tuple[int, int, Any | None, list[int]]:
    """Inspect the extracted PCM WAV and retain distinct stereo channels for ASR."""
    with wave.open(str(audio_path), "rb") as source:
        channel_count = source.getnchannels()
        sample_rate = source.getframerate()
        frame_count = source.getnframes()
        sample_width = source.getsampwidth()
        if sample_width != 2:
            raise RuntimeError("Audio nhận dạng phải là PCM 16-bit.")
        if channel_count == 1:
            return sample_rate, frame_count, None, [0]
        if channel_count != 2:
            raise RuntimeError(f"Audio nhận dạng có số kênh không hỗ trợ: {channel_count}.")

        raw_frames = source.readframes(frame_count)

    try:
        import numpy as np
    except ImportError as error:
        raise RuntimeError("AI Worker thiếu NumPy cần thiết để nhận dạng audio stereo.") from error

    samples = np.frombuffer(raw_frames, dtype="<i2")
    if samples.size != frame_count * channel_count:
        raise RuntimeError("Audio WAV bị thiếu hoặc hỏng dữ liệu frame.")
    samples = samples.reshape((frame_count, channel_count))
    if frame_count == 0:
        return sample_rate, frame_count, samples, [0]

    # A short 10 ms sample is enough to tell duplicated mono from genuinely
    # different/panned channels without scanning an hour-long file at full rate.
    stride = max(1, sample_rate // 100)
    probe = samples[::stride].astype(np.float64)
    left = probe[:, 0] - float(np.mean(probe[:, 0]))
    right = probe[:, 1] - float(np.mean(probe[:, 1]))
    left_rms = float(np.sqrt(np.mean(left * left)))
    right_rms = float(np.sqrt(np.mean(right * right)))
    strongest_rms = max(left_rms, right_rms)
    if strongest_rms < 80.0:
        return sample_rate, frame_count, samples, [0]
    if min(left_rms, right_rms) < 80.0:
        channel = 0 if left_rms > right_rms else 1
        return sample_rate, frame_count, samples, [channel]

    correlation_denominator = float(np.sqrt(np.sum(left * left) * np.sum(right * right)))
    correlation = (
        float(np.sum(left * right) / correlation_denominator)
        if correlation_denominator > 0
        else 1.0
    )
    level_delta = abs(left_rms - right_rms) / strongest_rms
    difference_rms = float(np.sqrt(np.mean((left - right) ** 2))) / strongest_rms

    # Avoid doubling work for ordinary dual-mono/stereo mixes. Independently
    # decode channels when their content or voice balance is meaningfully different.
    distinct_channels = correlation < 0.985 or level_delta > 0.18 or difference_rms > 0.16
    return sample_rate, frame_count, samples, [0, 1] if distinct_channels else [0]


def _transcribe_channel(
    model: Any,
    audio: Any,
    language: str | None,
    on_progress: Callable[[float], None] | None = None,
) -> tuple[list[dict[str, Any]], str | None]:
    segments, info = model.transcribe(
        audio,
        beam_size=5,
        language=language,
    )
    results: list[dict[str, Any]] = []
    for segment in segments:
        text = str(segment.text)
        if not text.strip():
            continue
        try:
            confidence = float(segment.avg_logprob)
        except (AttributeError, TypeError, ValueError):
            confidence = -10.0
        results.append({
            "start": float(segment.start),
            "end": float(segment.end),
            "text": text,
            "_confidence": confidence,
        })
        if on_progress:
            try:
                duration = float(getattr(info, "duration", 0.0) or 0.0)
            except (TypeError, ValueError):
                duration = 0.0
            if duration > 0:
                on_progress(min(float(segment.end) / duration, 0.99))
    return results, getattr(info, "language", None)


def _align_channel_segments(
    whisperx: Any,
    align_model: Any,
    align_metadata: dict[str, Any],
    segments: list[dict[str, Any]],
    audio: Any,
    device: str,
) -> list[dict[str, Any]]:
    if not segments:
        return segments
    align_input = [
        {"start": float(segment["start"]), "end": float(segment["end"]), "text": str(segment["text"])}
        for segment in segments
    ]
    aligned = whisperx.align(
        align_input,
        align_model,
        align_metadata,
        audio,
        device,
        return_char_alignments=False,
    )
    aligned_segments = aligned.get("segments", [])
    if len(aligned_segments) != len(segments):
        raise RuntimeError("WhisperX trả về số segment sau alignment không khớp transcript.")

    output: list[dict[str, Any]] = []
    for original, aligned_segment in zip(segments, aligned_segments):
        result = dict(original)
        words = aligned_segment.get("words") or []
        word_starts = [float(word["start"]) for word in words if word.get("start") is not None]
        word_ends = [float(word["end"]) for word in words if word.get("end") is not None]
        if word_starts and word_ends:
            start = min(word_starts)
            end = max(word_ends)
            if 0 <= start < end:
                result["start"] = start
                result["end"] = end
        output.append(result)
    return output


def _normalized_segment_text(value: str) -> str:
    normalized = unicodedata.normalize("NFKC", value).casefold()
    return "".join(character for character in normalized if character.isalnum())


def _same_channel_segment(left: dict[str, Any], right: dict[str, Any]) -> bool:
    left_text = _normalized_segment_text(str(left["text"]))
    right_text = _normalized_segment_text(str(right["text"]))
    if not left_text or not right_text:
        return False
    similarity = SequenceMatcher(None, left_text, right_text, autojunk=False).ratio()
    if similarity < 0.94:
        return False

    overlap = max(0.0, min(float(left["end"]), float(right["end"])) - max(float(left["start"]), float(right["start"])))
    shorter_duration = max(0.1, min(
        float(left["end"]) - float(left["start"]),
        float(right["end"]) - float(right["start"]),
    ))
    return overlap / shorter_duration >= 0.55 or (
        abs(float(left["start"]) - float(right["start"])) <= 0.35
        and abs(float(left["end"]) - float(right["end"])) <= 1.0
    )


def _merge_channel_segments(channel_results: list[list[dict[str, Any]]]) -> list[dict[str, Any]]:
    candidates = sorted(
        (segment for results in channel_results for segment in results),
        key=lambda segment: (float(segment["start"]), float(segment["end"])),
    )
    merged: list[dict[str, Any]] = []
    for candidate in candidates:
        duplicate_index: int | None = None
        for index in range(len(merged) - 1, -1, -1):
            previous = merged[index]
            if float(previous["start"]) < float(candidate["start"]) - 30.0:
                break
            if _same_channel_segment(previous, candidate):
                duplicate_index = index
                break
        if duplicate_index is None:
            merged.append(candidate)
        elif float(candidate["_confidence"]) > float(merged[duplicate_index]["_confidence"]):
            merged[duplicate_index] = candidate

    for segment in merged:
        segment.pop("_confidence", None)
    return sorted(merged, key=lambda segment: (float(segment["start"]), float(segment["end"])))


def transcribe(request: dict[str, Any], emit: Emitter) -> None:
    engine = str(request.get("engine", "fasterWhisper"))
    model_name = str(request.get("model", "small"))
    device = str(request.get("device", "auto"))
    language = request.get("language")
    audio_path = Path(str(request["audioPath"]))
    output_path = Path(str(request["outputPath"]))
    cache_dir = Path(str(request["cacheDir"]))
    whisperx_cache_dir = Path(str(request.get("whisperxCacheDir", cache_dir / "whisperx")))

    if engine not in {"fasterWhisper", "whisperX"}:
        raise ValueError("Unsupported Speech Recognition engine")
    if model_name not in SUPPORTED_MODELS:
        raise ValueError(f"Unsupported faster-whisper model: {model_name}")
    if device not in {"auto", "cpu", "cuda"}:
        raise ValueError(f"Unsupported device: {device}")
    if not audio_path.is_file():
        raise FileNotFoundError("Extracted WAV audio was not found")
    if not language or language == "auto":
        language = None
    if engine == "whisperX":
        if not language or language not in SUPPORTED_ALIGNMENT_LANGUAGES:
            raise RuntimeError("WhisperX cần chọn ngôn ngữ cụ thể trong Cài đặt → Speech Recognition để dùng đúng aligner đã cài.")
        if not _alignment_cache_is_complete(whisperx_cache_dir, language):
            raise RuntimeError(f"WhisperX chưa cài aligner cho ngôn ngữ {language}. Mở Cài đặt → Speech Recognition để cài trước.")
        os.environ["HF_HUB_OFFLINE"] = "1"
        os.environ["TRANSFORMERS_OFFLINE"] = "1"
        os.environ["HF_HOME"] = str(whisperx_cache_dir)
        os.environ["NLTK_DATA"] = str(whisperx_cache_dir / "nltk_data")

    try:
        from faster_whisper import WhisperModel
    except ImportError as error:
        raise RuntimeError("AI Worker không có thành phần faster-whisper cần thiết. Hãy sửa chữa hoặc cài lại ứng dụng.") from error

    model_dir = _model_directory(cache_dir, model_name)
    if not _model_is_complete(model_dir):
        raise RuntimeError(f"Model {model_name} chưa được cài. Mở Cài đặt → Speech Recognition để cài model trước khi tạo transcript.")
    progress(emit, "model-loading", f"Loading {model_name} model")
    LOGGER.info("Loading model %s on %s", model_name, device)
    try:
        compute_type = "default" if device == "auto" else "int8" if device == "cpu" else "float16"
        # Load only from the validated local directory. Passing the model name
        # here would allow faster-whisper to contact Hugging Face on every run.
        model = WhisperModel(str(model_dir), device=device, compute_type=compute_type, local_files_only=True)
    except Exception as error:
        if device == "cuda":
            raise RuntimeError(f"CUDA model initialization failed. Select CPU or Auto in Speech Recognition settings. {error}") from error
        raise

    sample_rate, frame_count, samples, channel_indices = _audio_channel_plan(audio_path)
    if sample_rate <= 0:
        raise RuntimeError("Audio WAV có sample rate không hợp lệ.")
    duration = frame_count / sample_rate
    channel_results: list[list[dict[str, Any]]] = []
    detected_languages: list[str | None] = []
    channel_total = len(channel_indices)
    stereo_separated = samples is not None and channel_total > 1
    for channel_position, channel_index in enumerate(channel_indices):
        if samples is None:
            audio_input: Any = str(audio_path)
        else:
            audio_input = samples[:, channel_index].astype("float32") / 32768.0

        channel_name = ("trái" if channel_index == 0 else "phải") if stereo_separated else None
        message = (
            f"Đang nhận dạng riêng kênh {channel_name} ({channel_position + 1}/{channel_total}) để giữ lời thoại chồng lấn…"
            if stereo_separated
            else "Đang nhận dạng lời thoại…"
        )
        progress(emit, "recognizing", message, channel_position / channel_total)

        def report_channel_progress(fraction: float) -> None:
            overall = (channel_position + fraction) / channel_total
            progress(emit, "recognizing", message, min(overall, 0.99))

        results, detected_language = _transcribe_channel(
            model,
            audio_input,
            language,
            on_progress=report_channel_progress,
        )
        channel_results.append(results)
        detected_languages.append(detected_language)
        del audio_input

    results = _merge_channel_segments(channel_results) if channel_total > 1 else channel_results[0]
    language_counts = [len(segments) for segments in channel_results]
    language_index = max(range(channel_total), key=lambda index: language_counts[index])
    detected_language = detected_languages[language_index]

    if engine == "whisperX":
        if detected_language and detected_language != language:
            raise RuntimeError(
                f"Ngôn ngữ được nhận diện là {detected_language}, khác ngôn ngữ aligner đã cài ({language}). "
                "Hãy chọn đúng ngôn ngữ nguồn trong Settings hoặc chuyển về faster-whisper."
            )
        progress(emit, "aligning", f"WhisperX đang căn chỉnh timestamp từng segment ({language})…", 0.0)
        try:
            import gc
            import nltk
            import torch
            import whisperx
        except ImportError as error:
            raise RuntimeError("WhisperX/ PyTorch chưa được cài trong Python Worker. Hãy cài runtime WhisperX trong Settings hướng dẫn.") from error

        nltk.data.path.insert(0, str(whisperx_cache_dir / "nltk_data"))
        if device == "cuda":
            if not torch.cuda.is_available():
                raise RuntimeError("PyTorch không thấy CUDA dù faster-whisper có CUDA. Chọn CPU hoặc cài PyTorch phù hợp trong môi trường GPU.")
            align_device = "cuda"
        elif device == "auto":
            align_device = "cuda" if torch.cuda.is_available() else "cpu"
        else:
            align_device = "cpu"

        # Release the faster-whisper model before loading the alignment model;
        # both can otherwise compete for scarce CPU memory/VRAM.
        del model
        gc.collect()
        if align_device == "cuda":
            torch.cuda.empty_cache()
        align_model, align_metadata = whisperx.load_align_model(
            language_code=language,
            device=align_device,
            model_dir=str(_alignment_language_directory(whisperx_cache_dir, language)),
            model_cache_only=True,
        )
        if samples is None:
            alignment_audio: Any = whisperx.load_audio(str(audio_path))
            for index, channel_segments in enumerate(channel_results):
                progress(emit, "aligning", "WhisperX đang căn chỉnh timestamp transcript…", index / channel_total)
                channel_results[index] = _align_channel_segments(
                    whisperx, align_model, align_metadata, channel_segments, alignment_audio, align_device
                )
        else:
            for index, channel_index in enumerate(channel_indices):
                progress(emit, "aligning", f"WhisperX đang căn chỉnh kênh {index + 1}/{channel_total}…", index / channel_total)
                alignment_audio = samples[:, channel_index].astype("float32") / 32768.0
                channel_results[index] = _align_channel_segments(
                    whisperx, align_model, align_metadata, channel_results[index], alignment_audio, align_device
                )
        del align_model
        gc.collect()
        if align_device == "cuda":
            torch.cuda.empty_cache()
        results = _merge_channel_segments(channel_results) if channel_total > 1 else channel_results[0]

    if not results:
        raise RuntimeError("Speech recognition returned no transcript segments")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = output_path.with_suffix(output_path.suffix + ".tmp")
    try:
        with temp_path.open("w", encoding="utf-8", newline="\n") as output:
            json.dump({"language": detected_language, "segments": results}, output, ensure_ascii=False, indent=2)
            output.flush()
            os.fsync(output.fileno())
        temp_path.replace(output_path)
    except Exception:
        temp_path.unlink(missing_ok=True)
        raise

    emit({"type": "completed", "job": "transcribe", "output": str(output_path), "segmentCount": len(results), "language": detected_language})
