$ErrorActionPreference = "Continue"
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$Out = Join-Path $env:USERPROFILE "Downloads\ReproDeck-Windows-Blocks-$Stamp.txt"

function Section([string]$Name) {
  "`r`n============================================================" | Out-File $Out -Append -Encoding utf8
  $Name | Out-File $Out -Append -Encoding utf8
  "============================================================" | Out-File $Out -Append -Encoding utf8
}

"ReproDeck Windows execution-block diagnostics" | Out-File $Out -Encoding utf8
("Generated: " + (Get-Date)) | Out-File $Out -Append -Encoding utf8

Section "PowerShell execution policy"
Get-ExecutionPolicy -List | Format-Table -AutoSize | Out-String | Out-File $Out -Append -Encoding utf8

Section "Windows / machine"
Get-CimInstance Win32_OperatingSystem |
  Select-Object Caption,Version,BuildNumber |
  Format-List | Out-String | Out-File $Out -Append -Encoding utf8

Section "AppLocker effective policy (if available)"
try {
  Get-AppLockerPolicy -Effective -Xml | Out-File $Out -Append -Encoding utf8
} catch {
  ("AppLocker policy unavailable: " + $_.Exception.Message) | Out-File $Out -Append -Encoding utf8
}

Section "Defender ASR configuration (if available)"
try {
  $mp = Get-MpPreference
  [pscustomobject]@{
    DisableRealtimeMonitoring = $mp.DisableRealtimeMonitoring
    ASR_RuleIds = ($mp.AttackSurfaceReductionRules_Ids -join ", ")
    ASR_Actions = ($mp.AttackSurfaceReductionRules_Actions -join ", ")
  } | Format-List | Out-String | Out-File $Out -Append -Encoding utf8
} catch {
  ("Defender preference unavailable: " + $_.Exception.Message) | Out-File $Out -Append -Encoding utf8
}

$logs = @(
  "Microsoft-Windows-AppLocker/EXE and DLL",
  "Microsoft-Windows-AppLocker/MSI and Script",
  "Microsoft-Windows-CodeIntegrity/Operational",
  "Microsoft-Windows-Windows Defender/Operational"
)
foreach ($log in $logs) {
  Section ("Recent events: " + $log)
  try {
    Get-WinEvent -LogName $log -MaxEvents 60 -ErrorAction Stop |
      Where-Object {
        $_.LevelDisplayName -in @("Error","Warning") -or
        $_.Message -match "(?i)block|blocked|блок|deny|denied|AppLocker|Code Integrity|ASR"
      } |
      Select-Object TimeCreated,Id,LevelDisplayName,ProviderName,Message |
      Format-List | Out-String -Width 220 |
      Out-File $Out -Append -Encoding utf8
  } catch {
    ("Could not read log: " + $_.Exception.Message) | Out-File $Out -Append -Encoding utf8
  }
}

Section "Important note"
@"
This report changes NOTHING. It does not disable Defender, AppLocker, WDAC, ASR or PowerShell policy.
Use the event ID/provider/message to determine exactly what Windows blocked before changing anything.
"@ | Out-File $Out -Append -Encoding utf8

Write-Host "Diagnostics saved to:" -ForegroundColor Green
Write-Host $Out