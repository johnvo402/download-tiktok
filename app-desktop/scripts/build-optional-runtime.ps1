param()
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Target = Join-Path $Root "src-tauri\resources\optional-runtime"
$Source = Join-Path $Target "source"
$Uv = Get-Command uv.exe -ErrorAction Stop
$UvVersion = & $Uv.Source --version
if ($LASTEXITCODE -ne 0 -or $UvVersion -notmatch '^uv 0\.11\.12 ') {
    throw "The release build requires uv 0.11.12; found '$UvVersion'."
}
New-Item -ItemType Directory -Path (Join-Path $Source "worker") -Force | Out-Null
Remove-Item -LiteralPath (Join-Path $Source "requirements-optional.in") -ErrorAction SilentlyContinue
Copy-Item -LiteralPath $Uv.Source -Destination (Join-Path $Target "uv.exe") -Force
Get-ChildItem -LiteralPath (Join-Path $Root "python") -Filter "requirements-optional-*.in" -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $Source $_.Name) -Force
}
foreach ($requirementsName in @("requirements-optional-whisperx.in", "requirements-optional-acceleration.in")) {
    $requirementsPath = Join-Path $Source $requirementsName
    $requirementsText = Get-Content -LiteralPath $requirementsPath -Raw
    if ($requirementsText -notmatch '(?m)^transformers==4\.57\.6\s*$') {
        throw "$requirementsName must pin transformers==4.57.6."
    }
    if ($requirementsText -notmatch '(?m)^huggingface-hub==0\.36\.2\s*
$WorkerRoot = Join-Path $Root "python\worker"
Get-ChildItem -LiteralPath $WorkerRoot -Filter "*.py" -File -Recurse | ForEach-Object {
    $relative = $_.FullName.Substring($WorkerRoot.Length + 1)
    $destination = Join-Path (Join-Path $Source "worker") $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}
Write-Host "Optional AI installer and source staged for the release."
) {
        throw "$requirementsName must pin huggingface-hub==0.36.2."
    }
    if ($requirementsName -eq "requirements-optional-acceleration.in" -and
        $requirementsText -notmatch '(?m)^gradio==5\.49\.1\s*
$WorkerRoot = Join-Path $Root "python\worker"
Get-ChildItem -LiteralPath $WorkerRoot -Filter "*.py" -File -Recurse | ForEach-Object {
    $relative = $_.FullName.Substring($WorkerRoot.Length + 1)
    $destination = Join-Path (Join-Path $Source "worker") $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}
Write-Host "Optional AI installer and source staged for the release."
) {
        throw "$requirementsName must pin gradio==5.49.1 to remain compatible with Transformers 4.57.6."
    }
}
$WorkerRoot = Join-Path $Root "python\worker"
Get-ChildItem -LiteralPath $WorkerRoot -Filter "*.py" -File -Recurse | ForEach-Object {
    $relative = $_.FullName.Substring($WorkerRoot.Length + 1)
    $destination = Join-Path (Join-Path $Source "worker") $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}
Write-Host "Optional AI installer and source staged for the release."
