# Shared SDK: executables alone cannot satisfy ffmpeg-next's native link.
# The Tauri launcher invokes this same helper and imports its child environment.
param([string]$EnvironmentFile)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$version = "9.0.2"
$archiveName = "ffmpeg-$version-full_build-shared.7z"
$archiveUrl = "https://github.com/GyanD/codexffmpeg/releases/download/$version/$archiveName"
$expectedHash = "4d2060a8b34a940aa47d785142055bb92a63053781e55f2ace4546edd519a8f5"
$target = Join-Path $PSScriptRoot "src-tauri/target"
$archive = Join-Path $target $archiveName
$sdk = Join-Path $target "ffmpeg-sdk/$version-$($expectedHash.Substring(0,12))"
$receipt = Join-Path $sdk "bsr-sdk.json"
$required = @("include/libavformat/avformat.h", "include/libswresample/swresample.h", "include/libavfilter/avfilter.h", "lib/avformat.lib", "lib/avcodec.lib", "lib/avutil.lib", "lib/swresample.lib", "lib/avfilter.lib", "bin/avformat-63.dll", "bin/avcodec-63.dll", "bin/avutil-61.dll", "bin/swresample-7.dll", "bin/avfilter-12.dll")
function Assert-Sdk([string]$directory) {
    foreach ($file in $required) {
        if (-not (Test-Path (Join-Path $directory $file) -PathType Leaf)) { throw "Incomplete FFmpeg 9 SDK: $directory/$file" }
    }
}
function File-Hash([string]$file) { (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() }
New-Item -ItemType Directory -Force -Path $target | Out-Null

if ($env:FFMPEG_DIR) {
    $sdk = (Resolve-Path -LiteralPath $env:FFMPEG_DIR).Path
    Assert-Sdk $sdk
} elseif (Test-Path $sdk) {
    Assert-Sdk $sdk
    if (-not (Test-Path $receipt)) { throw "Unverified FFmpeg cache; remove $sdk and retry" }
    $saved = Get-Content -Raw -LiteralPath $receipt | ConvertFrom-Json
    if ($saved.archive -ne $expectedHash) { throw "FFmpeg cache version mismatch; remove $sdk and retry" }
    foreach ($file in $saved.files.PSObject.Properties) {
        if ((File-Hash (Join-Path $sdk $file.Name)) -ne $file.Value) { throw "Invalid FFmpeg cache: $($file.Name); remove $sdk and retry" }
    }
} else {
    $tar = Join-Path $env:SystemRoot 'System32/tar.exe'
    $sevenZip = Get-Command 7z -ErrorAction SilentlyContinue
    if (-not $sevenZip) {
        foreach ($candidate in @("$env:ProgramFiles/7-Zip/7z.exe", "${env:ProgramFiles(x86)}/7-Zip/7z.exe", "$env:USERPROFILE/scoop/apps/7zip/current/7z.exe")) {
            if (Test-Path $candidate) { $sevenZip = Get-Item $candidate; break }
        }
    }
    if (-not (Test-Path $tar) -and -not $sevenZip) { throw "Windows tar or 7-Zip is required to unpack FFmpeg. Install current Windows tools or 64-bit 7-Zip; no PATH configuration is needed." }
    $sevenZipPath = if ($sevenZip -is [System.IO.FileInfo]) { $sevenZip.FullName } elseif ($sevenZip) { $sevenZip.Source } else { $null }
    $work = Join-Path $target (".ffmpeg-prepare-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        if (-not (Test-Path $archive)) {
            Write-Host "[native] Downloading FFmpeg $version shared SDK (first run)"
            $download = Join-Path $work $archiveName
            Invoke-WebRequest -UseBasicParsing -Uri $archiveUrl -OutFile $download
            if ((File-Hash $download) -ne $expectedHash) { throw "FFmpeg archive checksum mismatch" }
            # Another launcher may have completed the same download.
            if (-not (Test-Path $archive)) { Move-Item -LiteralPath $download -Destination $archive }
        }
        if ((File-Hash $archive) -ne $expectedHash) { throw "FFmpeg archive checksum mismatch: $archive. Remove it and retry." }
        Write-Host "[native] Preparing FFmpeg $version shared SDK"
        if (Test-Path $tar) { & $tar -xf $archive -C $work }
        else { & $sevenZipPath x $archive "-o$work" -y | Out-Null }
        if ($LASTEXITCODE -ne 0) { throw "Failed to unpack FFmpeg SDK" }
        $unpacked = Join-Path $work "ffmpeg-$version-full_build-shared"
        Assert-Sdk $unpacked
        $files = [ordered]@{}
        $entries = @(Get-ChildItem (Join-Path $unpacked 'include') -Recurse -File) + @(Get-ChildItem (Join-Path $unpacked 'lib') -File) + @(Get-ChildItem (Join-Path $unpacked 'bin/*.dll') -File)
        foreach ($file in $entries) {
            $relative = $file.FullName.Substring($unpacked.Length + 1)
            $files[$relative] = File-Hash $file.FullName
        }
        @{ archive = $expectedHash; files = $files } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $unpacked 'bsr-sdk.json') -Encoding UTF8
        New-Item -ItemType Directory -Force -Path (Split-Path $sdk) | Out-Null
        if (-not (Test-Path $sdk)) { Move-Item -LiteralPath $unpacked -Destination $sdk }
    } finally { Remove-Item -LiteralPath $work -Recurse -Force }
}

$runtime = Join-Path $PSScriptRoot "src-tauri/windows-runtime"
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
# Keep all transitive FFmpeg DLLs, but no CLI executables. Preserve CUDA/sherpa DLLs.
foreach ($dll in Get-ChildItem (Join-Path $sdk 'bin/*.dll') -File) {
    $copy = Join-Path $runtime $dll.Name
    if (-not (Test-Path $copy) -or (File-Hash $copy) -ne (File-Hash $dll.FullName)) {
        Copy-Item -LiteralPath $dll.FullName -Destination $copy -Force
    }
}
$env:FFMPEG_DIR = $sdk
$env:PATH = "$(Join-Path $sdk 'bin');$env:PATH"
if (-not $env:LIBCLANG_PATH) {
    $vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
    $candidates = @("$env:ProgramFiles/LLVM/bin")
    if (Test-Path $vswhere) {
        $studio = & $vswhere -latest -products '*' -property installationPath
        if ($studio) { $candidates += Join-Path $studio 'VC/Tools/LLVM/x64/bin' }
    }
    foreach ($directory in $candidates) {
        if (Test-Path (Join-Path $directory 'libclang.dll')) { $env:LIBCLANG_PATH = $directory; break }
    }
}
if ($EnvironmentFile) {
    @{ FFMPEG_DIR = $sdk; PATH = $env:PATH; LIBCLANG_PATH = $env:LIBCLANG_PATH } | ConvertTo-Json | Set-Content -LiteralPath $EnvironmentFile -Encoding UTF8
}
if ($env:GITHUB_ENV) {
    "FFMPEG_DIR=$sdk" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
    if ($env:LIBCLANG_PATH) { "LIBCLANG_PATH=$env:LIBCLANG_PATH" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append }
    (Join-Path $sdk 'bin') | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
}
Write-Host "[native] FFmpeg shared SDK: $sdk"
