---
description: Rare maximum-depth GPT-5 Mini implementation mode. Uses GPT-OSS-120B architect/debugger as a second brain.
mode: primary
model: azure-gpt5mini/gpt-5-mini
reasoningEffort: high
textVerbosity: low
temperature: 0.1
permission:
  read:
    "*": allow
    ".env": deny
    ".env.*": deny
    "*.pem": deny
    "*.key": deny
  edit:
    "*": allow
    "AGENTS.md": deny
    "scripts/quality-gate.ps1": deny
    ".opencode/agents/*": deny
    ".opencode/commands/*": deny
    ".env": deny
    ".env.*": deny
    "*.pem": deny
    "*.key": deny
  bash:
    "*": allow
    "git commit *": deny
    "git push *": deny
    "git reset --hard *": deny
    "git clean *": deny
    "diskpart *": deny
    "format *": deny
    "sudo *": deny
    "doas *": deny
  task:
    "*": deny
    "repro-architect": allow
    "repro-debugger": allow
    "repro-researcher": allow
    "repro-reviewer": allow
    "repro-security": allow
    "explore": allow
    "scout": allow
  external_directory: deny
  webfetch: allow
  websearch: allow
  skill: allow
---

Use only for genuinely hard implementation. Read durable state/current diff first. Invoke @repro-architect for the hard design/risk analysis, then implement yourself. Use @repro-debugger for stubborn failures and @repro-researcher for unstable APIs. Keep prose concise despite HIGH reasoning. Continue autonomously through the active checkpoint. Never modify control/gate files. Do not commit/push.