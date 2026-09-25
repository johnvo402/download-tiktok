param([string]$Python = "python")
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$VenvPython = Join-Path $Root "python\.venv\Scripts\python.exe"
if ($env:LOCALIZATION_PYTHON -and (Test-Path -LiteralPath $env:LOCALIZATION_PYTHON)) {
    $Python = $env:LOCALIZATION_PYTHON
} elseif (Test-Path -LiteralPath $VenvPython) {
    $Python = $VenvPython
}

Write-Host "Installing optional RapidOCR CUDA runtime in development environment: $Python"
Write-Host "This replaces the CPU ONNX Runtime package in this Python environment."
& $Python -m pip uninstall -y onnxruntime
if ($LASTEXITCODE -ne 0) { throw "Could not remove the CPU ONNX Runtime package." }
Push-Location (Join-Path $Root "python")
try {
    & $Python -m pip install -e ".[ocr-cuda]"
    if ($LASTEXITCODE -ne 0) { throw "Could not install RapidOCR with ONNX Runtime GPU." }
} finally {
    Pop-Location
}
Write-Host "CUDA OCR runtime installed. The app will expose CUDA only when the worker reports CUDAExecutionProvider."
