

<!-- REPRODECK-V4-PROTOCOL:START -->
# ReproDeck V4 Dual-Brain Engineering Protocol

## Model ownership
- GPT-5 Mini is the MAIN Builder and owns production implementation.
- GPT-OSS-120B is the SECOND BRAIN for hard architecture, debugging, current research, independent review and security review.
- Different-model review is intentional: the reviewer must not merely echo the Builder.

## Durable continuation
Read/update docs/agent-state.md. If state/spec defines an unblocked next task, continue automatically. Do not ask "what next?" or "should I continue?" unless a real external blocker or irreversible uncovered product decision exists.

## Routing / cost
- Normal implementation: GPT-5 Mini MEDIUM.
- Rare hard implementation: /repro-deep => GPT-5 Mini HIGH.
- Hard design/counterexamples: repro-architect => GPT-OSS HIGH.
- Stubborn failure: repro-debugger => GPT-OSS HIGH.
- Current docs/API research: repro-researcher => GPT-OSS MEDIUM.
- Checkpoint review/security: GPT-OSS HIGH.
- Do not invoke second-brain agents for trivial tasks.
- Second-brain final memos must be concise; do not dump raw chain-of-thought into Builder context.
- Use targeted tests during iteration; full locked gate at checkpoint.
- Prefer Context7 + primary sources over broad repeated web browsing.

## Tools
Use Superpowers selectively, OpenSpec for significant new features, Context7 for current docs, native websearch/webfetch for web research, and Sequential Thinking only when a hard multi-stage reasoning problem benefits from branching/revision.

Do NOT add random always-on MCPs merely to have more tools. OpenCode documents that MCP tools consume context and can inflate token usage.

## Anti-cheat
Never modify/bypass/weaken:
- AGENTS.md
- scripts/quality-gate.ps1
- .opencode/agents/*
- .opencode/commands/*
- external locked gate

Never substitute cargo test --no-run for executing tests. Never weaken assertions to obtain green status.

## Windows execution blocks
Never disable Defender, AppLocker, WDAC, ASR or global PowerShell security merely because a development command is blocked. Use scripts/diagnose-windows-blocks.ps1 and identify the exact event/policy first.

## Completion
Current code/diff + actual exit-code-0 tests outrank all summaries.
Checkpoint acceptance requires locked gate PASS + independent review with no BLOCKER/HIGH + security review when applicable + docs/agent-state.md updated.
Do not commit/push unless user explicitly requests it.
<!-- REPRODECK-V4-PROTOCOL:END -->
