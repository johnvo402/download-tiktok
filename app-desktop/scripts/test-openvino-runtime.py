"""Opt-in installation smoke test in a disposable environment, never user data.

python scripts/test-openvino-runtime.py --uv <uv.exe>
Downloads the small optional runtime, not models. Requires usable Intel GPU.
"""
import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--uv", default="uv")
    args = parser.parse_args()
    environment = os.environ.copy()
    environment.update(PYTHONPATH=str(ROOT / "python"), PYTHONUTF8="1", UV_LINK_MODE="copy")
    with tempfile.TemporaryDirectory(prefix="JC AI Intel thử ") as directory:
        target = Path(directory) / "staging"
        subprocess.run([args.uv, "venv", "--managed-python", "--python", "3.12", "--no-project", str(target)], check=True, env=environment)
        python = target / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        subprocess.run([args.uv, "pip", "install", "--python", str(python), "-r", str(ROOT / "python/requirements-optional-ocr-openvino.in"), "onnxruntime==1.30.0"], check=True, env=environment)
        subprocess.run([args.uv, "pip", "check", "--python", str(python)], check=True, env=environment)
        script = """
import json
from importlib.metadata import distributions, version
from worker.ocr.backends import probe_openvino_gpu
packages = {d.metadata['Name'].lower(): d.version for d in distributions()}
assert not any(name in packages for name in ('torch','torchaudio','whisperx','transformers','onnxruntime-gpu'))
assert version('openvino') == '2026.4.0'
cap = probe_openvino_gpu()
assert cap.available, cap.message
print(json.dumps({'capability':cap.payload(),'packages':packages}))
"""
        result = subprocess.run([str(python), "-c", script], capture_output=True, text=True, check=True, env=environment)
        report = json.loads(result.stdout)
        subprocess.run([str(python), "-c", "import worker.__main__"], check=True, env=environment)
        # Exercise the actual JSONL inspect operation in this OCR-only worker.
        request = json.dumps({"operation": "ocr_inspect", "cacheDir": str(Path(directory) / "models"), "model": "tiny"}) + "\n"
        result = subprocess.run([str(python), "-m", "worker"], input=request, text=True,
                                capture_output=True, check=True, env=environment)
        message = json.loads(result.stdout.splitlines()[-1])
        assert message["backends"]["openvino"]["available"], message
        assert not message["modelCached"]
        report["venvBytes"] = sum(path.stat().st_size for path in target.rglob("*") if path.is_file())
        print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
