# SPDX-License-Identifier: Apache-2.0
param(
    [string]$Makensis,
    [switch]$NoPackage,
    [switch]$StageOnly,
    [switch]$Offline,
    [ValidateRange(1,8)][int]$Jobs = 1,
    [string[]]$ToolDirectories = @()
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildRoot = Join-Path $projectRoot '.build'
$temporaryRoot = Join-Path $buildRoot 'tmp'
$cargoRoot = Join-Path $buildRoot 'cargo'
New-Item -ItemType Directory -Path $temporaryRoot,$cargoRoot -Force | Out-Null
$original = @{}
foreach ($name in @('CARGO_HOME','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','TEMP','TMP','TMPDIR','PYTHONDONTWRITEBYTECODE','PATH')) {
    $original[$name] = [Environment]::GetEnvironmentVariable($name,'Process')
}
Push-Location $projectRoot
$buildProcess = [Diagnostics.Process]::GetCurrentProcess()
$originalPriority = $buildProcess.PriorityClass
try {
    $buildProcess.PriorityClass = [Diagnostics.ProcessPriorityClass]::BelowNormal
    $env:CARGO_HOME = $cargoRoot
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
    $env:CARGO_BUILD_JOBS = [string]$Jobs
    $env:TEMP = $temporaryRoot
    $env:TMP = $temporaryRoot
    $env:TMPDIR = $temporaryRoot
    $env:PYTHONDONTWRITEBYTECODE = '1'
    foreach ($directory in $ToolDirectories) {
        if (-not [IO.Path]::IsPathFullyQualified($directory) -or -not (Test-Path -LiteralPath $directory -PathType Container)) {
            throw 'ToolDirectories must be explicit existing absolute directories.'
        }
    }
    if ($ToolDirectories.Count) { $env:PATH = ($ToolDirectories -join [IO.Path]::PathSeparator) + [IO.Path]::PathSeparator + $env:PATH }
    & python -B scripts/collect-native-notices.py
    if ($LASTEXITCODE -ne 0) { throw 'Native dependency notices require review; no build or package was started' }
    & python -B scripts/package-native.py --prepare-manifest
    if ($LASTEXITCODE -ne 0) { throw 'Locked native resources or dependency notices are incomplete' }
    if (-not $StageOnly) {
        $cargoArguments = @('build','--locked','--release','--jobs',[string]$Jobs,'--bin','flow-desktop','--bin','flow-cli')
        if ($Offline) { $cargoArguments += '--offline' }
        & cargo @cargoArguments
        if ($LASTEXITCODE -ne 0) { throw 'Native release build failed' }
    }
    & python -B scripts/package-native.py --stage
    if ($LASTEXITCODE -ne 0) { throw 'Native staging failed' }
    & python -B scripts/package-native.py --audit
    if ($LASTEXITCODE -ne 0) { throw 'Native package audit failed' }
    if (-not $NoPackage) {
        if (-not $Makensis) { throw 'Pass an existing -Makensis path, or use -NoPackage. No system tools will be installed.' }
        & python -B scripts/package-native.py --compile --makensis $Makensis
        if ($LASTEXITCODE -ne 0) { throw 'Native installer compile failed' }
    }
} finally {
    try {
        $buildProcess.PriorityClass = $originalPriority
    } finally {
        Pop-Location
        foreach ($name in $original.Keys) {
            [Environment]::SetEnvironmentVariable($name,$original[$name],'Process')
        }
    }
}
