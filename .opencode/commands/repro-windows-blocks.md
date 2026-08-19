---
description: Diagnose why Windows blocks OpenCode/test commands without disabling security
agent: repro-debugger
subtask: false
---
Run powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/diagnose-windows-blocks.ps1. Read the generated report in Downloads if accessible, identify the exact policy/provider/event that blocked execution, and propose the narrowest safe fix or invocation workaround. Do not disable Defender/AppLocker/WDAC/ASR globally.