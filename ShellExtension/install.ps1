<#
.SYNOPSIS
    Builds and installs the Explorer thumbnail provider for Fallout .FRM and .RIX files (current user only, no admin rights).

.DESCRIPTION
    1. builds the DLL (cargo build --release) unless -NoBuild is given,
    2. copies it to %LOCALAPPDATA%\FalloutImageBrowser\ShellExtension (a copy is used so the build folder is never locked
       by Explorer's thumbnail host),
    3. registers it with regsvr32 for the current user (HKEY_CURRENT_USER).
    Run uninstall.ps1 to remove it. Thumbnails already cached by Windows may need a moment to be regenerated;
    if an old icon stays, restart Explorer (or sign out and in).

.EXAMPLE
    .\install.ps1
.EXAMPLE
    .\install.ps1 -NoBuild -Dll C:\path\to\fallout_thumbnails.dll
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'FalloutImageBrowser\ShellExtension'),
    # Use an already built DLL instead of target\release\fallout_thumbnails.dll.
    [string]$Dll,
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (-not $Dll) {
    if (-not $NoBuild) {
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)" }
    }
    $Dll = Join-Path $PSScriptRoot 'target\release\fallout_thumbnails.dll'
}
if (-not (Test-Path $Dll)) { throw "DLL not found: $Dll" }

New-Item -ItemType Directory -Force $InstallDir | Out-Null
$target = Join-Path $InstallDir 'fallout_thumbnails.dll'

# An older copy may be registered and loaded: unregister it first so the file can be replaced.
if (Test-Path $target) {
    Start-Process regsvr32.exe -ArgumentList '/s', '/u', "`"$target`"" -Wait | Out-Null
}
for ($i = 1; ; $i++) {
    try { Copy-Item $Dll $target -Force; break }
    catch {
        if ($i -ge 10) { throw "Cannot replace $target (still in use by Explorer?). Close Explorer windows or sign out and retry." }
        Start-Sleep -Seconds 1
    }
}

$p = Start-Process regsvr32.exe -ArgumentList '/s', "`"$target`"" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "regsvr32 failed (exit code $($p.ExitCode))" }
Write-Host "Installed: $target" -ForegroundColor Green
Write-Host '.FRM and .RIX files now get thumbnails in Explorer (palette: Fallout default.pal, first image of the first direction).'
