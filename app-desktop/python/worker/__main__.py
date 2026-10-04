from __future__ import annotations
import os
from pathlib import Path
from jc_worker_protocol import serve, sanitize_unicode as _sanitize_unicode
from worker._version import __version__

_gpu_dll_directory = None
_gpu_preloaded = False

def preload_gpu() -> None:
    global _gpu_dll_directory, _gpu_preloaded
    if _gpu_preloaded or os.environ.get("JC_AI_GPU_RUNTIME") != "1" or os.name != "nt":
        return
    # Loading Torch is expensive. Only AI operations requiring this runtime load it.
    import torch
    _gpu_dll_directory = os.add_dll_directory(str(Path(torch.__file__).parent / "lib"))
    import onnxruntime
    if hasattr(onnxruntime, "preload_dlls"):
        onnxruntime.preload_dlls()
    _gpu_preloaded = True

def dispatch(request, emit) -> None:
    operation = request.get("operation")
    if operation in {"inspect", "asr_install", "whisperx_alignment_install", "transcribe"}:
        from worker.asr.service import inspect_environment, install_model, install_whisperx_alignment, transcribe
    if operation in {"tts_inspect", "tts_install", "tts_generate"}:
        from worker.tts.service import inspect_tts, install_tts_model, synthesize
    if operation in {"inspect", "transcribe", "tts_inspect", "tts_generate", "ocr_inspect", "ocr_transcribe", "text_removal_detect_v2", "text_removal_inpaint_v2", "text_removal_process", "text_removal_process_intervals", "text_removal_scan", "dialogue_inspect", "dialogue_separate"}:
        preload_gpu()
    if operation == "inspect":
        emit({"type": "capabilities", **inspect_environment(
            str(request["model"]),
            str(request["cacheDir"]),
            request.get("legacyCacheDir"),
            request.get("alignmentLanguage"),
            str(request.get("engine", "fasterWhisper")),
            str(request.get("device", "cpu")),
        )})
    elif operation == "asr_install":
        if request.get("backend") == "openvino":
            from worker.asr.openvino_models import install_model as install_intel_model
            install_intel_model(str(request["model"]), str(request["cacheDir"]), emit)
        else:
            install_model(str(request["model"]), str(request["cacheDir"]), emit)
    elif operation == "whisperx_alignment_install":
        install_whisperx_alignment(str(request["language"]), str(request["cacheDir"]), emit)
    elif operation == "transcribe":
        transcribe(request, emit)
    elif operation in {"ocr_inspect", "ocr_install", "ocr_transcribe"}:
        # OCR stays optional in development and is loaded only when used.
        from worker.ocr.service import inspect_ocr, install_ocr, transcribe_frames

        if operation == "ocr_inspect":
            emit({"type": "capabilities", **inspect_ocr(str(request["cacheDir"]), str(request.get("model", "tiny")))})
        elif operation == "ocr_install":
            install_ocr(str(request["cacheDir"]), emit, str(request.get("model", "tiny")))
        else:
            transcribe_frames(request, emit)
    elif operation in {"text_removal_detect_v2", "text_removal_inpaint_v2", "text_removal_patch_video"}:
        if operation == "text_removal_detect_v2":
            from worker.text_removal.detection import detect
            emit({"type": "completed", **detect(request, emit)})
        elif operation == "text_removal_inpaint_v2":
            from worker.text_removal.pipeline import process
            emit({"type": "completed", **process(request, emit)})
        else:
            from worker.text_removal.patch_cache import patch_video
            emit({"type": "completed", "output": str(patch_video(Path(request["patchPath"])))})
    elif operation in {"text_removal_process", "text_removal_process_intervals", "text_removal_scan"}:
        from worker.text_removal.service import process_frames, process_intervals, scan_subtitle_intervals

        if operation == "text_removal_scan":
            scan_subtitle_intervals(request, emit)
        elif operation == "text_removal_process_intervals":
            process_intervals(request, emit)
        else:
            process_frames(request, emit)
    elif operation == "tts_inspect":
        emit({"type": "capabilities", **inspect_tts(str(request["cacheDir"]))})
    elif operation == "tts_install":
        install_tts_model(request, emit)
    elif operation == "tts_generate":
        synthesize(request, emit)
    elif operation in {"dialogue_inspect", "dialogue_separate"}:
        from worker.dialogue.service import inspect_runtime, separate_dialogue

        if operation == "dialogue_inspect":
            emit({"type": "capabilities", **inspect_runtime()})
        else:
            separate_dialogue(request, emit)
    else:
        raise ValueError("Unsupported worker operation")

def main() -> int:
    return serve("ai", __version__, dispatch)

if __name__ == "__main__":
    raise SystemExit(main())
