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
$WorkerRoot = Join-Path $Root "python\worker"
Get-ChildItem -LiteralPath $WorkerRoot -Filter "*.py" -File -Recurse | ForEach-Object {
    $relative = $_.FullName.Substring($WorkerRoot.Length + 1)
    $destination = Join-Path (Join-Path $Source "worker") $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}
Write-Host "Optional AI installer and source staged for the release."
