# PyInstaller onedir spec for the stdin/stdout JSONL AI worker.
from PyInstaller.utils.hooks import collect_data_files
from pathlib import Path

ROOT = Path(SPECPATH).parent.resolve()
import sys
sys.path.insert(0, str(ROOT / "packaging"))
from bundle_helpers import dynamic_package, attach_licenses
datas = [(str(ROOT / "worker/asr/openvino-models.json"), "worker/asr")]
binaries = []
hiddenimports = []

for package in (
    "faster_whisper",
    "ctranslate2",
    "huggingface_hub",
    "tokenizers",
    "av",
    "onnxruntime",
    "rapidocr",
    "cv2",
    "shapely",
    "omegaconf",
    "pyclipper",
    "colorlog",
    "sea_g2p",
):
    package_datas, package_binaries, package_hiddenimports = dynamic_package(package)
    datas += package_datas
    binaries += package_binaries
    hiddenimports += package_hiddenimports

datas += collect_data_files("vieneu", includes=["assets/**"])

hiddenimports += [
    "onnxruntime.capi._pybind_state",
    "ctranslate2._ext",
    "vieneu.v3turbo",
    "vieneu._v3_turbo_engine.onnx_runtime_lite",
    "vieneu_utils.phonemize_text",
    "worker.asr.service",
    "worker.asr.backends",
    "worker.asr.openvino_models",
    "worker.asr.openvino_backend",
    "worker.tts.service",
    "worker.ocr.service",
    "worker.ocr.backends",
    "worker.text_removal.service",
    "worker.text_removal.detection",
    "worker.text_removal.frames",
    "worker.text_removal.masks",
    "worker.text_removal.patch_cache",
    "worker.text_removal.pipeline",
    "worker.text_removal.inpainting",
    "worker.tts.providers",
    "worker.dialogue.service",
]

a = Analysis(
    [str(ROOT / "packaging" / "worker_launcher.py")],
    pathex=[str(ROOT)],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        "openvino", "openvino_telemetry", "openvino_genai", "openvino_tokenizers",
        "torch", "torchvision", "torchaudio", "transformers", "tensorflow",
        "gradio", "gradio_client", "fastapi", "uvicorn", "librosa",
        "numba", "llvmlite", "scipy", "pandas",
        "vieneu.fast", "vieneu.turbo", "vieneu.core_xpu", "vieneu.standard",
        "vieneu.remote", "vieneu.serve", "vieneu.v3nano", "vieneu.v3_turbo_serve",
        "jupyter", "IPython", "pytest", "tkinter", "playwright", "curl_cffi", "downloader_worker",
    ],
    noarchive=False,
)
attach_licenses(a)
# Video decoding uses PyAV/FFmpeg, never OpenCV VideoCapture/VideoWriter.
# Keep cv2 itself for OCR/masks; its unused video plugin duplicates FFmpeg.
a.binaries = [row for row in a.binaries if not Path(row[0]).name.lower().startswith("opencv_videoio_ffmpeg")]
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="ai-worker",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name="ai-worker",
)
