# Changelog

## 0.3.0 — evidence-first debugging workbench

### Added

- Persistent Investigation Cases with explicit evidence relationships, bounded context compilation, structured hypotheses, and causal experiment receipts.
- Protected Investigation-to-Verification handoff with exact patch transfer, deterministic regression recommendations, and a visible proof chain.
- Deterministic Try Demo fixture, ReproDeck Bench baseline runner, recovery records, and native product screenshots.
- Desktop preferences for language, density, typography, motion, layout, behavior, optional AI, privacy, and scoped resets.

### Changed

- Reworked the interface into a keyboard-first desktop Workbench with an activity rail, contextual navigation, focused Checks surface, and resizable Investigation inspector.
- Bound After proof and Ready to Apply to source commit/working state, success criterion, shadow commit, binary patch SHA-256, and all required regression receipts.
- Made AI explicitly optional and evidence-bound, with inspectable context and an OpenAI-compatible provider path including LM Studio.

### Fixed

- Made repeated verification-session IDs collision-safe.
- Removed the Cargo output-name collision between the desktop application and CLI.
- Replaced unsupported WebView confirmations with localized Tauri dialogs and isolated recovery tests from user application data.

### Security

- Added patch-drift invalidation, verified-session-only Apply, durable path-validated recovery, stronger provider URL validation, and expanded secret/path/capsule regression coverage.
- Preserved the rule that Git worktrees isolate repository writes but do not sandbox project code at the operating-system level.

## Alpha 3 — Smart Bug Hunter

- Added diagnostics-first planning, failure clustering, blocker separation and evidence-linked investigation seeds.

## 0.2.0 — protected verification and capsules

- Added protected Before/After cycles, typed evidence, reviewed diff evidence and backend-gated Apply.
- Added secure `.reprodeck` v1 export/import, EN/RU UI, themes, Project Intelligence and Windows release automation.
- Hardened recovery, migrations, command execution, capsule integrity and import TOCTOU behavior.

## 0.1.0

- First functional Windows field-test slice with Git shadow workspace, Before/After verification and explicit Apply.
