param(
    [string]$Python = "python",
    [switch]$SkipTests,
    [switch]$RequireSigning,
    [string]$CertificateThumbprint,
    [ValidateSet("zlib", "lzma", "bzip2", "none")]
    [string]$Compression = "zlib"
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Release = Join-Path $Root "release"
$Timings = [System.Collections.Generic.List[object]]::new()
function Invoke-ReleaseStage([string]$Name, [scriptblock]$Action) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    if ($env:GITHUB_ACTIONS -eq "true") { Write-Host "::group::$Name" } else { Write-Host "Starting: $Name" }
    $status = "failed"
    try {
        & $Action
        $status = "success"
    } finally {
        $timer.Stop()
        $seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 2)
        $Timings.Add([ordered]@{ stage = $Name; seconds = $seconds; status = $status })
        Write-Host "${Name}: $status in ${seconds}s"
        if ($env:GITHUB_ACTIONS -eq "true") { Write-Host "::endgroup::" }
        if ($env:GITHUB_STEP_SUMMARY) {
            Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value "- ${Name}: **$status**, ${seconds}s"
        }
    }
}
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
if ($Package.version -ne $Tauri.version -or $Package.version -ne $CargoVersion -or -not $WorkerVersion -or $WorkerVersion -ne $WorkerCodeVersion) {
    throw "Release version mismatch: package.json=$($Package.version), tauri=$($Tauri.version), Cargo.toml=$CargoVersion, pyproject.toml=$WorkerVersion, worker/_version.py=$WorkerCodeVersion."
}

Invoke-ReleaseStage "Verify and stage FFmpeg" { & (Join-Path $PSScriptRoot "build-ffmpeg.ps1") -Force }
Invoke-ReleaseStage "Build or reuse Python worker" { & (Join-Path $PSScriptRoot "build-worker.ps1") -Python $Python }
Invoke-ReleaseStage "Stage optional runtime installer" { & (Join-Path $PSScriptRoot "build-optional-runtime.ps1") }
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
    Invoke-ReleaseStage "Install frontend dependencies" {
        npm ci
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed." }
    }
    if (-not $SkipTests) {
      Invoke-ReleaseStage "Run automated tests" {
        npm test
        if ($LASTEXITCODE -ne 0) { throw "Frontend tests failed." }
        npm run test:ci
        if ($LASTEXITCODE -ne 0) { throw "Release tooling tests failed." }
        $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
        cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
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
            & $Python -m unittest discover -s scripts/tests -p "test_*.py" -v
            if ($LASTEXITCODE -ne 0) { throw "Worker cache tests failed." }
        } finally {
            if ($null -eq $PreviousPythonPath) {
                Remove-Item Env:PYTHONPATH -ErrorAction SilentlyContinue
            } else {
                $env:PYTHONPATH = $PreviousPythonPath
            }
        }
      }
    }
    Invoke-ReleaseStage "Generate dependency notices" {
        & $Python (Join-Path $Root "scripts\generate_dependency_notices.py") $Root
        if ($LASTEXITCODE -ne 0) { throw "Could not generate Rust/npm third-party license notices." }
    }
    # Invoke the local Tauri CLI directly. Recent npm versions parse unknown
    # flags after `npm run` as npm options even when separated by `--`.
    Invoke-ReleaseStage "Build frontend and Rust release" {
        & (Join-Path $Root "node_modules\.bin\tauri.cmd") build --no-bundle -- --locked
        if ($LASTEXITCODE -ne 0) { throw "Tauri application build failed." }
    }
    $BundleStarted = [DateTime]::UtcNow
    Invoke-ReleaseStage "Package NSIS installer ($Compression)" {
        # A file avoids PowerShell/native argument quoting differences for JSON.
        $Config = Join-Path $Root ".release-cache\installer-config.json"
        New-Item -ItemType Directory -Path (Split-Path $Config) -Force | Out-Null
        @{ bundle = @{ windows = @{ nsis = @{ compression = $Compression } } } } |
            ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Config -Encoding utf8
        & (Join-Path $Root "node_modules\.bin\tauri.cmd") bundle --bundles nsis --config $Config
        if ($LASTEXITCODE -ne 0) { throw "Tauri NSIS packaging failed." }
    }
} finally { Pop-Location }

$SetupPath = Join-Path $TargetRoot "release\bundle\nsis\$($Tauri.productName)_$($Package.version)_x64-setup.exe"
$Setup = Get-Item -LiteralPath $SetupPath -ErrorAction SilentlyContinue
if (-not $Setup -or $Setup.LastWriteTimeUtc -lt $BundleStarted) { throw "Tauri did not produce a fresh installer for version $($Package.version)." }
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
    workerVersion = $WorkerVersion
    workerFingerprint = (Get-Content (Join-Path $Root "src-tauri\resources\ai-worker\worker-build.json") -Raw | ConvertFrom-Json).fingerprint
    installerCompression = $Compression
    installerBytes = $Setup.Length
    target = "windows-x64"
    installer = (Split-Path $Artifact -Leaf)
    sha256 = $Hash
    signatureStatus = $signature.Status.ToString()
    builtAtUtc = [DateTime]::UtcNow.ToString("o")
    ffmpeg = Get-Content (Join-Path $Root "src-tauri\resources\ffmpeg\ffmpeg-build.json") -Raw | ConvertFrom-Json
    pythonPackages = Get-Content (Join-Path $Root "src-tauri\resources\ai-worker\python-packages.json") -Raw | ConvertFrom-Json
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Release "release-manifest.json") -Encoding utf8
$Timings | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Release "build-timings.json") -Encoding utf8
if ($env:GITHUB_STEP_SUMMARY) {
    Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value "- Installer: **$([Math]::Round($Setup.Length / 1MB, 1)) MiB**, compression: **$Compression**"
}
Write-Host "Release artifact: $Artifact"
Write-Host "SHA-256: $Hash"
Write-Host "Signature: $($signature.Status)"
