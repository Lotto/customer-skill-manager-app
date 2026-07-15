<#
.SYNOPSIS
  Run a cargo/tauri command with the Visual Studio (MSVC) build environment
  initialized, so native C/C++ dependencies (which Tauri needs) can compile.

.EXAMPLE
  ./scripts/build-local.ps1 "cargo tauri build"
  ./scripts/build-local.ps1 "cargo tauri dev"
  ./scripts/build-local.ps1 "cargo build -p customer-skill-manager"
#>
param(
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
  [string[]] $Command
)

$ErrorActionPreference = "Stop"

# Locate a Visual Studio installation with the C++ tools.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsPath = $null
if (Test-Path $vswhere) {
  $vsPath = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath
}
if (-not $vsPath) {
  # Fall back to the common Community path.
  $vsPath = "C:\Program Files\Microsoft Visual Studio\2022\Community"
}

$devShell = Join-Path $vsPath "Common7\Tools\Launch-VsDevShell.ps1"
if (-not (Test-Path $devShell)) {
  throw "Could not find Launch-VsDevShell.ps1 under '$vsPath'. Install the C++ build tools via the Visual Studio Installer."
}

Write-Host "Initializing MSVC environment from: $vsPath" -ForegroundColor Cyan
& $devShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null

if (-not $env:INCLUDE -or ($env:INCLUDE -notmatch "VC\\Tools\\MSVC")) {
  Write-Warning "MSVC toolset headers are not on INCLUDE. If compilation fails with C1083 (missing vcruntime.h/excpt.h), install 'Desktop development with C++' in the Visual Studio Installer."
}

Set-Location (Split-Path $PSScriptRoot -Parent)

$cmd = ($Command -join " ")
Write-Host "> $cmd" -ForegroundColor Green
Invoke-Expression $cmd
exit $LASTEXITCODE
