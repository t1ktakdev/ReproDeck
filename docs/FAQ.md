# ReproDeck FAQ

## What is ReproDeck?

ReproDeck is an evidence-first desktop debugging workbench for AI-assisted code fixes. It captures real failures, links claims to inspectable evidence, tests one intervention in an isolated Git worktree, and requires Before/After plus required-regression proof for the exact patch before Apply is available.

## Why not just use Claude Code, Codex, or Cursor?

Those tools can be excellent at proposing code and reasoning about a repository. ReproDeck addresses a different boundary: whether the proposed change fixes the observed failure, whether the evidence supports the claim, and whether the exact patch still satisfies the required checks at Apply time. It complements a coding agent rather than competing to be the same kind of tool.

## Does ReproDeck replace my coding agent?

No. Use the model or coding agent you already trust. ReproDeck can also run its deterministic investigation and verification workflow without AI.

## Can I use ReproDeck with LM Studio?

Yes. Enable AI and configure an OpenAI-compatible endpoint such as `http://127.0.0.1:1234/v1`. Use the exact model identifier loaded in LM Studio. Qwen2.5 Coder 7B Instruct has been exercised in the Windows v0.3.0 field flow, but ReproDeck does not depend on that model.

## Can I use ReproDeck without AI?

Yes. Project Intelligence, Project Health, Bug Hunter, evidence, manual hypotheses, causal experiments, Fix Workspaces, Before/After verification, regression gates, Apply, recovery, and capsules are deterministic product capabilities. AI is off by default.

## What exactly does “Verified Fix” mean?

It means a specific patch is bound to a recorded source commit and working/index state, a concrete failing Before criterion, a passing After receipt, and all required regression receipts. ReproDeck records the binary patch SHA-256. Any drift in those inputs invalidates Ready to Apply.

Verified Fix does not mean that every possible behavior of the project is correct. It means the recorded criterion and required regression contract passed for the identified patch.

## When does ReproDeck modify my repository?

Opening, discovery, health checks, investigation, experiments, and verification do not modify the original working tree. ReproDeck uses separate Git worktrees for controlled execution and candidate changes. The original repository is changed only after an explicit Apply action passes the backend gate. ReproDeck does not automatically commit or push.

## What does a causal experiment prove?

It tests whether one reviewed intervention changes the selected outcome under recorded conditions. A supported hypothesis is useful evidence, not yet a Verified Fix. The candidate must still pass the separate exact-patch Before/After and regression workflow.

## Does ReproDeck sandbox project code?

No. A Git worktree isolates repository writes, not operating-system access. Project commands run with the current user's permissions and may access resources that user can access. Run only repositories and commands you trust.

## What data can be sent to an AI provider?

Only after explicit action, ReproDeck can send selected project facts, the investigation question, and bounded source/evidence snippets shown in the context view. Known secret-like paths are excluded before reading, and local paths and text are redacted. Redaction reduces risk but is not a guarantee; review context before sending it to any provider.

## Does ReproDeck collect telemetry?

No. ReproDeck has no telemetry or ads. Data is stored locally. GitHub and AI providers are optional, explicit network integrations.

## Why is the Windows installer unsigned?

The v0.3.0 community build does not yet have an Authenticode code-signing certificate. Windows may display a publisher warning. Download only from the official GitHub Release and verify the file against the published `SHA256SUMS.txt`.

## Does ReproDeck support Linux or macOS?

Windows 11 x64 is the runtime-tested v0.3.0 platform. Linux participates in source-level Core CI, but desktop runtime field testing is pending. macOS has not been runtime-tested.

## How do I report a bug?

Use the repository's [bug report form](https://github.com/t1ktakdev/ReproDeck/issues/new?template=bug_report.yml). Include the ReproDeck version, OS, reproduction steps, expected and actual behavior, and sanitized logs. Remove tokens, private paths, proprietary source, and other sensitive data before posting.

## How do I report a security issue?

Do not open a public issue for an active vulnerability. Use GitHub's private vulnerability reporting for the repository when available and follow [SECURITY.md](../SECURITY.md). Never include live credentials or unnecessary private source.
