param(
    [ValidateSet("8.1", "9.0")]
    [string[]]$Versions = @("9.0"),
    [string]$SdkRoot = (Join-Path ([IO.Path]::GetTempPath()) "bsr-ffmpeg")
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$SdkRoot = [IO.Path]::GetFullPath($SdkRoot)
$base = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"
New-Item -ItemType Directory -Force -Path $SdkRoot | Out-Null
$checksumFile = Join-Path $SdkRoot "checksums.sha256"
Invoke-WebRequest "$base/checksums.sha256" -OutFile $checksumFile
$checksums = Get-Content $checksumFile -Raw
$prefixes = foreach ($version in $Versions) {
    $name = "ffmpeg-n$version-latest-win64-gpl-shared-$version.zip"
    $archive = Join-Path $SdkRoot $name
    Invoke-WebRequest "$base/$name" -OutFile $archive
    $expected = [regex]::Match($checksums, "(?m)^([a-f0-9]{64})  $([regex]::Escape($name))\r?$").Groups[1].Value
    if ((Get-FileHash $archive -Algorithm SHA256).Hash -ne $expected) {
        throw "FFmpeg archive checksum mismatch: $archive"
    }
    $unpack = Join-Path $SdkRoot "unpack-$version"
    $prefix = Join-Path $SdkRoot $version
    Remove-Item $unpack, $prefix -Recurse -Force -ErrorAction SilentlyContinue
    [IO.Compression.ZipFile]::ExtractToDirectory($archive, $unpack)
    $directories = @(Get-ChildItem $unpack -Directory)
    if ($directories.Count -ne 1) { throw "Unexpected FFmpeg archive layout: $archive" }
    Move-Item $directories[0].FullName $prefix
    Remove-Item $unpack -Recurse -Force
    $prefix
}

# Install the newest runtime only; both SDKs can still produce ABI adapters.
$runtimeSdk = Join-Path $SdkRoot ($Versions | Sort-Object { [version]$_ } | Select-Object -Last 1)
$runtime = Join-Path $PSScriptRoot "src-tauri/windows-runtime"
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
Copy-Item "$runtimeSdk/bin/*.exe" (Join-Path $PSScriptRoot "src-tauri") -Force
Copy-Item "$runtimeSdk/bin/*.dll" $runtime -Force
Copy-Item "$runtimeSdk/LICENSE.txt" (Join-Path $runtime "FFmpeg-LICENSE.txt") -Force
$env:PATH = "$runtimeSdk/bin;$env:PATH"
$env:BSR_FFMPEG_PREFIXES = $prefixes -join [IO.Path]::PathSeparator
if ($env:GITHUB_ENV) {
    "BSR_FFMPEG_PREFIXES=$env:BSR_FFMPEG_PREFIXES" | Out-File $env:GITHUB_ENV -Encoding utf8 -Append
    "$runtimeSdk/bin" | Out-File $env:GITHUB_PATH -Encoding utf8 -Append
}
