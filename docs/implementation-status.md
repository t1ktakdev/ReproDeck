# ReproDeck implementation status

Updated: 2026-08-21

ReproDeck is now organized as an evidence-first desktop workbench rather than a dashboard. The product shell, Project Intelligence, Bug Hunter, Investigation Cases and the protected Before/After engine are integrated in one local-first workflow.

## Current product architecture

- A 52 px global activity rail owns Home, Projects, Sessions, Capsules and Settings.
- A contextual project/session sidebar appears only when a workspace is open.
- The main surface owns the current engineering task and scrolls independently.
- Investigation opens as a resizable, independently scrolling inspector instead of extending Checks vertically.
- AI remains optional. It produces bounded, evidence-linked engineering hypotheses and never sets verification state.
- A supported causal experiment hands the exact checkpointed patch, criterion and provenance to a separate protected verification session.

## Safety and reliability boundary

- Project discovery is bounded, Git-ignore-aware and does not execute repository code.
- Health runs, experiments and fixes use disposable or recoverable Git worktrees.
- The original repository is protected by HEAD, porcelain-state and tracked-diff integrity checks.
- Evidence starts Neutral and must be explicitly classified as Supports or Contradicts.
- Apply remains backend-gated on a failing Before, passing After, exact binary patch SHA-256, unchanged source commit/state and every required regression receipt.
- Recovery cleanup validates the temporary-directory boundary and `reprodeck-shadow-*` branch/worktree names before deletion.
- Recovery unit tests use a process-local temporary database and cannot populate the user's recovery queue.
- OpenAI-compatible provider URLs reject credentials, query strings, fragments and unsupported schemes.
- No telemetry, automatic upload, commit, push or Investigation Apply path exists.

## Windows verification

The complete quality gate passed on the final 0.3.0 tree on this machine:

- Node.js 24.18.0 / npm 11.16.0
- Rust/Cargo 1.97.1
- Git 2.55.0
- `npm run typecheck`
- `npm test`
- `npm run build`
- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- `scripts\verify-windows.cmd -SkipInstall`

The final run contained 29 frontend tests, 108 Core tests and one shadow-workspace acceptance test. The NSIS bundle was built after this gate; the release task handoff records its exact hash and signing status.

## Field test

The deterministic demo fixture completed the real Tauri flow on Windows:

1. Project scan detected three checks without executing them.
2. `check` passed, `test` failed and `build` passed.
3. An explicit Investigation Case captured baseline and source evidence.
4. LM Studio at `http://127.0.0.1:1234/v1` returned three structured hypotheses. The first candidate was broader than the exact defect, so it remained an engineering candidate rather than proof.
5. A minimal tenant-scoped cache-key intervention ran only in a Fix Workspace.
6. The exact failing criterion passed in the causal experiment while the original repository remained unchanged.
7. The final 0.3 field pass created a distinct verification worktree, recorded a failing Before, transferred the exact checkpointed patch, recorded a passing After and passed two promoted Required regressions for the same SHA-256.
8. A post-After workspace edit immediately changed the status to Patch Changed and blocked Apply. Restoring the exact verified bytes restored Ready to Apply.
9. Apply was intentionally not executed in the live investigation demo, so the original fixture stayed clean at its original HEAD. Separate Core/acceptance tests exercise successful Apply.
10. A pre-final 0.3.0 bundle upgraded an installed 0.2.0 successfully. The final post-fix current-user bundle then completed a fresh install, launched a real ReproDeck window and uninstalled cleanly while preserving the database and artifacts.

## Known boundaries

- Regression recommendations are deterministic command/file heuristics, not an import-graph test selector.
- A causal experiment proves that the selected intervention changes the recorded criterion under integrity guards; it cannot semantically prove that broad model wording is the unique explanation.
- Windows 125% scaling was exercised natively. 150% system scaling and Linux/macOS native packaging were not exercised in this pass.
- Linux/macOS native packaging remains a field-test responsibility; Windows is the release platform verified in this repository.
