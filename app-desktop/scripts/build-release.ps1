param(
    [string]$Python = "python",
    [switch]$SkipTests,
    [switch]$RequireSigning,
    [string]$CertificateThumbprint
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Release = Join-Path $Root "release"
$ReleaseVersion = $env:AI_VIDEO_LOCALIZATION_VERSION
if ($ReleaseVersion) {
    $VersionScript = Join-Path $PSScriptRoot "set-project-version.mjs"
    & node $VersionScript $ReleaseVersion
    if ($LASTEXITCODE -ne 0) { throw "Could not synchronize project version to $ReleaseVersion." }
}
$TargetRoot = if ($env:CARGO_TARGET_DIR) {
    [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR, $Root)
} else {
    Join-Path $Root "src-tauri\target"
}
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
if ($env:OS -ne "Windows_NT") { throw "This script produces the Windows x64 release only." }
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw "Run the Windows x64 release build on an x64 Windows host."
}
$PythonVersion = & $Python -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")'
if ($LASTEXITCODE -ne 0 -or $PythonVersion -ne "3.12") { throw "Use a working Python 3.12 x64 interpreter for the release build." }
if ($RequireSigning -and -not $CertificateThumbprint) { throw "Provide -CertificateThumbprint for a signed public release." }
if ($CertificateThumbprint -or $RequireSigning) {
    if (-not (Get-Command signtool.exe -ErrorAction SilentlyContinue)) { throw "Windows SDK signtool.exe was not found." }
}
$Package = Get-Content (Join-Path $Root "package.json") -Raw | ConvertFrom-Json
$Tauri = Get-Content (Join-Path $Root "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$CargoText = Get-Content (Join-Path $Root "src-tauri\Cargo.toml") -Raw
$PyProjectText = Get-Content (Join-Path $Root "python\pyproject.toml") -Raw
$WorkerVersionText = Get-Content (Join-Path $Root "python\worker\_version.py") -Raw
$CargoVersion = [regex]::Match($CargoText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$WorkerVersion = [regex]::Match($PyProjectText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$WorkerCodeVersion = [regex]::Match($WorkerVersionText, '(?m)^__version__\s*=\s*"([^"]+)"').Groups[1].Value
if ($Package.version -ne $Tauri.version -or $Package.version -ne $CargoVersion -or $Package.version -ne $WorkerVersion -or $Package.version -ne $WorkerCodeVersion) {
    throw "Release version mismatch: package.json=$($Package.version), tauri=$($Tauri.version), Cargo.toml=$CargoVersion, pyproject.toml=$WorkerVersion, worker/_version.py=$WorkerCodeVersion."
}

& (Join-Path $PSScriptRoot "build-ffmpeg.ps1") -Force
& (Join-Path $PSScriptRoot "build-worker.ps1") -Python $Python -Clean
& (Join-Path $PSScriptRoot "build-optional-runtime.ps1")
foreach ($path in @(
    (Join-Path $Root "src-tauri\resources\ffmpeg\ffmpeg.exe"),
    (Join-Path $Root "src-tauri\resources\ffmpeg\ffprobe.exe"),
    (Join-Path $Root "src-tauri\resources\ai-worker\ai-worker.exe"),
    (Join-Path $Root "src-tauri\resources\optional-runtime\uv.exe"),
    (Join-Path $Root "src-tauri\resources\optional-runtime\source\worker\__main__.py"),
    (Join-Path $Root "src-tauri\resources\optional-runtime\source\requirements-optional-whisperx.in"),
    (Join-Path $Root "src-tauri\resources\optional-runtime\source\requirements-optional-bandit.in"),
    (Join-Path $Root "src-tauri\resources\optional-runtime\source\requirements-optional-acceleration.in"),
    (Join-Path $Root "src-tauri\resources\THIRD_PARTY_LICENSES.txt")
)) { if (-not (Test-Path $path)) { throw "Release resource is missing: $path" } }

Push-Location $Root
try {
    npm ci
    if ($LASTEXITCODE -ne 0) { throw "npm ci failed." }
    if (-not $SkipTests) {
        npm test
        if ($LASTEXITCODE -ne 0) { throw "Frontend tests failed." }
        $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
        cargo test --manifest-path src-tauri/Cargo.toml --lib
        if ($LASTEXITCODE -ne 0) { throw "Rust tests failed." }
        $PreviousPythonPath = $env:PYTHONPATH
        $WorkerSourcePath = Join-Path $Root "python"
        if ([string]::IsNullOrWhiteSpace($PreviousPythonPath)) {
            $env:PYTHONPATH = $WorkerSourcePath
        } else {
            $env:PYTHONPATH = "$WorkerSourcePath$([System.IO.Path]::PathSeparator)$PreviousPythonPath"
        }
        try {
            & $Python -m unittest discover -s python/tests -v
            if ($LASTEXITCODE -ne 0) { throw "Python worker tests failed." }
        } finally {
            if ($null -eq $PreviousPythonPath) {
                Remove-Item Env:PYTHONPATH -ErrorAction SilentlyContinue
            } else {
                $env:PYTHONPATH = $PreviousPythonPath
            }
        }
    }
    & $Python (Join-Path $Root "scripts\generate_dependency_notices.py") $Root
    if ($LASTEXITCODE -ne 0) { throw "Could not generate Rust/npm third-party license notices." }
    # Invoke the local Tauri CLI directly. Recent npm versions parse unknown
    # flags after `npm run` as npm options even when separated by `--`.
    & (Join-Path $Root "node_modules\.bin\tauri.cmd") build --bundles nsis
    if ($LASTEXITCODE -ne 0) { throw "Tauri NSIS build failed." }
} finally { Pop-Location }

$Setup = Get-ChildItem (Join-Path $TargetRoot "release\bundle\nsis") -Filter "*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $Setup) { throw "Tauri did not produce an NSIS setup executable." }
if ($CertificateThumbprint) {
    $signTool = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if (-not $signTool) { throw "Windows SDK signtool.exe was not found." }
    & $signTool.Source sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com /sha1 $CertificateThumbprint $Setup.FullName
    if ($LASTEXITCODE -ne 0) { throw "Code signing the installer failed." }
} elseif ($RequireSigning) {
    throw "Public releases require code signing; unsigned installers are not copied as releases."
}

New-Item -ItemType Directory -Path $Release -Force | Out-Null
$Artifact = Join-Path $Release "JC-AI-$($Package.version)-windows-x64-setup.exe"
Copy-Item -LiteralPath $Setup.FullName -Destination $Artifact -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\THIRD_PARTY_LICENSES.txt") -Destination $Release -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\ffmpeg\ffmpeg-build.json") -Destination $Release -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\ai-worker\THIRD_PARTY_NOTICES.txt") -Destination $Release -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\ai-worker\python-packages.json") -Destination $Release -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\dependency-license-inventory.json") -Destination $Release -Force
Copy-Item -LiteralPath (Join-Path $Root "src-tauri\resources\THIRD_PARTY_DEPENDENCY_LICENSES.txt") -Destination $Release -Force
Copy-Item -LiteralPath (Join-Path $Root "docs\RELEASE_NOTES.md") -Destination $Release -Force
$Hash = (Get-FileHash -LiteralPath $Artifact -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath (Join-Path $Release "SHA256SUMS.txt") -Value "$Hash  $(Split-Path $Artifact -Leaf)" -Encoding ascii
$signature = Get-AuthenticodeSignature -LiteralPath $Artifact
@{
    version = $Package.version
    target = "windows-x64"
    installer = (Split-Path $Artifact -Leaf)
    sha256 = $Hash
    signatureStatus = $signature.Status.ToString()
    builtAtUtc = [DateTime]::UtcNow.ToString("o")
    ffmpeg = Get-Content (Join-Path $Root "src-tauri\resources\ffmpeg\ffmpeg-build.json") -Raw | ConvertFrom-Json
    pythonPackages = Get-Content (Join-Path $Root "src-tauri\resources\ai-worker\python-packages.json") -Raw | ConvertFrom-Json
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Release "release-manifest.json") -Encoding utf8
Write-Host "Release artifact: $Artifact"
Write-Host "SHA-256: $Hash"
Write-Host "Signature: $($signature.Status)"
