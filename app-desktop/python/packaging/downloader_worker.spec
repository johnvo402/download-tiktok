from pathlib import Path
import sys
ROOT = Path(SPECPATH).parent.resolve()
sys.path.insert(0, str(ROOT / 'packaging'))
from bundle_helpers import dynamic_package, attach_licenses
datas, binaries, hiddenimports = [], [], []
for package in ('playwright', 'curl_cffi'):
    d, b, h = dynamic_package(package)
    datas += d; binaries += b; hiddenimports += h
hiddenimports += ['downloader_worker.tiktok_drama.service', 'downloader_worker.tiktok_drama.browser_session']
a = Analysis([str(ROOT/'packaging/downloader_launcher.py')], pathex=[str(ROOT)], binaries=binaries,
    datas=datas, hiddenimports=hiddenimports, hookspath=[], hooksconfig={}, runtime_hooks=[],
    excludes=['worker', 'torch', 'onnxruntime', 'openvino', 'faster_whisper', 'rapidocr', 'vieneu',
              'cv2', 'numpy', 'tokenizers', 'transformers', 'gradio', 'tkinter', 'pytest', 'IPython'], noarchive=False)
attach_licenses(a)
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name='downloader-worker', debug=False,
    bootloader_ignore_signals=False, strip=False, upx=False, console=True)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name='downloader-worker')
