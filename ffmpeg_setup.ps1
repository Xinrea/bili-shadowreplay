# Shared SDK: the executables alone cannot satisfy ffmpeg-next's native link.
# Release and SHA-256 from https://www.gyan.dev/ffmpeg/builds/.
$ErrorActionPreference = "Stop"
$version = "9.0.2"
$archiveName = "ffmpeg-$version-full_build-shared.7z"
$archiveUrl = "https://github.com/GyanD/codexffmpeg/releases/download/$version/$archiveName"
$expectedHash = "4d2060a8b34a940aa47d785142055bb92a63053781e55f2ace4546edd519a8f5"
$target = Join-Path $PSScriptRoot "src-tauri/target"
$archive = Join-Path $target $archiveName
$sdk = Join-Path $target "ffmpeg-sdk"
New-Item -ItemType Directory -Force -Path $target | Out-Null

if (-not (Test-Path $archive)) {
    Invoke-WebRequest -Uri $archiveUrl -OutFile $archive
}
if ((Get-FileHash -Path $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) {
    throw "FFmpeg archive checksum mismatch: $archive. Remove it and run this script again."
}
if (-not (Get-Command 7z -ErrorAction SilentlyContinue)) {
    throw "7-Zip is required to unpack the FFmpeg shared SDK. Install 7-Zip and add 7z to PATH."
}
$unpacked = Join-Path $target "ffmpeg-unpack"
if (Test-Path $unpacked) { Remove-Item $unpacked -Recurse -Force }
& 7z x $archive "-o$unpacked" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Failed to unpack FFmpeg SDK" }
$root = Join-Path $unpacked "ffmpeg-$version-full_build-shared"
foreach ($required in @("include/libavformat/avformat.h", "include/libswresample/swresample.h", "include/libavfilter/avfilter.h", "lib/avformat.lib", "lib/avcodec.lib", "lib/avutil.lib", "lib/swresample.lib", "lib/avfilter.lib", "bin/ffmpeg.exe", "bin/ffprobe.exe", "bin/ffplay.exe", "bin/avformat-63.dll", "bin/avcodec-63.dll", "bin/avutil-61.dll", "bin/swresample-7.dll", "bin/avfilter-12.dll")) {
    if (-not (Test-Path (Join-Path $root $required))) { throw "Incomplete FFmpeg SDK: $required" }
}
if (Test-Path $sdk) { Remove-Item $sdk -Recurse -Force }
Move-Item $root $sdk
Remove-Item $unpacked -Recurse -Force

$runtime = Join-Path $PSScriptRoot "src-tauri/windows-runtime"
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
Copy-Item (Join-Path $sdk "bin/*.exe") -Destination (Join-Path $PSScriptRoot "src-tauri") -Force
# Include all DLLs: codec/format depend on avutil and other FFmpeg components.
Copy-Item (Join-Path $sdk "bin/*.dll") -Destination $runtime -Force
$env:FFMPEG_DIR = $sdk
$env:PATH = "$(Join-Path $sdk 'bin');$env:PATH"
if (-not $env:LIBCLANG_PATH) {
    $vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
    if (Test-Path $vswhere) {
        $visualStudio = & $vswhere -latest -property installationPath
        if ($visualStudio) {
            $llvm = Join-Path $visualStudio "VC/Tools/LLVM/x64/bin"
            if (Test-Path (Join-Path $llvm "libclang.dll")) { $env:LIBCLANG_PATH = $llvm }
        }
    }
}
if (-not $env:LIBCLANG_PATH -and (Test-Path "${env:ProgramFiles}/LLVM/bin/libclang.dll")) {
    $env:LIBCLANG_PATH = "${env:ProgramFiles}/LLVM/bin"
}
if ($env:GITHUB_ENV) {
    "FFMPEG_DIR=$sdk" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
    if ($env:LIBCLANG_PATH) { "LIBCLANG_PATH=$env:LIBCLANG_PATH" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append }
    (Join-Path $sdk "bin") | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
}
Write-Host "FFmpeg shared SDK ready: $sdk"
