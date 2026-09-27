"""Fingerprint and validate a frozen worker. Uses only the Python standard library."""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = "worker-build.json"


def fingerprint(root: Path, interpreter: str | None = None) -> str:
    identity = interpreter or f"{sys.implementation.name}-{sys.version}-{sys.platform}-{platform.machine()}"
    files = [root / "python/pyproject.toml", root / "scripts/build-worker.ps1", root / "scripts/worker-cache.py"]
    files += list((root / "python").glob("requirements-release.*"))
    for directory in (root / "python/worker", root / "python/packaging"):
        files += [file for file in directory.rglob("*") if file.is_file() and "__pycache__" not in file.parts and file.suffix != ".pyc"]
    digest = hashlib.sha256(f"jc-worker-v1:{identity}".encode())
    for file in sorted(set(files)):
        digest.update(file.relative_to(root).as_posix().encode() + b"\0")
        digest.update(file.read_bytes() + b"\0")
    return digest.hexdigest()


def inventory(directory: Path) -> dict[str, str]:
    files = {}
    for file in sorted(directory.rglob("*")):
        if file.is_symlink():
            raise ValueError("A worker cache must not contain symlinks.")
        if file.is_file() and file.name != MANIFEST:
            with file.open("rb") as stream:
                files[file.relative_to(directory).as_posix()] = hashlib.file_digest(stream, "sha256").hexdigest()
    required = {"ai-worker.exe", "worker-version.txt", "python-packages.json", "THIRD_PARTY_NOTICES.txt"}
    if not required <= files.keys():
        raise ValueError("The worker cache is incomplete.")
    return files


def verify(directory: Path, key: str) -> bool:
    try:
        manifest = json.loads((directory / MANIFEST).read_text(encoding="utf-8"))
        return manifest["fingerprint"] == key and manifest["files"] == inventory(directory)
    except (OSError, ValueError, KeyError):
        return False


def smoke(directory: Path) -> None:
    executable = directory / "ai-worker.exe"
    version = subprocess.run([str(executable), "--version"], capture_output=True, timeout=60, check=True)
    expected = (directory / "worker-version.txt").read_text().strip()
    if version.stdout.decode("utf-8").strip() != expected:
        raise ValueError("Packaged worker version does not match its inventory.")
    # Inspect capabilities with an isolated, empty model directory. Never downloads models.
    with tempfile.TemporaryDirectory(prefix="jc-worker-smoke-") as temporary:
        requests = [{"operation": "inspect", "model": "small", "cacheDir": temporary},
                    {"operation": "tts_inspect", "cacheDir": temporary},
                    {"operation": "ocr_inspect", "cacheDir": temporary}]
        result = subprocess.run([str(executable)], input="".join(json.dumps(item) + "\n" for item in requests),
                                capture_output=True, encoding="utf-8", timeout=120, check=True)
        responses = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        if len(responses) != len(requests) or any(item.get("type") != "capabilities" for item in responses):
            raise ValueError("Packaged worker did not return the expected capability protocol.")
        if not (responses[0].get("workerInstalled") and responses[1].get("workerInstalled")
                and responses[2].get("ocrInstalled") and responses[2].get("runtimeInstalled")):
            raise ValueError("Packaged worker is missing a required ASR, TTS or OCR dependency.")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", choices=["key", "verify", "seal", "smoke"])
    parser.add_argument("--directory", type=Path, default=ROOT / "src-tauri/resources/ai-worker")
    args = parser.parse_args()
    key = fingerprint(ROOT)
    if args.operation == "key":
        print(key)
    elif args.operation == "verify":
        return 0 if verify(args.directory, key) else 1
    elif args.operation == "seal":
        (args.directory / MANIFEST).write_text(json.dumps({"fingerprint": key, "files": inventory(args.directory)}, indent=2), encoding="utf-8")
    else:
        smoke(args.directory)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
