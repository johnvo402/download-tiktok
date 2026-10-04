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


def fingerprint(root: Path, interpreter: str | None = None, kind: str = "ai") -> str:
    identity = interpreter or f"{sys.implementation.name}-{sys.version}-{sys.platform}-{platform.machine()}"
    files = [root / "python/pyproject.toml", root / "scripts/build-worker.ps1", root / "scripts/worker-cache.py"]
    files += list((root / "python").glob("requirements-release.*" if kind == "ai" else "requirements-downloader.*"))
    files += [p for p in [root / "python/jc_worker_protocol.py", root / "python/requirements-sdk.txt"] if p.is_file() and (kind == "ai" or 'sdk' not in p.name)]
    for directory in (root / ("python/worker" if kind == "ai" else "python/downloader_worker"), root / "python/packaging"):
        files += [file for file in directory.rglob("*") if file.is_file() and "__pycache__" not in file.parts and file.suffix != ".pyc" and (directory.name != "packaging" or (not file.name.startswith("downloader") if kind == "ai" else file.name not in {"ai_worker.spec", "worker_launcher.py", "check_dependencies.py"}))]
    digest = hashlib.sha256(f"jc-worker-v2:{kind}:{identity}".encode())
    for file in sorted(set(files)):
        digest.update(file.relative_to(root).as_posix().encode() + b"\0")
        digest.update(file.read_bytes() + b"\0")
    return digest.hexdigest()


def inventory(directory: Path, kind: str = "ai") -> dict[str, str]:
    files = {}
    for file in sorted(directory.rglob("*")):
        if file.is_symlink():
            raise ValueError("A worker cache must not contain symlinks.")
        if file.is_file() and file.name != MANIFEST:
            with file.open("rb") as stream:
                files[file.relative_to(directory).as_posix()] = hashlib.file_digest(stream, "sha256").hexdigest()
    required = {("ai-worker.exe" if kind == "ai" else "downloader-worker.exe"), "worker-version.txt", "python-packages.json", "THIRD_PARTY_NOTICES.txt"}
    if not required <= files.keys():
        raise ValueError("The worker cache is incomplete.")
    return files


def verify(directory: Path, key: str, kind: str = "ai") -> bool:
    try:
        manifest = json.loads((directory / MANIFEST).read_text(encoding="utf-8"))
        return manifest["fingerprint"] == key and manifest["files"] == inventory(directory, kind)
    except (OSError, ValueError, KeyError):
        return False


def smoke(directory: Path, kind: str = "ai") -> None:
    executable = directory / ("ai-worker.exe" if kind == "ai" else "downloader-worker.exe")
    version = subprocess.run([str(executable), "--version"], capture_output=True, timeout=60, check=True)
    expected = (directory / "worker-version.txt").read_text().strip()
    if version.stdout.decode("utf-8").strip() != expected:
        raise ValueError("Packaged worker version does not match its inventory.")
    # Inspect capabilities with an isolated, empty model directory. Never downloads models.
    with tempfile.TemporaryDirectory(prefix="jc-worker-smoke-") as temporary:
        requests = [{"operation": "inspect", "model": "small", "cacheDir": temporary},
                    {"operation": "tts_inspect", "cacheDir": temporary},
                    {"operation": "ocr_inspect", "cacheDir": temporary}] if kind == "ai" else [{"operation": "downloader_inspect"}]
        requests.insert(0, {"operation": "ping", "protocolVersion": 2})
        result = subprocess.run([str(executable)], input="".join(json.dumps(item) + "\n" for item in requests),
                                capture_output=True, encoding="utf-8", timeout=120, check=True)
        responses = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        if len(responses) != len(requests) or responses[0].get('type') != 'ready' or any(item.get('protocolVersion') != 2 for item in responses) or any(item.get("type") != "capabilities" for item in responses[1:]):
            raise ValueError("Packaged worker did not return the expected capability protocol.")
        responses = responses[1:]
        if kind == "downloader" and not responses[0].get('workerInstalled'):
            raise ValueError('Downloader is missing its dependencies.')
        if kind == "ai" and not (responses[0].get("workerInstalled") and responses[1].get("workerInstalled")
                and responses[2].get("ocrInstalled") and responses[2].get("runtimeInstalled")):
            raise ValueError("Packaged worker is missing a required ASR, TTS or OCR dependency.")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", choices=["key", "verify", "seal", "smoke"])
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--kind", choices=['ai', 'downloader', 'all'], default='ai')
    args = parser.parse_args()
    if args.kind == 'all':
        if args.operation != 'key':
            parser.error('--kind all is only valid for key')
        print(hashlib.sha256((fingerprint(ROOT) + fingerprint(ROOT, kind='downloader')).encode()).hexdigest())
        return 0
    directory = args.directory or ROOT / ('src-tauri/resources/ai-worker' if args.kind == 'ai' else 'src-tauri/resources/downloader-worker')
    key = fingerprint(ROOT, kind=args.kind)
    if args.operation == "key":
        print(key)
    elif args.operation == "verify":
        return 0 if verify(directory, key, args.kind) else 1
    elif args.operation == "seal":
        (directory / MANIFEST).write_text(json.dumps({"fingerprint": key, "files": inventory(directory, args.kind)}, indent=2), encoding="utf-8")
    else:
        smoke(directory, args.kind)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
