param(
    [string]$Model = "",
    [int]$Port = 1234,
    [int]$ContextLength = 8192
)

$ErrorActionPreference = "Stop"

function Convert-LmsJson([object[]]$Lines) {
    $text = (($Lines | ForEach-Object { [string]$_ }) -join "`n").Trim()
    if (-not $text) { return @() }

    $candidates = @($text)

    $arrayStart = $text.IndexOf("[")
    $arrayEnd = $text.LastIndexOf("]")
    if ($arrayStart -ge 0 -and $arrayEnd -gt $arrayStart) {
        $candidates += $text.Substring($arrayStart, $arrayEnd - $arrayStart + 1)
    }

    $objectStart = $text.IndexOf("{")
    $objectEnd = $text.LastIndexOf("}")
    if ($objectStart -ge 0 -and $objectEnd -gt $objectStart) {
        $candidates += $text.Substring($objectStart, $objectEnd - $objectStart + 1)
    }

    foreach ($candidate in ($candidates | Select-Object -Unique)) {
        try {
            $parsed = $candidate | ConvertFrom-Json -ErrorAction Stop
            if ($parsed -is [System.Array]) { return @($parsed) }
            if ($null -ne $parsed.models) { return @($parsed.models) }
            if ($null -ne $parsed.data) { return @($parsed.data) }
            if ($null -ne $parsed.items) { return @($parsed.items) }
            return @($parsed)
        } catch {
            # Try the next JSON-shaped candidate. LM Studio may print a wake-up/status line first.
        }
    }

    throw "Could not parse JSON returned by LM Studio CLI. Raw output:`n$text"
}

function Convert-NativeText([object]$Value) {
    if ($null -eq $Value) { return "" }
    if ($Value -is [System.Array]) {
        return (($Value | ForEach-Object { if ($null -eq $_) { "" } else { [string]$_ } }) -join "`n")
    }
    return [string]$Value
}

function Invoke-LmsCommand {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$AllowFailure,
        [switch]$ShowOutput
    )

    $stderrFile = [System.IO.Path]::GetTempFileName()
    $oldPreference = $ErrorActionPreference
    $stdout = @()
    $exitCode = -1
    try {
        # Windows PowerShell 5 converts native stderr into ErrorRecord objects. LM Studio also writes
        # harmless lifecycle messages (for example "Waking up LM Studio service...") to stderr.
        # Redirect stderr to a file and decide success strictly from the native process exit code.
        $ErrorActionPreference = "Continue"
        $stdout = @(& lms @Arguments 2>$stderrFile)
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $oldPreference
    }

    $stderr = ""
    if (Test-Path -LiteralPath $stderrFile) {
        $stderrRaw = Get-Content -LiteralPath $stderrFile -Raw -ErrorAction SilentlyContinue
        $stderr = Convert-NativeText $stderrRaw
        Remove-Item -LiteralPath $stderrFile -Force -ErrorAction SilentlyContinue
    }

    $stdout = @($stdout | Where-Object { $null -ne $_ } | ForEach-Object { [string]$_ })
    $stderrHasText = -not [string]::IsNullOrWhiteSpace($stderr)

    if ($ShowOutput) {
        if (@($stdout).Count -gt 0) { $stdout | Out-Host }
        if ($stderrHasText) { Write-Host $stderr.TrimEnd() -ForegroundColor DarkGray }
    }

    if ($exitCode -ne 0 -and -not $AllowFailure) {
        $stdoutText = Convert-NativeText $stdout
        $detail = if ($stderrHasText) { $stderr.Trim() } elseif (-not [string]::IsNullOrWhiteSpace($stdoutText)) { $stdoutText.Trim() } else { "No diagnostic output." }
        throw "lms $($Arguments -join ' ') failed with exit code $exitCode. $detail"
    }

    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = @($stdout)
        Error = [string]$stderr
    }
}

