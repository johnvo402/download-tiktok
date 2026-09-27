# PyInstaller onedir spec for the stdin/stdout JSONL AI worker.
from PyInstaller.utils.hooks import collect_all, collect_data_files, collect_submodules
from pathlib import Path

ROOT = Path(SPECPATH).parent.resolve()
datas = []
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
    "playwright",
    "yt_dlp",
    "curl_cffi",
):
    package_datas, package_binaries, package_hiddenimports = collect_all(package)
    datas += package_datas
    binaries += package_binaries
    hiddenimports += package_hiddenimports
    hiddenimports += collect_submodules(package)

datas += collect_data_files("vieneu", includes=["assets/**"])

hiddenimports += [
    "onnxruntime.capi._pybind_state",
    "ctranslate2._ext",
    "vieneu.v3turbo",
    "vieneu._v3_turbo_engine.onnx_runtime_lite",
    "vieneu_utils.phonemize_text",
    "worker.asr.service",
    "worker.tts.service",
    "worker.ocr.service",
    "worker.tts.providers",
    "worker.dialogue.service",
    "worker.tiktok_drama.service",
    "worker.tiktok_drama.browser",
    "worker.tiktok_drama.browser_session",
    "worker.tiktok_drama.config",
    "worker.tiktok_drama.cookies",
    "worker.tiktok_drama.metadata",
    "worker.tiktok_drama.urls",
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
        "torch", "torchvision", "torchaudio", "transformers", "tensorflow",
        "gradio", "gradio_client", "fastapi", "uvicorn", "librosa",
        "numba", "llvmlite", "scipy", "pandas",
        "vieneu.fast", "vieneu.turbo", "vieneu.core_xpu", "vieneu.standard",
        "vieneu.remote", "vieneu.serve", "vieneu.v3nano", "vieneu.v3_turbo_serve",
        "jupyter", "IPython",
    ],
    noarchive=False,
)
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
