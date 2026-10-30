[CmdletBinding()]
param(
    [string]$OutputDirectory = '',
    [ValidateSet('h264_vulkan', 'hevc_vulkan')]
    [string]$Encoder = 'h264_vulkan',
    [ValidateRange(160, 4096)][int]$Width = 1920,
    [ValidateRange(64, 4096)][int]$Height = 1080,
    [ValidateRange(1, 120)][int]$Fps = 60,
    [ValidateRange(12, 3600)][int]$Frames = 120,
    [ValidateNotNullOrEmpty()][string]$Device = '0',
    [string]$Python = '',
    [switch]$SkipBuild
)

# Windows PowerShell 5.1: native stderr must not enter a PowerShell pipeline.
# Invoke programs directly, with CRT argument quoting and two asynchronous pipes.
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This driver requires Windows PowerShell or PowerShell on Windows.' }
if (($Width % 2) -ne 0 -or ($Height % 2) -ne 0) { throw 'Width and Height must be even.' }

function Full-Path([string]$Value, [string]$Base) {
    if ([IO.Path]::IsPathRooted($Value)) { return [IO.Path]::GetFullPath($Value) }
    return [IO.Path]::GetFullPath((Join-Path $Base $Value))
}

function Quote-NativeArgument([AllowEmptyString()][string]$Value) {
    # CommandLineToArgvW/CRT rules: escape quotes, and double trailing backslashes.
    $builder = New-Object Text.StringBuilder
    [void]$builder.Append([char]34)
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]92) { $slashes++; continue }
        if ($character -eq [char]34) {
            [void]$builder.Append([char]92, [int](2 * $slashes + 1))
            [void]$builder.Append([char]34)
        } else {
            if ($slashes -gt 0) { [void]$builder.Append([char]92, [int]$slashes) }
            [void]$builder.Append($character)
        }
        $slashes = 0
    }
    if ($slashes -gt 0) { [void]$builder.Append([char]92, [int](2 * $slashes)) }
    [void]$builder.Append([char]34)
    return $builder.ToString()
}

function Native-Executable([string]$Name) {
    if ([IO.Path]::IsPathRooted($Name) -or $Name.Contains('\') -or $Name.Contains('/')) {
        $resolved = Full-Path $Name $callerDirectory
        if (-not [IO.File]::Exists($resolved)) { throw "Executable not found: $resolved" }
        if ([IO.Path]::GetExtension($resolved) -ne '.exe') { throw "A native .exe is required: $resolved" }
        return $resolved
    }
    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $command) { throw "Executable not found: $Name" }
    if ([IO.Path]::GetExtension($command.Source) -ne '.exe') { throw "A native .exe is required: $($command.Source)" }
    return $command.Source
}

