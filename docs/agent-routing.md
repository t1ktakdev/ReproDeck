# ReproDeck Dual-Brain Routing

## Main model
GPT-5 Mini = owner/Builder. Medium reasoning by default. It writes and integrates production code.

## GPT-OSS-120B second brain
Deployment: gpt-oss-120b-reprodeck
Resource: nexqwz-7867-resource

Use it for:
- epro-architect: hard design/invariants/counterexamples (HIGH)
- epro-debugger: stubborn failures/root cause (HIGH)
- epro-researcher: current docs/API research (MEDIUM)
- epro-reviewer: independent checkpoint review (HIGH)
- epro-security: hostile security review (HIGH)

Subagents return compact memos rather than giant reasoning dumps. This keeps GPT-5 Mini context smaller.

## Tool policy
Active extra tools are intentionally curated:
- Context7: current library documentation
- Sequential Thinking: only for genuinely difficult multi-stage reasoning
- OpenCode native websearch/webfetch
- OpenCode native file/grep/LSP/shell tools
- Superpowers methodology
- OpenSpec specs/tasks

OpenCode warns that too many MCP servers inflate context/token usage, so redundant filesystem/Git/GitHub-search MCPs are intentionally not always-on.

## Commands
- /repro-continue normal autonomous work
- /repro-deep rare GPT-5 Mini HIGH implementation
- /repro-architect ask GPT-OSS architect directly
- /repro-review independent GPT-OSS review
- /repro-security independent GPT-OSS security audit
- /repro-windows-blocks diagnose Windows command blocking without disabling security