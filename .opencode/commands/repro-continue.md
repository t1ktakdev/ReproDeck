---
description: Autonomous dual-brain continuation: GPT-5 Mini builds, GPT-OSS-120B reasons/researches/reviews when useful.
agent: repro-builder
---
Resume from CURRENT workspace state. Read AGENTS.md, docs/agent-state.md, docs/agent-routing.md, active OpenSpec tasks and git diff. Continue the next unblocked task automatically; do not ask me to say "continue".

Use GPT-5 Mini for implementation. Delegate only the difficult thinking:
- repro-architect for hard architecture/invariants/platform/concurrency/data-safety decisions;
- repro-debugger for stubborn failures after one focused attempt;
- repro-researcher for uncertain/current APIs;
- repro-reviewer at checkpoint;
- repro-security at security-sensitive checkpoint.

Keep GPT-OSS memos compact. Use targeted tests while iterating and the external locked gate at checkpoint. Fix BLOCKER/HIGH, rerun gate/review, update docs/agent-state.md, and continue until the current checkpoint is genuinely complete or there is a real external blocker. Do not commit or push.