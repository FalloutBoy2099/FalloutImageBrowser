<#
.SYNOPSIS
    Removes the Explorer thumbnail provider installed by install.ps1.

.EXAMPLE
    .\uninstall.ps1                  # unregister, keep the files
.EXAMPLE
    .\uninstall.ps1 -RemoveFiles     # also delete the installed DLL
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'FalloutImageBrowser\ShellExtension'),
    [switch]$RemoveFiles
)

$ErrorActionPreference = 'Stop'
$target = Join-Path $InstallDir 'fallout_thumbnails.dll'
if (-not (Test-Path $target)) { Write-Host "Nothing to uninstall ($target not found)."; return }

$p = Start-Process regsvr32.exe -ArgumentList '/s', '/u', "`"$target`"" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "regsvr32 /u failed (exit code $($p.ExitCode))" }
Write-Host 'Unregistered.' -ForegroundColor Green

if ($RemoveFiles) {
    for ($i = 1; ; $i++) {
        try { Remove-Item $target -Force; break }
        catch {
            if ($i -ge 10) { Write-Warning "$target is still in use by Explorer; delete it later (or sign out first)."; break }
            Start-Sleep -Seconds 1
        }
    }
    if ((Test-Path $InstallDir) -and -not (Get-ChildItem $InstallDir -Force)) { Remove-Item $InstallDir -Force }
}