function Test-LmStudioApi([string]$BaseUrl) {
    try {
        $null = Invoke-RestMethod -Method Get -Uri "$BaseUrl/models" -TimeoutSec 2
        return $true
    } catch {
        return $false
    }
}

function Wait-LmStudioApi([string]$BaseUrl, [int]$Attempts = 30) {
    for ($i = 0; $i -lt $Attempts; $i++) {
        if (Test-LmStudioApi $BaseUrl) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

if (-not (Get-Command lms -ErrorAction SilentlyContinue)) {
    throw "LM Studio CLI 'lms' is not in PATH. Official install command: npx lmstudio install-cli"
}

Write-Host "=== LM Studio CLI ===" -ForegroundColor Cyan
$null = Invoke-LmsCommand -Arguments @("--version") -ShowOutput

Write-Host "`n=== Downloaded LLMs ===" -ForegroundColor Cyan
$listResult = Invoke-LmsCommand -Arguments @("ls", "--llm", "--json")
$models = Convert-LmsJson $listResult.Output
if ($models.Count -eq 0) {
    throw "LM Studio has no downloaded LLMs. Download at least one local model first."
}

$gpuMiB = 0
if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
    $gpuRaw = & nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits 2>$null | Select-Object -First 1
    if ($gpuRaw -match '^\s*(\d+)') { $gpuMiB = [int]$Matches[1] }
}
$budgetBytes = if ($gpuMiB -gt 0) { [int64]($gpuMiB * 1MB * 0.72) } else { [int64](6GB) }

function Get-ModelScore($m) {
    $text = ("{0} {1} {2}" -f $m.modelKey, $m.displayName, $m.path).ToLowerInvariant()
    $score = 0
    if ($text -match 'coder') { $score += 120 }
    if ($text -match 'qwen3') { $score += 55 }
    elseif ($text -match 'qwen2\.5') { $score += 45 }
    elseif ($text -match 'qwen') { $score += 30 }
    if ($text -match 'deepseek') { $score += 35 }
    if ($text -match 'code') { $score += 35 }
    if ($text -match 'instruct') { $score += 15 }
    if ($m.trainedForToolUse -eq $true) { $score += 20 }
    $size = if ($null -ne $m.sizeBytes) { [int64]$m.sizeBytes } else { [int64]0 }
    if ($size -gt 0 -and $size -le $budgetBytes) { $score += 35 }
    elseif ($size -gt $budgetBytes) { $score -= 90 }
    if ($size -ge 2GB -and $size -le 6GB) { $score += 20 }
    return $score
}

$ranked = @($models | ForEach-Object {
    [pscustomobject]@{
        Model = $_
        Key = $_.modelKey
        Name = $_.displayName
        Params = $_.paramsString
        SizeGB = if ($_.sizeBytes) { [math]::Round([double]$_.sizeBytes / 1GB, 2) } else { 0 }
        Context = $_.maxContextLength
        ToolUse = $_.trainedForToolUse
        FitsBudget = (-not $_.sizeBytes) -or ([int64]$_.sizeBytes -le $budgetBytes)
        Score = Get-ModelScore $_
    }
} | Sort-Object @{Expression="Score"; Descending=$true}, @{Expression="SizeGB"; Descending=$true})

$ranked | Select-Object -First 12 Key, Name, Params, SizeGB, Context, ToolUse, FitsBudget, Score | Format-Table -AutoSize | Out-Host

if (-not [string]::IsNullOrWhiteSpace($Model)) {
    $selected = $models | Where-Object { $_.modelKey -eq $Model -or $_.displayName -eq $Model } | Select-Object -First 1
    if (-not $selected) { throw "Requested model was not found: $Model" }
} else {
    $eligible = @($ranked | Where-Object { $_.FitsBudget })
    if ($eligible.Count -gt 0) {
        $selected = ($eligible | Select-Object -First 1).Model
    } else {
        Write-Host "No model fits the conservative VRAM budget; selecting the smallest local LLM." -ForegroundColor Yellow
        $selected = ($ranked | Sort-Object SizeGB | Select-Object -First 1).Model
    }
}

if (-not $selected.modelKey) {
    throw "LM Studio returned a model without modelKey; cannot load it safely."
}

$actualContextLength = $ContextLength
if ($selected.maxContextLength -and [int64]$selected.maxContextLength -gt 0 -and [int64]$selected.maxContextLength -lt $actualContextLength) {
    $actualContextLength = [int]$selected.maxContextLength
}

Write-Host "`nSelected model: $($selected.modelKey)" -ForegroundColor Green
if ($gpuMiB -gt 0) { Write-Host "Detected NVIDIA VRAM: $gpuMiB MiB" }
Write-Host "Requested context: $ContextLength tokens"
if ($actualContextLength -ne $ContextLength) {
    Write-Host "Using context: $actualContextLength tokens (model maximum)" -ForegroundColor Yellow
}

$baseUrl = "http://127.0.0.1:$Port/v1"
Write-Host "`n=== Starting LM Studio server ===" -ForegroundColor Cyan
if (Test-LmStudioApi $baseUrl) {
    Write-Host "LM Studio API is already reachable at $baseUrl" -ForegroundColor DarkGreen
} else {
    $serverResult = Invoke-LmsCommand -Arguments @("server", "start", "--port", [string]$Port) -AllowFailure -ShowOutput
    if (-not (Wait-LmStudioApi $baseUrl)) {
        $serverError = [string]$serverResult.Error
        $serverOutput = Convert-NativeText $serverResult.Output
        $detail = if (-not [string]::IsNullOrWhiteSpace($serverError)) { $serverError.Trim() } elseif (-not [string]::IsNullOrWhiteSpace($serverOutput)) { $serverOutput.Trim() } else { "No diagnostic output." }
        throw "LM Studio server did not become reachable at $baseUrl. CLI exit code: $($serverResult.ExitCode). $detail"
    }
}

Write-Host "`n=== Loading selected model ===" -ForegroundColor Cyan
$null = Invoke-LmsCommand -Arguments @("unload", "reprodeck-local") -AllowFailure
$loadResult = Invoke-LmsCommand -Arguments @(
    "load",
    [string]$selected.modelKey,
    "--gpu", "max",
    "--context-length", [string]$actualContextLength,
    "--identifier", "reprodeck-local"
) -ShowOutput

if (-not (Wait-LmStudioApi $baseUrl 10)) {
    throw "LM Studio API stopped responding after model load."
}

Write-Host "`n=== API model list ===" -ForegroundColor Cyan
$apiModels = Invoke-RestMethod -Method Get -Uri "$baseUrl/models" -TimeoutSec 15
$apiModels.data | Select-Object id, object | Format-Table -AutoSize | Out-Host

Write-Host "`n=== Smoke inference ===" -ForegroundColor Cyan
$body = @{
    model = "reprodeck-local"
    temperature = 0
    max_tokens = 80
    messages = @(
        @{ role = "system"; content = "Reply with exactly: REPRODECK_AI_READY" },
        @{ role = "user"; content = "Connectivity test" }
    )
} | ConvertTo-Json -Depth 8
$response = Invoke-RestMethod -Method Post -Uri "$baseUrl/chat/completions" -ContentType "application/json" -Body $body -TimeoutSec 120
$text = [string]$response.choices[0].message.content
if (-not $text.Trim()) {
    throw "LM Studio returned an empty smoke-test response."
}
Write-Host $text

Write-Host "`n=== ReproDeck settings ===" -ForegroundColor Green
Write-Host "Enable AI: ON"
Write-Host "Base URL : $baseUrl"
Write-Host "Model    : reprodeck-local"
Write-Host "API key  : leave empty"
Write-Host "`nLM Studio is ready for ReproDeck." -ForegroundColor Green
