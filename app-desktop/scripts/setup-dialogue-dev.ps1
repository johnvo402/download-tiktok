param(
    [string]$Python = ".venv\Scripts\python.exe"
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$PythonExe = if ([IO.Path]::IsPathRooted($Python)) { $Python } else { Join-Path $Root $Python }
if (-not (Test-Path -LiteralPath $PythonExe)) {
    throw "Không tìm thấy Python development environment: $PythonExe. Tạo .venv hoặc truyền -Python tới interpreter của dự án."
}
$PythonVersion = & $PythonExe -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")'
if ($LASTEXITCODE -ne 0 -or [version]$PythonVersion -lt [version]'3.10') {
    throw "BandIt yêu cầu Python 3.10 trở lên."
}
Write-Host "Installing CPU PyTorch and Torchaudio (large download; CUDA is not included)..."
& $PythonExe -m pip install torch==2.8.0 torchaudio==2.8.0 --index-url https://download.pytorch.org/whl/cpu
if ($LASTEXITCODE -ne 0) { throw "Could not install CPU PyTorch runtime." }
Write-Host "Installing the pinned BandIt inference adapter and WAV support..."
& $PythonExe -m pip install soundfile "bandit-infer @ git+https://github.com/openmirlab/bandit-infer.git@7ec03cb568811958db65a96a10fdb8879922b2ac"
if ($LASTEXITCODE -ne 0) { throw "Could not install BandIt inference packages." }
Write-Host "Dialogue separation runtime is ready. Restart JC AI and check Settings -> Speech Recognition."
