param([switch]$SkipInstall)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'verify-windows.ps1') -SkipInstall:$SkipInstall
