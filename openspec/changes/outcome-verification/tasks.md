# Outcome Verification — Implementation Tasks

1. Create domain types in Rust (outcome_verification module) with enums and versioning.
2. Add persistent DB schema/migration using existing SQLite migrations; include outcome_contracts, verification_checks, verification_runs, outcome_results tables with foreign keys to timeline/session.
3. Implement APIs described in design and wire them into existing service layer.
4. Integrate with Runner: start_verification_run should create Timeline Action/Execution and await Receipt; link Receipt -> Evidence artifacts.
5. Implement recovery: mark Running -> Interrupted on startup.
6. Hook permission checks (Allow/Ask/Deny) to the existing permission engine; Ask returns decision-required without executing.
7. Implement deterministic evaluation rules and OutcomeResult calculation.
8. Write TDD tests covering PHASE I test matrix.
9. Add docs: API comments and docs/superpowers/specs/2026-08-19-outcome-verification-design.md.
10. Run targeted tests and iterate until passing.
11. Run locked quality gate, prepare evidence packets, run GPT-OSS code/security judgments, fix confirmed BLOCKER/HIGH findings.
