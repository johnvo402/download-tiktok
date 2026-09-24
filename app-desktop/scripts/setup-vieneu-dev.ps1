$ErrorActionPreference = 'Stop'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$python = Join-Path $repositoryRoot '.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python -PathType Leaf)) {
    throw "Không tìm thấy Python worker tại '$python'. Hãy tạo .venv và cài worker cơ bản trước."
}

# The app only uses VieNeu v3 Turbo's CPU/ONNX path. Install its SDK without
# optional Gradio/Librosa UI and legacy dependencies; install the runtime audio
# packages the worker actually uses. Base worker setup provides ONNX Runtime,
# tokenizers, NumPy and Hugging Face Hub.
& $python -m pip install --no-deps 'vieneu==3.8.3'
if ($LASTEXITCODE -ne 0) { throw 'Không cài được VieNeu-TTS SDK.' }

& $python -m pip install 'sea-g2p>=0.9.1' soundfile soxr 'kaldi-native-fbank>=1.20'
if ($LASTEXITCODE -ne 0) { throw 'Không cài được dependency audio của VieNeu-TTS.' }

Write-Host 'Đã cài VieNeu-TTS SDK cho worker development. Model weights vẫn được quản lý riêng trong app-data.'
