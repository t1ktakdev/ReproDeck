---
description: Run locked non-bypassable ReproDeck gate with real tests
agent: repro-reviewer
---
Run powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$env:USERPROFILE\.config\opencode\reprodeck-locked-quality-gate.ps1" -ProjectPath "$PWD". Report the exact failing stage. Do not edit anything and do not claim success unless it ends with QUALITY_GATE_PASS.