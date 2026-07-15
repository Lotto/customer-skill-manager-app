<#
.SYNOPSIS
  Run a cargo/tauri command with a fully-wired MSVC + Windows SDK environment.

.DESCRIPTION
  On some Visual Studio installs the C++ toolset (VC.Tools) is present but not
  paired with a Windows SDK component. rustc/cc then find the VC libs/headers
  but NOT the SDK ones, so builds fail with:
    - LNK1181: cannot open input file 'kernel32.lib'   (SDK libs missing)
    - C1083:   Cannot open include file 'windows.h'     (SDK headers missing)

  The clean fix is to install the "Windows 11 SDK" component in the Visual
  Studio Installer. Failing that, this script injects the SDK paths directly:
    - SDK libs     -> CARGO_ENCODED_RUSTFLAGS (-L native=...)  for rustc linking
    - SDK includes -> CXXFLAGS/CFLAGS (/I...)                  for cc compiling
  and initializes the VS Developer environment for the VC toolset itself.

.EXAMPLE
  ./scripts/build-local.ps1 "npx tauri dev"     # the Tauri CLI is the npm dep
  ./scripts/build-local.ps1 "npx tauri build"
  ./scripts/build-local.ps1 "cargo build -p customer-skill-manager"
#>
param(
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
  [string[]] $Command
)

$ErrorActionPreference = "Stop"

# --- Locate Visual Studio and enter its Developer environment (VC toolset) ---
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsPath = $null
if (Test-Path $vswhere) {
  $vsPath = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath
}
if (-not $vsPath) { $vsPath = "C:\Program Files\Microsoft Visual Studio\2022\Community" }

$devShell = Join-Path $vsPath "Common7\Tools\Launch-VsDevShell.ps1"
if (-not (Test-Path $devShell)) {
  throw "Launch-VsDevShell.ps1 not found under '$vsPath'. Install the C++ build tools via the Visual Studio Installer."
}
Write-Host "Initializing MSVC environment from: $vsPath" -ForegroundColor Cyan
& $devShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null

# --- Find the newest Windows SDK that actually has the libs on disk ---
$kitsRoot = (Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots" -ErrorAction SilentlyContinue).KitsRoot10
if (-not $kitsRoot) { $kitsRoot = "C:\Program Files (x86)\Windows Kits\10\" }
$libRoot = Join-Path $kitsRoot "Lib"
$incRoot = Join-Path $kitsRoot "Include"

$sdkVer = Get-ChildItem $libRoot -Directory -ErrorAction SilentlyContinue |
  Where-Object { Test-Path (Join-Path $_.FullName "um\x64\kernel32.lib") } |
  Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name

if ($sdkVer) {
  Write-Host "Injecting Windows SDK $sdkVer paths (workaround for unpaired SDK)." -ForegroundColor Cyan
  $fso = New-Object -ComObject Scripting.FileSystemObject
  $short = { param($p) $fso.GetFolder($p).ShortPath }  # 8.3 path: no spaces for CXXFLAGS

  $umLib   = Join-Path $libRoot "$sdkVer\um\x64"
  $ucrtLib = Join-Path $libRoot "$sdkVer\ucrt\x64"
  $us = [string][char]0x1f  # rustflags encoded separator (paths may contain spaces)
  $env:CARGO_ENCODED_RUSTFLAGS = "-Lnative=$umLib" + $us + "-Lnative=$ucrtLib"

  $incFlags = @("um", "shared", "ucrt") | ForEach-Object {
    "/I" + (& $short (Join-Path $incRoot "$sdkVer\$_"))
  }
  $env:CXXFLAGS = ($incFlags -join " ")
  $env:CFLAGS = $env:CXXFLAGS
} else {
  Write-Warning "No Windows SDK libs found under $libRoot. If the build fails with LNK1181/C1083, install the 'Windows 11 SDK' component in the Visual Studio Installer."
}

Set-Location (Split-Path $PSScriptRoot -Parent)
$cmd = ($Command -join " ")
Write-Host "> $cmd" -ForegroundColor Green
Invoke-Expression $cmd
exit $LASTEXITCODE
