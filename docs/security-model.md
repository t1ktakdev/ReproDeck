# Security model

ReproDeck treats repositories, command output, capsules and model endpoints as untrusted inputs.

## Invariants

1. The original working tree is not modified during reproduction or fixing.
2. Apply is explicit and backend-gated on a verified Before/After outcome and all required regressions.
3. A passing After persists the exact binary patch SHA-256, source commit, source working/index-state hash, shadow commit, criterion and command identity. Any later byte or source-state change blocks Apply until verification is rerun.
4. User changes are never silently overwritten; HEAD/patch conflicts stop Apply.
4. Commands are represented as executable + argv. Shell wrappers are not the normal execution path.
5. Privilege escalation, destructive disk operations and dangerous Git operations against the original repo are denied by policy.
6. No telemetry, analytics SDK, automatic upload, automatic commit or automatic push.

## Paths and artifacts

Artifact keys must be relative normal components. Reads canonicalize both the store and candidate path, reject escape, symlinks/reparse points and checksum mismatches. Secret-like paths are excluded from exported diffs.

Apply also validates every changed Git path as a normal relative UTF-8 path and refuses a target whose existing path chain contains a symbolic link or Windows reparse point. Git's atomic `apply --check` preflight is used without `--reject` or `--unsafe-paths`; submodule and symlink-mode patches are rejected.

## Investigation handoff and Apply

Investigation Fix Workspace has no Apply entry point. A supported experiment may produce a handoff candidate, but Core first checks its recorded intervention SHA-256 and source commit, then runs `git apply --check` against an empty, separate verification shadow. The candidate remains staged until Before proves the baseline; only then is it applied and checkpointed inside that verification workspace.

Ready to Apply is recomputed rather than trusted as a label. Core compares the current patch and source state with the persisted proof and verifies that every Required regression passed against the same patch SHA-256. Apply repeats the source-state and patch-hash comparison inside the mutation primitive before Git preflight, closing the UI/backend and verify/apply mismatch paths.

## Redaction

Text is redacted before persistence where applicable. Rules cover common credential names, authorization headers and token patterns. Redaction is defense-in-depth rather than a promise that arbitrary unknown secret formats can always be detected; users should inspect export summaries before sharing.

## Capsules

Import validates the exact archive entry set, normalized relative paths, symlink status, per-entry size, aggregate uncompressed size, manifest version, `checksums.json`, individual SHA-256 values and duplicate/undeclared entries. Import validates and copies from the same open file handle to avoid path-swap TOCTOU.

## Network integrations

GitHub operations use the installed `gh` executable and require confirmation. Draft PR creation additionally requires a clean working tree and supplies an explicit `--head` branch so `gh` does not enter its fork/push prompt flow; ReproDeck never commits or pushes. AI requests are disabled by default, require a runtime API key and explicit network confirmation, and cannot set verification state. Provider base URLs are parsed as URLs and must use HTTP or HTTPS with a host; embedded credentials, query strings and fragments are rejected before any request.

## Recovery cleanup

An Apply that succeeds before shadow cleanup finishes is recorded separately from a failed Apply, preventing a second application of the same patch. Recovery retry is explicit and idempotent. Before invoking Git cleanup it requires a `reprodeck-shadow-*` branch, a `reprodeck-shadow-*` worktree directory and a canonical path inside the operating-system temporary directory. Unit tests use an isolated process-local recovery database rather than the user's application data.

## Reporting vulnerabilities

Please follow `SECURITY.md`. Do not attach real credentials, private source archives or unredacted logs to public reports.

## Project discovery and model context

Opening or rescanning a project is read-only and never executes repository scripts. Discovery is bounded, skips generated/vendor directories, does not follow symlinks, respects Git-ignore decisions when available and excludes secret-like paths before reading their content. Detected commands are inert metadata until a later permission-gated execution flow explicitly selects them.

Context Compiler uses independent file/character budgets, rejects secret-like/symlink/binary/oversized candidates and redacts selected text. The local filesystem root is not included in model prompt text. Model output is treated as a hypothesis and cannot mutate verification state.
