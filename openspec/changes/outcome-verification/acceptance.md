# Outcome Verification — Acceptance Criteria

Minimum acceptance criteria for this OpenSpec change:

- Domain types and enums implemented and persisted with migrations.
- Verification runs create Timeline Actions/Executions/Receipts and link Evidence artifacts with appropriate roles.
- BEFORE and AFTER runs produce persisted VerificationRun rows with statuses and evidence links.
- Deterministic evaluation: BEFORE failed + AFTER passed => Verified Fix; BEFORE passed => Reproduction Not Proven; BEFORE failed + AFTER failed => Not Fixed; errors/interruptions => Inconclusive.
- Permissions: Deny prevents execution; Ask returns decision-required and does not execute.
- Recovery: Running -> Interrupted on restart.
- Tests: TDD suite covers PHASE I test matrix.
- Locked quality gate passes and GPT-OSS CODE/SECURITY judges report no BLOCKER/HIGH.
