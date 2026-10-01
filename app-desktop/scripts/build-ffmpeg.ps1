param([switch]$Force, [switch]$DownloadOnly)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ReleaseTag = "autobuild-2026-09-22-13-18"
$AssetName = "ffmpeg-n8.1.3-win64-gpl-8.1.zip"
$BaseUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$ReleaseTag"
$Archive = Join-Path $Root ".release-cache\$AssetName"
$Extract = Join-Path $Root ".release-cache\ffmpeg-extract"
$Target = Join-Path $Root "src-tauri\resources\ffmpeg"
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
    Write-Host "Using existing bundled FFmpeg binaries. Pass -Force to replace them."
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
$encoders = & (Join-Path $Target "ffmpeg.exe") -hide_banner -encoders 2>&1 | Out-String
$filters = & (Join-Path $Target "ffmpeg.exe") -hide_banner -filters 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $encoders -notmatch 'libx264' -or $encoders -notmatch 'libx265' -or $encoders -notmatch 'h264_nvenc' -or $encoders -notmatch 'hevc_nvenc' -or $encoders -notmatch '\baac\b' -or $filters -notmatch '\bass\b' -or $filters -notmatch '\bboxblur\b') {
    throw "Pinned FFmpeg build lacks a renderer-required H.264/HEVC CPU/GPU encoder or filter."
}
$manifest = [ordered]@{
    source = "BtbN/FFmpeg-Builds"
    releaseTag = $ReleaseTag
    asset = $AssetName
    sha256 = $actualHash
    ffmpegVersion = $version
    license = "GPL-3.0-or-later (GPL build; libx264/libx265 and NVENC encoders enabled)"
    sourceUrl = "https://github.com/BtbN/FFmpeg-Builds/tree/$ReleaseTag"
    ffmpegSourceUrl = "https://ffmpeg.org/download.html"
    features = @("libx264", "libx265", "h264_nvenc", "hevc_nvenc", "aac", "ass/libass", "boxblur", "gblur")
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Target "ffmpeg-build.json") -Encoding utf8
Write-Host "Bundled verified $version ($actualHash)"
