# Outcome Verification — OpenSpec Change Proposal

Summary
-------
Add an Outcome Verification subsystem to ReproDeck that allows creating versioned OutcomeContracts containing ordered VerificationChecks, running deterministic BEFORE/AFTER verification runs using the existing Timeline/Runner/Evidence systems, and producing an OutcomeResult that deterministically evaluates whether a fix is verified, inconclusive, or not fixed.

Goals
-----
- Reproduce a failure (BEFORE) and re-run verification after a fix (AFTER).
- Reuse Timeline, Evidence, Runner, Permissions, and Storage (SQLite).
- Provide typed domain primitives, deterministic evaluation rules, and persisted runs linking to Timeline/Evidence artifacts.
- Surface a minimal Rust API for future Tauri UI.

Non-Goals
---------
- UI work or Tauri integration in this change.
- New execution runner or separate evidence store.

Risks
-----
- Migration to EvidenceRole if extension required; will be designed backwards-compatible.
- Permission/Ask semantics must be preserved — tests will cover deny/ask behavior.
