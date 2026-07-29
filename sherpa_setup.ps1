$ErrorActionPreference = "Stop"

$sherpaVersion = "1.13.4"
$archiveName = "sherpa-onnx-v$sherpaVersion-win-x64-shared-MT-Release-lib.tar.bz2"
$downloadUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$sherpaVersion/$archiveName"

$runtimeRoot = if ($env:RUNNER_TEMP) {
  Join-Path $env:RUNNER_TEMP "bili-shadowreplay-sherpa"
} else {
  Join-Path ([System.IO.Path]::GetTempPath()) "bili-shadowreplay-sherpa"
}
$archiveDirectory = Join-Path $runtimeRoot "archives"
$extractDirectory = Join-Path $runtimeRoot "runtime"
$archivePath = Join-Path $archiveDirectory $archiveName
$extractedArchiveDirectory = Join-Path $extractDirectory $archiveName.Replace(".tar.bz2", "")
$libraryDirectory = Join-Path $extractedArchiveDirectory "lib"
$destinationDirectory = Join-Path $PSScriptRoot "src-tauri"

New-Item -ItemType Directory -Force -Path $archiveDirectory, $extractDirectory | Out-Null

if (-not (Test-Path $archivePath -PathType Leaf)) {
  Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath
}

tar -xjf $archivePath -C $extractDirectory
if ($LASTEXITCODE -ne 0) {
  throw "Failed to extract sherpa-onnx runtime archive: $archivePath"
}

$runtimeDlls = @(
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "sherpa-onnx-c-api.dll",
  "sherpa-onnx-cxx-api.dll"
)

foreach ($dll in $runtimeDlls) {
  $source = Join-Path $libraryDirectory $dll
  if (-not (Test-Path $source -PathType Leaf)) {
    throw "Required sherpa-onnx runtime DLL not found: $source"
  }

  Copy-Item $source -Destination $destinationDirectory -Force
}

if ($env:GITHUB_ENV) {
  "SHERPA_ONNX_ARCHIVE_DIR=$archiveDirectory" |
    Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
}
