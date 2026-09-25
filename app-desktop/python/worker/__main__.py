from __future__ import annotations

import json
import logging
import sys
import traceback

for stream in (sys.stdin, sys.stdout, sys.stderr):
    try:
        # Rust sends request JSON over stdin as UTF-8. Explicitly configure
        # stdin as well as output streams; Windows' legacy code page turns
        # names such as "Adam bựa" into mojibake otherwise.
        errors = "strict" if stream is sys.stdin else "backslashreplace"
        stream.reconfigure(encoding="utf-8", errors=errors)
    except (AttributeError, OSError):
        pass

from worker.asr.service import inspect_environment, install_model, install_whisperx_alignment, transcribe
from worker.tts.service import inspect_tts, install_tts_model, synthesize

logging.basicConfig(stream=sys.stderr, level=logging.INFO)


def _sanitize_unicode(value: object) -> object:
    """Replace isolated UTF-16 surrogates before they reach the JSONL protocol.

    Model runtimes occasionally return strings containing lone surrogates. JSON
    permits escaped surrogate pairs only, and Rust's serde_json correctly rejects
    a lone one. Keep valid pairs as their Unicode scalar value and replace only
    malformed code units so normal Vietnamese and emoji remain unchanged.
    """
    if isinstance(value, str):
        output: list[str] = []
        index = 0
        while index < len(value):
            codepoint = ord(value[index])
            if 0xD800 <= codepoint <= 0xDBFF:
                if index + 1 < len(value):
                    following = ord(value[index + 1])
                    if 0xDC00 <= following <= 0xDFFF:
                        output.append(chr(0x10000 + ((codepoint - 0xD800) << 10) + (following - 0xDC00)))
                        index += 2
                        continue
                output.append("\uFFFD")
            elif 0xDC00 <= codepoint <= 0xDFFF:
                output.append("\uFFFD")
            else:
                output.append(value[index])
            index += 1
        return "".join(output)
    if isinstance(value, list):
        return [_sanitize_unicode(item) for item in value]
    if isinstance(value, dict):
        return {
            _sanitize_unicode(key): _sanitize_unicode(item)
            for key, item in value.items()
        }
    return value


def emit(message: dict[str, object]) -> None:
    safe_message = _sanitize_unicode(message)
    sys.stdout.write(json.dumps(safe_message, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def main() -> int:
    for raw in sys.stdin:
        try:
            request = _sanitize_unicode(json.loads(raw))
            operation = request.get("operation")
            if operation == "inspect":
                emit({"type": "capabilities", **inspect_environment(
                    str(request["model"]),
                    str(request["cacheDir"]),
                    request.get("legacyCacheDir"),
                    request.get("alignmentLanguage"),
                )})
            elif operation == "asr_install":
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
            elif operation == "tiktok_drama_download":
                # TikTok scraping dependencies are loaded only for this operation.
                # A missing downloader package must not take ASR/TTS offline.
                from worker.tiktok_drama.service import download_drama

                download_drama(request, emit)
            else:
                raise ValueError("Unsupported worker operation")
        except Exception as error:  # worker errors are returned as protocol messages
            logging.error("Worker request failed: %s\n%s", error, traceback.format_exc())
            emit({"type": "failed", "code": "WORKER_ERROR", "message": str(error)})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
