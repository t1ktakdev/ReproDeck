Name: timeline-evidence
Title: Timeline + Evidence Foundation

What: Implement a durable Timeline and Evidence persistence layer for ReproDeck. Capture actions, executions, receipts, and artifacts with safe sanitization, redaction, and controlled artifact storage.

Why: Product goal — Capture. Reproduce. Fix. Prove. Provide backend primitives needed by the future Tauri UI and Outcome Verification.

Scope: database schema + migrations, typed Rust APIs (reprodeck-core::timeline, reprodeck-core::evidence), artifact store abstraction, tests (migration, crash/restart, lifecycle, redaction). No polished UI in this change.
