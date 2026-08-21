param(
    [string]$Model = "",
    [int]$Port = 1234,
    [int]$ContextLength = 8192,
    [switch]$ForceFixture
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot

& "$PSScriptRoot\start-lmstudio-reprodeck.ps1" -Model $Model -Port $Port -ContextLength $ContextLength
& "$PSScriptRoot\create-ai-root-cause-fixture.ps1" -Force:$ForceFixture

Write-Host "`nFixture: $HOME\Desktop\ReproDeck-AI-RootCause-Fixture" -ForegroundColor Cyan
Write-Host "In ReproDeck: Settings -> AI -> LM Studio -> Base URL http://127.0.0.1:$Port/v1 -> Model reprodeck-local -> Test connection -> Save." -ForegroundColor Yellow
Write-Host "Then open the fixture, run Smart Bug Hunter, Start case, Compile context, and Generate hypotheses with AI." -ForegroundColor Yellow

Push-Location $Root
try {
    npm run tauri dev
} finally {
    Pop-Location
}
