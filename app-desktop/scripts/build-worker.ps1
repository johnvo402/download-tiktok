param(
    [string]$Python = "python",
    [ValidateSet("all", "ai", "downloader")][string]$Kind = "all",
    [switch]$Clean
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$CacheScript = Join-Path $PSScriptRoot "worker-cache.py"
# Do not let a developer PYTHONPATH inject stale editable package metadata into
# either isolated build. Child workers use their own explicit source/frozen path.
$env:PYTHONPATH = $null
$env:PYTHONHOME = $null
$Kinds = if ($Kind -eq "all") { @("ai", "downloader") } else { @($Kind) }
function Remove-WorkspacePath([string]$Path, [string[]]$Allowed) {
    $absolute = [IO.Path]::GetFullPath($Path)
    $rootPrefix = $Root.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    if (-not $absolute.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or $absolute -notin $Allowed) {
        throw "Refusing removal outside named build directories: $absolute"
    }
    if (Test-Path -LiteralPath $absolute) {
        $entries = @(Get-Item -LiteralPath $absolute) + @(Get-ChildItem -LiteralPath $absolute -Recurse -Force)
        if ($entries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }) {
            throw "Refusing to remove a build directory containing links: $absolute"
        }
        Remove-Item -LiteralPath $absolute -Recurse -Force
    }
}
foreach ($WorkerKind in $Kinds) {
    $WorkerName = if ($WorkerKind -eq "ai") { "ai-worker" } else { "downloader-worker" }
    $Venv = Join-Path $Root "python\.release-$WorkerKind-venv"
    $PythonExe = Join-Path $Venv "Scripts\python.exe"
    $WorkerDist = Join-Path $Root "python\dist\$WorkerName"
    $WorkerTarget = Join-Path $Root "src-tauri\resources\$WorkerName"
    $BuildRoot = Join-Path $Root "python\build\$WorkerName"
    $Allowed = @($Venv, $WorkerDist, $WorkerTarget, $BuildRoot)
    if (-not $Clean) {
        & $Python $CacheScript verify --kind $WorkerKind
        if ($LASTEXITCODE -eq 0) {
            & $Python $CacheScript smoke --kind $WorkerKind
            if ($LASTEXITCODE -eq 0) { Write-Host "Reusing verified $WorkerName."; continue }
        }
    }
    if ($Clean) { Remove-WorkspacePath $WorkerDist $Allowed; Remove-WorkspacePath $BuildRoot $Allowed }
    if (Test-Path -LiteralPath $PythonExe) {
        $IdentityCode = 'import sys, platform; print(sys.version, sys.platform, platform.machine())'
        $Expected = & $Python -c $IdentityCode
        if ($LASTEXITCODE -ne 0) { throw "Could not inspect the release Python interpreter." }
        $Actual = & $PythonExe -c $IdentityCode
        if ($LASTEXITCODE -ne 0 -or $Actual -ne $Expected) { Remove-WorkspacePath $Venv $Allowed }
    }
    if (-not (Test-Path -LiteralPath $PythonExe)) {
        & $Python -m venv $Venv
        if ($LASTEXITCODE -ne 0) { throw "Could not create $WorkerName build environment." }
    }
    $LockName = if ($WorkerKind -eq "ai") { "requirements-release.txt" } else { "requirements-downloader.txt" }
    & $PythonExe -m pip install -r (Join-Path $Root "python\$LockName")
    if ($LASTEXITCODE -ne 0) { throw "Could not install $WorkerName pinned dependencies." }
    if ($WorkerKind -eq "ai") {
        & $PythonExe -m pip install --no-deps -r (Join-Path $Root "python\requirements-sdk.txt")
        if ($LASTEXITCODE -ne 0) { throw "Could not install pinned ONNX SDKs." }
        & $PythonExe (Join-Path $Root "python\packaging\check_dependencies.py")
    } else { & $PythonExe -m pip check }
    if ($LASTEXITCODE -ne 0) { throw "$WorkerName dependencies are inconsistent." }
    $Spec = if ($WorkerKind -eq "ai") { "ai_worker.spec" } else { "downloader_worker.spec" }
    Push-Location $Root
    try {
        & $PythonExe -m PyInstaller --noconfirm --distpath (Join-Path $Root "python\dist") --workpath $BuildRoot (Join-Path $Root "python\packaging\$Spec")
        if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed to package $WorkerName." }
    } finally { Pop-Location }
    if (-not (Test-Path -LiteralPath (Join-Path $WorkerDist "$WorkerName.exe"))) { throw "$WorkerName.exe was not produced." }
    Remove-WorkspacePath $WorkerTarget $Allowed
    New-Item -ItemType Directory -Path $WorkerTarget -Force | Out-Null
    Copy-Item -Path (Join-Path $WorkerDist "*") -Destination $WorkerTarget -Recurse -Force
    & $PythonExe (Join-Path $Root "python\packaging\write_worker_inventory.py") $WorkerTarget
    if ($LASTEXITCODE -ne 0) { throw "Could not generate $WorkerName license inventory." }
    Set-Content -LiteralPath (Join-Path $WorkerTarget "THIRD_PARTY_NOTICES.txt") -Value "Exact versions and license files are listed in python-packages.json. PyInstaller bootloader includes its redistribution exception. See included notices." -Encoding utf8
    $VersionModule = if ($WorkerKind -eq "ai") { "worker/_version.py" } else { "downloader_worker/_version.py" }
    $VersionText = Get-Content -LiteralPath (Join-Path $Root "python/$VersionModule") -Raw
    $WorkerVersion = [regex]::Match($VersionText, '__version__\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $WorkerVersion) { throw "Missing independent worker version." }
    Set-Content -LiteralPath (Join-Path $WorkerTarget "worker-version.txt") -Value "$WorkerName $WorkerVersion" -Encoding ascii
    & $Python $CacheScript smoke --kind $WorkerKind
    if ($LASTEXITCODE -ne 0) { throw "$WorkerName failed its real JSONL capability checks." }
    & $Python $CacheScript seal --kind $WorkerKind
    if ($LASTEXITCODE -ne 0) { throw "Could not seal $WorkerName cache." }
    Write-Host "Packaged $WorkerName at $WorkerTarget"
}
