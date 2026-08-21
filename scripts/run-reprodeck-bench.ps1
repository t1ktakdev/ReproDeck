param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\bench-results")
)

$ErrorActionPreference = "Stop"
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("reprodeck-bench-" + [guid]::NewGuid().ToString("N"))
$fixturePath = Join-Path $temporaryRoot "auth-cache"
$generator = Join-Path $PSScriptRoot "create-ai-root-cause-fixture.ps1"

function Invoke-BenchCommand([string]$Name, [string[]]$Arguments) {
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $lines = & npm.cmd @Arguments 2>&1 | ForEach-Object { $_.ToString() }
    $exitCode = $LASTEXITCODE
    $watch.Stop()
    $outputText = $lines -join "`n"
    if ($outputText.Length -gt 2000) { $outputText = $outputText.Substring(0, 2000) + "`n[truncated]" }
    [ordered]@{
        name = $Name
        executable = "npm"
        arguments = $Arguments
        exitCode = $exitCode
        durationMs = $watch.ElapsedMilliseconds
        outputPreview = $outputText
    }
}

New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
try {
    & $generator -Path $fixturePath
    Push-Location $fixturePath
    try {
        $headBefore = (git rev-parse HEAD).Trim()
        $stateBefore = git status --porcelain=v1
        $runs = @(
            Invoke-BenchCommand "check" @("run", "check")
            Invoke-BenchCommand "test" @("test")
            Invoke-BenchCommand "build" @("run", "build")
        )
        $headAfter = (git rev-parse HEAD).Trim()
        $stateAfter = git status --porcelain=v1
    }
    finally {
        Pop-Location
    }

    $expectationsMet = $runs[0].exitCode -eq 0 -and $runs[1].exitCode -ne 0 -and $runs[2].exitCode -eq 0
    $originalUnchanged = $headBefore -eq $headAfter -and ($stateBefore -join "`n") -eq ($stateAfter -join "`n")
    $result = [ordered]@{
        schemaVersion = 1
        caseId = "auth-cache-tenant-boundary"
        kind = "deterministic-fixture-baseline"
        createdAtUtc = [DateTime]::UtcNow.ToString("o")
        model = $null
        comparisonClaim = $null
        commands = $runs
        expectationsMet = $expectationsMet
        originalRepositoryUnchanged = $originalUnchanged
        sourceCommit = $headBefore
    }
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $resultPath = Join-Path $OutputDirectory "local-baseline.json"
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resultPath -Encoding utf8
    if (-not $expectationsMet -or -not $originalUnchanged) {
        throw "The deterministic benchmark baseline did not satisfy its manifest."
    }
    Write-Host "Benchmark baseline verified: $resultPath"
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        $resolved = (Resolve-Path -LiteralPath $temporaryRoot).Path
        $tempBoundary = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
        if ($resolved.StartsWith($tempBoundary, [System.StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf).StartsWith("reprodeck-bench-")) {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
}
