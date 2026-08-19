# Outcome Verification — Design

Overview
--------
Outcome Verification introduces typed Rust domain types and persisted runs that use the existing Timeline/Runner/Evidence systems to perform BEFORE and AFTER verification. Each OutcomeContract contains ordered VerificationChecks. A VerificationRun executes checks by creating Timeline Actions and Receipts, and links produced Evidence artifacts with specified roles.

Core domain
-----------
- OutcomeContract { id, session_id, title, description, checks: Vec<VerificationCheck>, state, created_at, updated_at, version }
- VerificationCheck { stable_id, description, command_ref, expected_condition, required: bool, order: u32 }
- VerificationRun { id, contract_id, check_id, phase: Before|After, status: enum, started_at, finished_at, duration_ms, receipt_id, evidence_refs }
- OutcomeResult { contract_id, overall_state, before_summary, after_summary, verdict, evidence_links }

Enums and states
----------------
Use typed enums for run status (Pending, Running, Passed, Failed, Error, Interrupted) and phases. Version numerics for contract schema.

Evidence integration
--------------------
Link runs to Timeline Evidence artifacts using existing Evidence API. Use EvidenceRole values: Before, After, Verification, Diagnostic. If EvidenceRole needs extension, create a backward-compatible migration to add values.

Execution
---------
Runs must use the existing Runner via Action -> Execution -> Receipt flow. Permission Ask/Deny preserved: Ask returns a decision-required state and does not execute. Receipt metadata and sanitized stdout/stderr are linked as Evidence artifacts.

Recovery
--------
Persist run state frequently. On startup, any run with status Running is transitioned to Interrupted. Do not auto-retry destructive commands.

APIs
----
Provide Rust functions: create_outcome_contract, get_outcome_contract, list_outcome_contracts, add_verification_check, update_verification_check, start_verification_run, finish_verification_run, interrupt_running_verifications, get_verification_run, list_verification_runs, evaluate_outcome, get_outcome_summary.

Testing
-------
Follow PHASE I test matrix. Use test fixtures that run commands through Runner to assert Evidence links and sanitized outputs.
