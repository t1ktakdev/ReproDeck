param(
    [string]$Path = "$env:USERPROFILE\Desktop\ReproDeck-Demo-Fixture",
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$fixtureScript = Join-Path $PSScriptRoot "create-ai-root-cause-fixture.ps1"

if ($Force) {
    & $fixtureScript -Path $Path -Force
} else {
    & $fixtureScript -Path $Path
}

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
