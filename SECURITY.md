# Security policy

## Reporting a vulnerability

Do not open a public issue for an active or suspected vulnerability. Use the repository's **Private vulnerability reporting** option under the Security tab when it is available.

Include only the minimum sanitized information needed to reproduce the problem:

- affected ReproDeck version and platform;
- affected boundary, such as Apply, command runner, worktree/path handling, evidence, redaction, capsule import/export, recovery, or network integration;
- clear reproduction steps and observed impact;
- whether the original repository changed unexpectedly;
- a minimal fixture or redacted receipt when it is safe to share.

Do not include live credentials, private repository archives, proprietary source, personal paths, or unredacted `.reprodeck` capsules. If a credential may have been exposed, revoke or rotate it before reporting.

No public security email address is currently published. If GitHub private reporting is unavailable, open a minimal public issue that asks the maintainer to establish a private channel without disclosing vulnerability details.

## Supported release

Security fixes target the latest published release. The current release line is v0.3.x.

## Security model and limitations

ReproDeck uses isolated Git worktrees, path validation, explicit confirmations, secret exclusion/redaction, evidence validation, exact-patch identity, required regression gates, and a backend-controlled Apply boundary. AI output is a proposal and cannot establish proof.

A Git worktree is not an operating-system sandbox. Project commands run with the current user's permissions and can access resources available to that user. Run only code you trust. Redaction reduces accidental disclosure risk but cannot guarantee that arbitrary source or output contains no sensitive information; review AI context and exports before sending or sharing them.

ReproDeck deliberately has no telemetry, automatic repository upload, automatic commit, automatic push, or automatic Apply. Read the full threat model in [docs/security-model.md](docs/security-model.md).
