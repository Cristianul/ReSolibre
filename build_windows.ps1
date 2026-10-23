<#
.SYNOPSIS
Builds the native x64 Windows player using the shared locked Cargo build setup.
.DESCRIPTION
Requires Python 3.9+, x64 MSVC Rust, Visual C++ Build Tools and Windows SDK.
Place libmpv-2.dll, ffmpeg.exe and ffprobe.exe in .local/native/windows, or set
RESOLIBRE_NATIVE_DIR. Dependencies are never downloaded or installed here.
.EXAMPLE
.\build_windows.ps1 -Profile Release -Jobs 4 -Run
#>
[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Profile = 'Debug',
    [ValidateRange(1, 1024)]
    [int]$Jobs,
    [switch]$Offline,
    [switch]$Run,
    [string]$Python
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') {
    Write-Error 'This entry point requires Windows; use build_debug.py or build_release.py on Linux.'
    exit 1
}

$pythonArguments = @()
if ($Python) {
    $interpreter = $Python
} elseif (Get-Command py.exe -ErrorAction SilentlyContinue) {
    $interpreter = 'py.exe'
    $pythonArguments += '-3'
} elseif (Get-Command python.exe -ErrorAction SilentlyContinue) {
    $interpreter = 'python.exe'
} else {
    Write-Error 'Python 3.9+ is required. See docs/development/local-builds.md.'
    exit 1
}

$entryPoint = Join-Path $PSScriptRoot ('build_' + $Profile.ToLowerInvariant() + '.py')
$pythonArguments += $entryPoint
if ($Offline) { $pythonArguments += '--offline' }
if ($Run) { $pythonArguments += '--run' }
if ($PSBoundParameters.ContainsKey('Jobs')) { $pythonArguments += @('--jobs', [string]$Jobs) }

try {
    & $interpreter @pythonArguments
    exit $LASTEXITCODE
} catch {
    Write-Error 'Cannot run Python. Use -Python with its full executable path, or install Python 3.9+.'
    exit 1
}
