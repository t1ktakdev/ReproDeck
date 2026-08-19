Implementation tasks (rough order)

1. Create OpenSpec artifacts (this change) — proposal/design/tasks/acceptance
2. Add DB migrations: versioned SQL files that create tables with foreign keys and indexes; enable foreign_keys pragma in connection.
3. Implement reprodeck-core::timeline module: typed structs, insertion/query APIs, lifecycle functions.
4. Implement reprodeck-core::evidence module: artifact store abstraction, upload helpers, checksum, containment checks.
5. Add redaction/sanitization utilities and enforce at API boundary.
6. Implement TDD tests for migrations, lifecycle, restart recovery, truncation, artifact containment, pagination.
7. Run locked quality gate; address BLOCKER/HIGH if any.
8. Prepare checkpoint deliverables and update docs/agent-state.md with baseline SHA (after baseline commit already created).

Notes:
- Avoid unwrap/expect in new code; return typed Results.
- Keep migration backward-compatible for reasonable schema evolution.
