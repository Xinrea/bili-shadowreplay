# download ffmpeg from https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip
$ffmpegUrl = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
$ffmpegPath = "ffmpeg-release-essentials.zip"
# download the file if it doesn't exist
if (-not (Test-Path $ffmpegPath)) {
    Invoke-WebRequest -Uri $ffmpegUrl -OutFile $ffmpegPath
}
# extract the 7z file
Add-Type -AssemblyName System.IO.Compression.FileSystem
$extractPath = "ffmpeg"
# check if the directory exists, if not create it
if (-not (Test-Path $extractPath)) {
    New-Item -ItemType Directory -Path $extractPath
}

[System.IO.Compression.ZipFile]::ExtractToDirectory($ffmpegPath, $extractPath)

# move the bin directory to the src-tauri directory
# ffmpeg/ffmpeg-*-essentials_build/bin to src-tauri
$ffmpegDir = Get-ChildItem -Path $extractPath -Directory | Where-Object { $_.Name -match "ffmpeg-.*-essentials_build" }
if ($ffmpegDir) {
    $binPath = Join-Path $ffmpegDir.FullName "bin"
} else {
    Write-Host "No ffmpeg directory found in the extracted files."
    exit 1
}

$destPath = Join-Path $PSScriptRoot "src-tauri"
Copy-Item -Path "$binPath/*" -Destination $destPath -Recurse

# remove the extracted directory
Remove-Item $extractPath -Recurse -Force