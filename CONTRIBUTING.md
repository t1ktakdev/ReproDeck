# Contributing to ReproDeck

Thank you for helping improve ReproDeck. Keep changes focused, explain the debugging problem they solve, and preserve the local-first safety model.

## Prerequisites

- Git
- Node.js 22 or newer and npm
- Stable Rust with `rustfmt` and `clippy`
- Tauri 2 system prerequisites for your platform
- On Windows, WebView2 and the Microsoft C++ build tools

## Setup

```powershell
git clone https://github.com/t1ktakdev/ReproDeck.git
Set-Location ReproDeck
npm ci
cargo check --workspace --all-targets
npm run tauri dev
```

The frontend is under `src/`, the thin desktop adapter is under `src-tauri/`, and domain/security behavior lives in `crates/reprodeck-core/`. Keep product truth and mutation gates in Core rather than React or Tauri glue.

## Verification

Before submitting a pull request, run:

```powershell
npm run typecheck
npm test
npm run build
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

On Windows, the complete release gate is:

```powershell
.\scripts\verify-windows.cmd -SkipInstall
```

For UI changes, also run `npm run tauri dev` and inspect the affected workspaces at practical desktop sizes. Include clean screenshots in the pull request.

## Security invariants

- Never weaken original-repository protection.
- Never treat AI output as evidence or proof.
- Verified patch identity must remain backend-gated.
- Apply must remain explicit and bound to the exact verified patch and required receipts.
- Opening a project must not execute project code.
- Do not build shell strings from user-controlled commands; keep executable and argv separate.
- Do not add automatic network, commit, push, cleanup, or Apply behavior.
- Preserve secret exclusion/redaction, capsule validation, path containment, and symlink/reparse-point checks.

For changes to workspaces, Apply, runner, redaction, recovery, evidence, or capsules, add a regression test that proves the relevant invariant. Never add real credentials, proprietary source, or unsanitized logs to fixtures.

## Product and UI contributions

ReproDeck should feel like a focused desktop developer tool: clear hierarchy, useful density, keyboard and focus behavior, restrained status color, and no AI-first marketing chrome. Add complete English and Russian strings for user-facing text. Production paths must show real command output, evidence, receipts, and diffs—not mock data.

## Pull requests

Describe what changed, why it is needed, how it was tested, and any effect on repository mutation or data handling. Keep unrelated cleanup out of the same pull request. By contributing, you agree that your contribution is licensed under the repository's MIT License.