function Invoke-Native {
    param(
        [string]$Executable,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [string]$LogPrefix,
        [hashtable]$Environment = @{},
        [int]$TimeoutSeconds = 0,
        [switch]$AllowFailure
    )
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = (($Arguments | ForEach-Object { Quote-NativeArgument $_ }) -join ' ')
    $info.WorkingDirectory = $WorkingDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($name in $Environment.Keys) {
        if ($null -eq $Environment[$name]) { $info.EnvironmentVariables.Remove($name) }
        else { $info.EnvironmentVariables[$name] = [string]$Environment[$name] }
    }
    $stdoutPath = $LogPrefix + '.stdout.log'
    $stderrPath = $LogPrefix + '.stderr.log'
    $stdoutFile = [IO.File]::Open($stdoutPath, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $stderrFile = [IO.File]::Open($stderrPath, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $info
    $started = $false
    $clock = [Diagnostics.Stopwatch]::StartNew()
    try {
        $started = $process.Start()
        if (-not $started) { throw "Could not start $Executable" }
        # Stream both pipes concurrently to logs, including on build failure.
        $outTask = $process.StandardOutput.BaseStream.CopyToAsync($stdoutFile)
        $errTask = $process.StandardError.BaseStream.CopyToAsync($stderrFile)
        while (-not $process.WaitForExit(250)) {
            if ($outTask.IsFaulted) { $outTask.GetAwaiter().GetResult() }
            if ($errTask.IsFaulted) { $errTask.GetAwaiter().GetResult() }
            if ($TimeoutSeconds -gt 0 -and $clock.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
                $process.Kill()
                $process.WaitForExit()
                throw "Timed out after $TimeoutSeconds seconds: $Executable"
            }
        }
        $outTask.GetAwaiter().GetResult()
        $errTask.GetAwaiter().GetResult()
        $exitCode = $process.ExitCode
    } finally {
        if ($started -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
        $stdoutFile.Dispose()
        $stderrFile.Dispose()
    }
    $stdout = [IO.File]::ReadAllText($stdoutPath)
    $stderr = [IO.File]::ReadAllText($stderrPath)
    if ($exitCode -ne 0 -and -not $AllowFailure) {
        $tail = ($stderr -split '\r?\n' | Select-Object -Last 24) -join [Environment]::NewLine
        throw "Native command failed ($exitCode): $Executable`n$tail`nLogs: $stdoutPath / $stderrPath"
    }
    return [PSCustomObject]@{ ExitCode = $exitCode; Stdout = $stdout; Stderr = $stderr }
}

function Restore-Environment([hashtable]$Values, [string[]]$Names) {
    foreach ($name in $Names) { [Environment]::SetEnvironmentVariable($name, $Values[$name], 'Process') }
}

$repo = Full-Path '..\..' $PSScriptRoot
$callerDirectory = (Get-Location).ProviderPath
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
if (-not [IO.File]::Exists($manifest)) { throw "PoC manifest not found: $manifest" }
# Cargo runs at the repository root, so relative CARGO_TARGET_DIR resolves there.
$target = if ($env:CARGO_TARGET_DIR) { Full-Path $env:CARGO_TARGET_DIR $repo } else { Join-Path $repo 'src-tauri\target\vulkan-poc' }
$resultDirectory = if ($OutputDirectory) {
    Full-Path $OutputDirectory $callerDirectory
} else {
    Join-Path $target ('runs\' + (Get-Date -Format 'yyyyMMdd-HHmmssfff') + '-' + $Encoder)
}
[void][IO.Directory]::CreateDirectory($resultDirectory)
if ([IO.File]::Exists((Join-Path $resultDirectory 'api-dump.txt'))) {
    throw 'The output directory already contains an API trace. Choose a fresh directory to keep captures attributable to one run.'
}

$captureNames = @('VK_INSTANCE_LAYERS', 'VK_LOADER_LAYERS_DISABLE', 'VK_LAYER_PATH', 'VK_LAYER_SETTINGS_PATH',
    'VK_APIDUMP_LOG_FILENAME', 'VK_APIDUMP_OUTPUT_FORMAT', 'VK_APIDUMP_DETAILED', 'VK_APIDUMP_OUTPUT_RANGE',
    'VK_APIDUMP_FLUSH', 'VK_APIDUMP_TIMESTAMP', 'VK_APIDUMP_LOG_TO_FILE')
$environmentNames = @('FFMPEG_DIR', 'PATH', 'LIBCLANG_PATH', 'SKIA_BINARIES_URL', 'CARGO_TARGET_DIR') + $captureNames
$savedEnvironment = @{}
foreach ($name in $environmentNames) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$uncaptured = @{}
foreach ($name in $captureNames) { $uncaptured[$name] = $null }
$utf8 = [Text.UTF8Encoding]::new($false)

try {
    $sdk = if ($env:FFMPEG_DIR) { Full-Path $env:FFMPEG_DIR $repo } else { Join-Path $repo 'src-tauri\target\ffmpeg-sdk' }
    $sdkBin = Join-Path $sdk 'bin'
    $ffprobe = Join-Path $sdkBin 'ffprobe.exe'
    $ffmpeg = Join-Path $sdkBin 'ffmpeg.exe'
    foreach ($file in @($ffprobe, $ffmpeg)) {
        if (-not [IO.File]::Exists($file)) { throw "FFmpeg SDK executable missing: $file. Run ffmpeg_setup.ps1 first or set FFMPEG_DIR." }
    }
    $env:FFMPEG_DIR = $sdk
    $env:CARGO_TARGET_DIR = $target
    $env:PATH = $sdkBin + ';' + $env:PATH
    # Build and verifiers must never contribute calls to the GPU capture.
    Restore-Environment $uncaptured $captureNames

    $pythonCandidates = if ($Python) { @($Python) } else { @('python.exe', 'python3.exe', 'py.exe') }
    $pythonProgram = $null
    $pythonPrefix = @()
    $attempt = 0
    foreach ($candidate in $pythonCandidates) {
        try {
            $program = Native-Executable $candidate
            # Default WindowsApps aliases can launch the Store instead of Python.
            if (-not $Python -and $program -match '\\Microsoft\\WindowsApps\\') { continue }
            # Keep an array even for one launcher flag; scalar string + array
            # would concatenate the entire Python command in PowerShell 5.1.
            $prefix = @()
            if ([IO.Path]::GetFileNameWithoutExtension($program) -eq 'py') { $prefix = @('-3') }
            $probeArguments = $prefix + @('-c', 'import json,sys; print(json.dumps({"version":list(sys.version_info[:3]),"executable":sys.executable})); sys.exit(0 if sys.version_info >= (3,9) else 1)')
            $probe = Invoke-Native -Executable $program -Arguments $probeArguments -WorkingDirectory $repo -LogPrefix (Join-Path $resultDirectory "python-probe-$attempt") -TimeoutSeconds 15 -AllowFailure -Environment $uncaptured
            $attempt++
            if ($probe.ExitCode -ne 0) { continue }
            $pythonInfo = $probe.Stdout | ConvertFrom-Json
            if ($pythonInfo.version[0] -eq 3 -and $pythonInfo.version[1] -ge 9) {
                $pythonProgram = $program
                $pythonPrefix = $prefix
                break
            }
        } catch {
            if ($Python) { throw }
        }
    }
    if (-not $pythonProgram) { throw 'Python 3.9+ is required for independent verification. Pass -Python with a real python.exe/py.exe path.' }

    $vulkanBin = if ($env:VULKAN_SDK) { Join-Path $env:VULKAN_SDK 'Bin' } else { '' }
    if (-not $vulkanBin -or -not [IO.File]::Exists((Join-Path $vulkanBin 'VkLayer_api_dump.json'))) {
        throw 'VULKAN_SDK must point to a Vulkan SDK containing Bin\VkLayer_api_dump.json for full API capture.'
    }
    if (-not $SkipBuild) {
        if (-not $env:LIBCLANG_PATH) {
            $llvm = Join-Path $env:ProgramFiles 'LLVM\bin'
            if ([IO.File]::Exists((Join-Path $llvm 'libclang.dll'))) { $env:LIBCLANG_PATH = $llvm }
        }
        $node = Native-Executable 'node.exe'
        $cargo = Native-Executable 'cargo.exe'
        Write-Host 'Preparing the pinned Windows Skia Vulkan archive...'
        $prepared = Invoke-Native -Executable $node -Arguments @((Join-Path $repo 'scripts\prepare-windows-skia.mjs'), '--vulkan') -WorkingDirectory $repo -LogPrefix (Join-Path $resultDirectory 'prepare-skia')
        $urls = [regex]::Matches($prepared.Stdout, '(?m)^SKIA_BINARIES_URL=(.+)\r?$')
        if ($urls.Count -ne 1) { throw 'Skia preparation did not print exactly one SKIA_BINARIES_URL.' }
        $env:SKIA_BINARIES_URL = $urls[0].Groups[1].Value.Trim()
        if ($env:SKIA_BINARIES_URL -notmatch '^file://.+\{key\}\.tar\.gz$') { throw 'Unexpected Skia archive URL.' }
        Write-Host 'Building the standalone release PoC...'
        [void](Invoke-Native -Executable $cargo -Arguments @('build', '--release', '--locked', '--manifest-path', $manifest) -WorkingDirectory $repo -LogPrefix (Join-Path $resultDirectory 'cargo-build'))
    }

    $pocProfileDirectory = if ($env:CARGO_BUILD_TARGET) { Join-Path $env:CARGO_BUILD_TARGET 'release' } else { 'release' }
    $executable = Join-Path (Join-Path $target $pocProfileDirectory) 'vulkan-video-poc.exe'
    if (-not [IO.File]::Exists($executable)) { throw "Release PoC executable not found: $executable" }
    $video = Join-Path $resultDirectory 'output.mp4'
    $trace = Join-Path $resultDirectory 'api-dump.txt'
    # The layer also gets the absolute filename via env; the file key enables it.
    $settings = @(
        'lunarg_api_dump.file = true',
        'lunarg_api_dump.log_filename = api-dump.txt',
        'lunarg_api_dump.output_format = text',
        'lunarg_api_dump.output_range = 0-0',
        'lunarg_api_dump.detailed = false',
        'lunarg_api_dump.flush = true'
    ) -join "`n"
    [IO.File]::WriteAllText((Join-Path $resultDirectory 'vk_layer_settings.txt'), $settings + "`n", $utf8)
    try {
        $env:VK_INSTANCE_LAYERS = 'VK_LAYER_LUNARG_api_dump'
        $env:VK_LOADER_LAYERS_DISABLE = '~implicit~'
        $env:VK_LAYER_PATH = $vulkanBin
        $env:VK_LAYER_SETTINGS_PATH = $resultDirectory
        $env:VK_APIDUMP_LOG_FILENAME = $trace
        $env:VK_APIDUMP_OUTPUT_FORMAT = 'text'
        $env:VK_APIDUMP_OUTPUT_RANGE = '0-0'
        $env:VK_APIDUMP_DETAILED = 'false'
        $env:VK_APIDUMP_FLUSH = 'true'
        $env:VK_APIDUMP_TIMESTAMP = 'false'
        Write-Host "Capturing ${Encoder}: $Frames GPU frames at ${Width}x${Height}/$Fps..."
        $render = Invoke-Native -Executable $executable -Arguments @('--output', $video, '--encoder', $Encoder, '--device', $Device,
            '--width', [string]$Width, '--height', [string]$Height, '--fps', [string]$Fps, '--frames', [string]$Frames) -WorkingDirectory $resultDirectory -LogPrefix (Join-Path $resultDirectory 'poc')
        [void]($render.Stdout | ConvertFrom-Json)
        [IO.File]::WriteAllText((Join-Path $resultDirectory 'poc.json'), $render.Stdout, $utf8)
    } finally {
        Restore-Environment $savedEnvironment $captureNames
    }

    Write-Host 'Verifying the finished MP4 and actual Vulkan API trace outside capture...'
    $codec = if ($Encoder -eq 'h264_vulkan') { 'h264' } else { 'hevc' }
    $verifyArguments = $pythonPrefix + @((Join-Path $PSScriptRoot 'verify.py'), '--video', $video, '--trace', $trace,
        '--width', [string]$Width, '--height', [string]$Height, '--fps', [string]$Fps, '--frames', [string]$Frames,
        '--codec', $codec, '--ffprobe', $ffprobe, '--json', (Join-Path $resultDirectory 'verification.json'))
    [void](Invoke-Native -Executable $pythonProgram -Arguments $verifyArguments -WorkingDirectory $repo -LogPrefix (Join-Path $resultDirectory 'verify') -Environment $uncaptured)
    $sceneArguments = $pythonPrefix + @((Join-Path $PSScriptRoot 'verify_scene.py'), '--video', $video,
        '--width', [string]$Width, '--height', [string]$Height, '--fps', [string]$Fps, '--frames', [string]$Frames,
        '--ffmpeg', $ffmpeg, '--json', (Join-Path $resultDirectory 'scene-verification.json'))
    [void](Invoke-Native -Executable $pythonProgram -Arguments $sceneArguments -WorkingDirectory $repo -LogPrefix (Join-Path $resultDirectory 'verify-scene') -Environment $uncaptured)
    Write-Host "Verified Vulkan Video PoC. Results: $resultDirectory"
    Write-Output $resultDirectory
} finally {
    Restore-Environment $savedEnvironment $environmentNames
}
