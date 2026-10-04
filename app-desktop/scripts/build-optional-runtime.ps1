param([string]$Python = "python")
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
# This pure-Python dependency is published as an sdist. Build its wheel here,
# not on a user's machine (where a long cache path can break setuptools).
$Wheels = Join-Path $Target "wheels"
New-Item -ItemType Directory -Path $Wheels -Force | Out-Null
& $Python -m pip wheel --no-deps --wheel-dir $Wheels "antlr4-python3-runtime==4.9.3"
if ($LASTEXITCODE -ne 0) { throw "Could not prepare the pinned parser wheel." }
$Wheel = Get-ChildItem -LiteralPath $Wheels -Filter "antlr4_python3_runtime-4.9.3-*.whl" -File
if (@($Wheel).Count -ne 1) { throw "Expected one pinned parser wheel." }
@{ name = $Wheel.Name; sha256 = (Get-FileHash -LiteralPath $Wheel.FullName -Algorithm SHA256).Hash.ToLowerInvariant() } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Wheels "manifest.json") -Encoding utf8
$BanditRevision = "7ec03cb568811958db65a96a10fdb8879922b2ac"
$BanditSource = "https://github.com/openmirlab/bandit-infer/archive/$BanditRevision.tar.gz"
& $Python -m pip wheel --no-deps --wheel-dir $Wheels "bandit-infer @ $BanditSource"
if ($LASTEXITCODE -ne 0) { throw "Could not prepare the pinned BandIt wheel." }
$BanditWheel = Get-ChildItem -LiteralPath $Wheels -Filter "bandit_infer-0.1.0-*.whl" -File
if (@($BanditWheel).Count -ne 1) { throw "Expected one pinned BandIt wheel." }
@{ name = $BanditWheel.Name; sha256 = (Get-FileHash -LiteralPath $BanditWheel.FullName -Algorithm SHA256).Hash.ToLowerInvariant(); source = $BanditSource; revision = $BanditRevision } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Wheels "bandit-manifest.json") -Encoding utf8
Get-ChildItem -LiteralPath (Join-Path $Root "python") -Filter "requirements-optional-*.in" -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $Source $_.Name) -Force
}
foreach ($requirementsName in @("requirements-optional-whisperx.in", "requirements-optional-acceleration.in")) {
    $requirementsPath = Join-Path $Source $requirementsName
    $requirementsText = Get-Content -LiteralPath $requirementsPath -Raw
    if ($requirementsText -notmatch '(?m)^transformers==4\.57\.6\s*$') {
        throw "$requirementsName must pin transformers==4.57.6."
    }
    if ($requirementsText -notmatch '(?m)^huggingface-hub==0\.36\.2\s*$') {
        throw "$requirementsName must pin huggingface-hub==0.36.2."
    }
    if ($requirementsName -eq "requirements-optional-acceleration.in" -and
        $requirementsText -notmatch '(?m)^gradio==5\.49\.1\s*$') {
        throw "$requirementsName must pin gradio==5.49.1 to remain compatible with Transformers 4.57.6."
    }
}
foreach ($requirementsName in @("requirements-optional-whisperx.in", "requirements-optional-acceleration.in")) {
    $requirementsPath = Join-Path $Source $requirementsName
    $requirementsText = Get-Content -LiteralPath $requirementsPath -Raw
    if ($requirementsText -notmatch '(?m)^transformers==4\.57\.6\s*$') {
        throw "$requirementsName must pin transformers==4.57.6."
    }
    if ($requirementsText -notmatch '(?m)^huggingface-hub==0\.36\.2\s*$') {
        throw "$requirementsName must pin huggingface-hub==0.36.2."
    }
}
$IntelRequirements = Get-Content -LiteralPath (Join-Path $Source "requirements-optional-ocr-openvino.in") -Raw
if ($IntelRequirements -notmatch '(?m)^openvino==2026\.4\.0\s*$' -or
    $IntelRequirements -match '(?m)^(torch|torchaudio|whisperx|transformers|onnxruntime-gpu)[=><@]') {
    throw "Intel OCR must use pinned OpenVINO and must not install the heavy speech/CUDA packages."
}
$WorkerRoot = Join-Path $Root "python\worker"
Copy-Item -LiteralPath (Join-Path $Root "python\jc_worker_protocol.py") -Destination (Join-Path $Source "jc_worker_protocol.py") -Force
Copy-Item -LiteralPath (Join-Path $Root "python\runtime-matrix.json") -Destination (Join-Path $Source "runtime-matrix.json") -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\THIRD_PARTY_LICENSES.txt") -Destination (Join-Path $Target "THIRD_PARTY_NOTICES.txt") -Force
# Earlier releases staged downloader source into this directory. Remove only
# this known obsolete subtree, after checking both its path and link attributes.
$OldDownloader = [IO.Path]::GetFullPath((Join-Path $Source "worker\tiktok_drama"))
if (Test-Path -LiteralPath $OldDownloader) {
    if (-not $OldDownloader.StartsWith([IO.Path]::GetFullPath($Source) + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Unsafe staged downloader path." }
    $Entries = @(Get-Item -LiteralPath $OldDownloader) + @(Get-ChildItem -LiteralPath $OldDownloader -Recurse -Force)
    if ($Entries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }) { throw "Staged source contains links." }
    Remove-Item -LiteralPath $OldDownloader -Recurse -Force
}
Get-ChildItem -LiteralPath $WorkerRoot -Filter "*.py" -File -Recurse | ForEach-Object {
    $relative = $_.FullName.Substring($WorkerRoot.Length + 1)
    $destination = Join-Path (Join-Path $Source "worker") $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}
Copy-Item -LiteralPath (Join-Path $WorkerRoot "asr/openvino-models.json") -Destination (Join-Path $Source "worker/asr/openvino-models.json") -Force
Write-Host "Optional AI installer and source staged for the release."
