param([string]$Python = "python")
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$VenvPython = Join-Path $Root "python\.venv\Scripts\python.exe"
if ($env:LOCALIZATION_PYTHON -and (Test-Path -LiteralPath $env:LOCALIZATION_PYTHON)) {
    $Python = $env:LOCALIZATION_PYTHON
} elseif (Test-Path -LiteralPath $VenvPython) {
    $Python = $VenvPython
}
Push-Location (Join-Path $Root "python")
try {
    & $Python -m pip install -e ".[ocr]"
    if ($LASTEXITCODE -ne 0) { throw "Could not install the optional RapidOCR development runtime." }
} finally {
    Pop-Location
}
Write-Host "RapidOCR runtime installed for development. Open Settings → Speech Recognition to download its model weights."
