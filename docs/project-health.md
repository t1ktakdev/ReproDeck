# Project Health / Bug Hunter execution boundary

Project Health is ReproDeck's first deterministic Bug Hunter layer. It turns commands discovered by Project Intelligence into evidence without allowing discovery itself to execute repository code.

## Execution contract

1. Opening/analyzing a project is read-only.
2. Only deterministic command kinds (`build`, `test`, `lint`, `typecheck`, `check`) are eligible for automatic Project Health runs.
3. The user selects the checks and explicitly confirms code execution.
4. A Git repository with a committed `HEAD` is required.
5. ReproDeck records the original repository `HEAD`, porcelain working-tree status, and a SHA-256 of the complete tracked `git diff HEAD` state. The diff hash matters when a file was already dirty before the run: its porcelain status can stay `M` even if its contents change.
6. A disposable worktree is created from the committed `HEAD`. Uncommitted user changes are not copied into the run.
7. Each check is executed as executable + argv through the process runner. ReproDeck does not concatenate a shell command.
8. The child environment is cleared and rebuilt from a small runtime allow-list. Common token and command-injection environment variables are not inherited.
9. Per-check timeout and output limits are enforced. Persisted previews are redacted.
10. Dynamic package executors (`npx`, `npm exec`, `pnpm dlx`, `yarn dlx`) are blocked in Project Health because they can fetch and execute code outside the repository under a generic check confirmation.
11. For Node package-manager checks, a project that declares dependencies but has no dependency environment inside the disposable worktree is reported as **Blocked**, not **Failed**. ReproDeck deliberately does not turn "dependencies are missing" into a fake project bug and does not install packages automatically.
12. The disposable worktree is removed and the original repository Git state is compared with the pre-run snapshot.

A worktree is a repository-isolation boundary, **not an operating-system sandbox**. Project code still runs with the current user's OS permissions and can access reachable files or network resources. Project Health therefore never auto-runs during discovery, never auto-installs dependencies, and must not be presented as safe for untrusted code.

## Problem semantics

A non-zero deterministic check produces a persisted `Reproduced` project problem and a stable run-local `health:<run-id>:<check-id>` evidence ID. Static Project Intelligence signals remain separate and are not promoted to bugs.

Repeated failures of the same deterministic command update one problem record and increase its occurrence count; the latest failure summary/evidence replaces the current observation. If that command later passes, the historical problem becomes inactive / not currently reproducing. A pass does **not** promote the problem to `Verified`; only the explicit Before/After proof engine may claim a verified fix.

Blocked, timed-out, or runner-error checks are `Incomplete`. They do not clear a previously reproduced failure because they provide no evidence that the behavior disappeared.

## Storage

A Project Health report and its problem updates are committed in one SQLite transaction. A crash or storage error cannot leave a health-run row committed without the corresponding problem state updates.

## AI boundary

The optional model receives, at most:

- bounded Project Intelligence facts;
- bounded `ctx:*` source snippets from Context Compiler;
- the latest non-passing Project Health results and their `health:*` IDs.

Before a request is sent, ReproDeck applies secret redaction and scrubs project/home/temp/application-data paths. A failing check is evidence of a failure, not evidence of root cause. Model output cannot set `Verified` state.

## Next hardening steps

Future Bug Hunter work should add language-aware dependency preparation with separate consent, richer evidence artifacts, problem-specific reruns, optional stronger OS sandbox adapters where practical, and a root-cause experiment graph. None of those should weaken the explicit execution/verification boundaries above.
