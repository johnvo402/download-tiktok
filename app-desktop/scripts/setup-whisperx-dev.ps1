param(
    [string]$Python = ""
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$PythonExe = if ($Python) { $Python } else { Join-Path $Root ".venv\Scripts\python.exe" }
if (-not (Test-Path -LiteralPath $PythonExe -PathType Leaf)) {
    throw "Không tìm thấy Python development environment tại '$PythonExe'. Tạo .venv và cài worker cơ bản trước."
}

Write-Host "Cài WhisperX cùng PyTorch/TorchAudio vào development environment: $PythonExe"
Write-Host "Gói này khá lớn; trọng số aligner sẽ chỉ tải riêng sau khi xác nhận trong Settings."
& $PythonExe -m pip install -e "$Root\python[whisperx]"
if ($LASTEXITCODE -ne 0) {
    throw "Không cài được WhisperX. Kiểm tra phiên bản Python và hướng dẫn cài PyTorch/TorchAudio của WhisperX."
}
Write-Host "Đã cài runtime WhisperX. Mở Settings → Speech Recognition → Kiểm tra lại."
