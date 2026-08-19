# 2026-08-19 — Outcome Verification Design

This design documents the Outcome Verification feature for ReproDeck. It mirrors openspec/changes/outcome-verification and expands implementation notes.

See openspec/changes/outcome-verification/* for proposal, design, tasks, and acceptance.

Implementation notes
--------------------
- Use existing SQLite migrations and the project's migration tooling to add tables. Keep new tables optional and backward-compatible where possible.
- Use stable IDs for VerificationCheck to allow external references and idempotency.
- Do not introduce new execution pathways — use Runner and Timeline Action/Execution/Receipt semantics.
- Evidence artifacts must be linked using existing Evidence APIs; add EvidenceRole values only if strictly necessary.

APIs summary
------------
Rust-facing APIs (to be implemented in src/outcome_verification.rs):

- create_outcome_contract(session_id, title, description, checks) -> OutcomeContract
- get_outcome_contract(contract_id) -> OutcomeContract
- list_outcome_contracts(session_id, paging...) -> Vec<OutcomeContract>
- add_verification_check(contract_id, check) -> VerificationCheck
- update_verification_check(check_id, patch) -> VerificationCheck
- start_verification_run(contract_id, phase: Before|After) -> VerificationRun (starts Timeline Action and returns run id)
- finish_verification_run(run_id, receipt_id, status, evidence_refs) -> VerificationRun
- interrupt_running_verifications() -> count
- evaluate_outcome(contract_id) -> OutcomeResult

Testing
-------
Implement unit and integration tests that execute real commands through Runner in a sandbox project fixture. Assert Timeline receipts, sanitized outputs, evidence links, and persisted run states.
