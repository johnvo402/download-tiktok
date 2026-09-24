param(
    [string]$Python = "python",
    [switch]$Clean
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Venv = Join-Path $Root "python\.release-venv"
$PythonExe = Join-Path $Venv "Scripts\python.exe"
$WorkerDist = Join-Path $Root "python\dist\ai-worker"
$WorkerTarget = Join-Path $Root "src-tauri\resources\ai-worker"
$BuildRoot = Join-Path $Root "python\build"

function Remove-WorkspacePath([string]$Path) {
    $absolute = [IO.Path]::GetFullPath($Path)
    $rootPrefix = $Root.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $absolute.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing recursive removal outside the repository workspace: $absolute"
    }
    $allowed = @($WorkerDist, $BuildRoot, $WorkerTarget) | ForEach-Object { [IO.Path]::GetFullPath($_) }
    if ($absolute -notin $allowed) { throw "Refusing recursive removal of an unexpected path: $absolute" }
    if (Test-Path -LiteralPath $absolute) { Remove-Item -LiteralPath $absolute -Recurse -Force }
}

if ($Clean) {
    Remove-WorkspacePath $WorkerDist
    Remove-WorkspacePath $BuildRoot
}
if (-not (Test-Path $PythonExe)) {
    & $Python -m venv $Venv
    if ($LASTEXITCODE -ne 0) { throw "Could not create the release Python environment." }
}
$LockFile = Join-Path $Root "python\requirements-release.txt"
if (Test-Path $LockFile) {
    & $PythonExe -m pip install -r $LockFile
} else {
    & $PythonExe -m pip install -r (Join-Path $Root "python\requirements-release.in")
    if ($LASTEXITCODE -ne 0) { throw "Could not install pinned release worker dependencies." }
    & $PythonExe -m pip freeze | Set-Content -LiteralPath $LockFile -Encoding ascii
}
if ($LASTEXITCODE -ne 0) { throw "Could not install pinned release worker dependencies." }

Push-Location $Root
try {
    & $PythonExe -m PyInstaller --noconfirm --clean --distpath (Join-Path $Root "python\dist") --workpath $BuildRoot (Join-Path $Root "python\packaging\ai_worker.spec")
    if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed to package the AI worker." }
} finally { Pop-Location }

if (-not (Test-Path (Join-Path $WorkerDist "ai-worker.exe"))) { throw "PyInstaller did not produce ai-worker.exe." }
Remove-WorkspacePath $WorkerTarget
New-Item -ItemType Directory -Path $WorkerTarget -Force | Out-Null
Copy-Item -Path (Join-Path $WorkerDist "*") -Destination $WorkerTarget -Recurse -Force
& (Join-Path $WorkerTarget "ai-worker.exe") --version *> $null
if ($LASTEXITCODE -ne 0) { throw "The packaged AI worker did not start." }

$Inventory = & $PythonExe (Join-Path $Root "python\packaging\write_worker_inventory.py") $WorkerTarget
if ($LASTEXITCODE -ne 0) { throw "Could not generate the bundled Python package and license inventory." }
Set-Content -LiteralPath (Join-Path $WorkerTarget "THIRD_PARTY_NOTICES.txt") -Value @"
Third-party Python package inventory
====================================
Exact versions, declared licenses, and included license files are listed in python-packages.json.
Review the included license files and upstream notices before redistribution.
"@ -Encoding utf8
$PyProjectText = Get-Content (Join-Path $Root "python\pyproject.toml") -Raw
$WorkerVersion = [regex]::Match($PyProjectText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $WorkerVersion) { throw "Could not read the AI worker version." }
Set-Content -LiteralPath (Join-Path $WorkerTarget "worker-version.txt") -Value "ai-worker $WorkerVersion" -Encoding ascii
Write-Host "Packaged Python worker at $WorkerTarget"
