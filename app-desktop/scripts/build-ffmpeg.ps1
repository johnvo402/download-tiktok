param([switch]$Force, [switch]$DownloadOnly)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ReleaseTag = "autobuild-2026-09-22-13-18"
$AssetName = "ffmpeg-n8.1.3-win64-gpl-8.1.zip"
$BaseUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$ReleaseTag"
$Archive = Join-Path $Root ".release-cache\$AssetName"
$Extract = Join-Path $Root ".release-cache\ffmpeg-extract"
$Target = Join-Path $Root "src-tauri\resources\ffmpeg"
$RequiredEncoders = @("libx264", "libx265", "h264_nvenc", "hevc_nvenc", "h264_qsv", "hevc_qsv", "h264_amf", "hevc_amf", "aac")
$RequiredFilters = @("ass", "boxblur", "gblur")
$Features = $RequiredEncoders + @("ass/libass", "boxblur", "gblur")
$License = "GPL-3.0-or-later (GPL build; libx264/libx265, NVENC, QSV and AMF encoders enabled)"
function Test-BundledFFmpeg {
    # Build presence differs from actual hardware usability, which the app probes at runtime.
    $encoders = & (Join-Path $Target "ffmpeg.exe") -hide_banner -encoders 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Cannot read bundled FFmpeg encoders." }
    $filters = & (Join-Path $Target "ffmpeg.exe") -hide_banner -filters 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Cannot read bundled FFmpeg filters." }
    foreach ($name in $RequiredEncoders) {
        if ($encoders -notmatch ("(?m)^\s*[A-Z.]{6}\s+" + [regex]::Escape($name) + "\s")) { throw "FFmpeg build lacks required encoder: $name" }
    }
    foreach ($name in $RequiredFilters) {
        if ($filters -notmatch ("(?m)^\s*[A-Z.]{2,3}\s+" + [regex]::Escape($name) + "\s")) { throw "FFmpeg build lacks required filter: $name" }
    }
}
function Remove-WorkspacePath([string]$Path) {
    $absolute = [IO.Path]::GetFullPath($Path)
    $rootPrefix = $Root.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $absolute.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing recursive removal outside the repository workspace: $absolute"
    }
    if ($absolute -ne [IO.Path]::GetFullPath($Extract)) {
        throw "Refusing recursive removal of an unexpected path: $absolute"
    }
    if (Test-Path -LiteralPath $absolute) { Remove-Item -LiteralPath $absolute -Recurse -Force }
}
if ((Test-Path (Join-Path $Target "ffmpeg.exe")) -and (Test-Path (Join-Path $Target "ffprobe.exe")) -and -not $Force -and -not $DownloadOnly) {
    Test-BundledFFmpeg
    $metadataPath = Join-Path $Target "ffmpeg-build.json"
    if (-not (Test-Path -LiteralPath $metadataPath)) { throw "Bundled FFmpeg provenance is missing. Pass -Force to restore it." }
    $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    $metadata.features = $Features
    $metadata.license = $License
    $metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $metadataPath -Encoding utf8
    Write-Host "Verified existing bundled FFmpeg encoders and filters. Pass -Force to replace the binaries."
    return
}
New-Item -ItemType Directory -Path (Split-Path $Archive) -Force | Out-Null
if (-not (Test-Path $Archive)) { Invoke-WebRequest -Uri "$BaseUrl/$AssetName" -OutFile $Archive }
$ChecksumsPath = Join-Path (Split-Path $Archive) "checksums.sha256"
if (-not (Test-Path $ChecksumsPath)) { Invoke-WebRequest -Uri "$BaseUrl/checksums.sha256" -OutFile $ChecksumsPath }
$checksumLine = Get-Content $ChecksumsPath | Where-Object { $_ -match [regex]::Escape($AssetName) } | Select-Object -First 1
if (-not $checksumLine) { throw "Pinned release checksum for $AssetName was not published." }
$expectedHash = ($checksumLine -split '\s+')[0].Trim('*')
$actualHash = (Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash.ToLowerInvariant()) { throw "FFmpeg archive checksum mismatch. Expected $expectedHash, got $actualHash." }
if ($DownloadOnly) {
    Write-Host "Cached verified FFmpeg archive ($actualHash)."
    return
}
Remove-WorkspacePath $Extract
Expand-Archive -LiteralPath $Archive -DestinationPath $Extract -Force
$ffmpeg = Get-ChildItem -LiteralPath $Extract -Filter ffmpeg.exe -Recurse | Select-Object -First 1
$ffprobe = Get-ChildItem -LiteralPath $Extract -Filter ffprobe.exe -Recurse | Select-Object -First 1
if (-not $ffmpeg -or -not $ffprobe) { throw "The pinned FFmpeg archive has no ffmpeg.exe or ffprobe.exe." }
New-Item -ItemType Directory -Path $Target -Force | Out-Null
Copy-Item -LiteralPath $ffmpeg.FullName -Destination (Join-Path $Target "ffmpeg.exe") -Force
Copy-Item -LiteralPath $ffprobe.FullName -Destination (Join-Path $Target "ffprobe.exe") -Force
Get-ChildItem -LiteralPath $ffmpeg.DirectoryName -Filter "*.dll" -File | Copy-Item -Destination $Target -Force
$version = (& (Join-Path $Target "ffmpeg.exe") -version | Select-Object -First 1)
Test-BundledFFmpeg
$manifest = [ordered]@{
    source = "BtbN/FFmpeg-Builds"
    releaseTag = $ReleaseTag
    asset = $AssetName
    sha256 = $actualHash
    ffmpegVersion = $version
    license = $License
    sourceUrl = "https://github.com/BtbN/FFmpeg-Builds/tree/$ReleaseTag"
    ffmpegSourceUrl = "https://ffmpeg.org/download.html"
    features = $Features
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Target "ffmpeg-build.json") -Encoding utf8
Write-Host "Bundled verified $version ($actualHash)"
