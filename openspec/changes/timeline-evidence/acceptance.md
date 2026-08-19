Acceptance criteria — Timeline + Evidence Foundation

Functional
- Migrations apply on empty DB and on an existing DB; migration tests pass.
- API: create_session, append_action, start_execution, finish_execution, attach_artifact implemented and exercised by tests.
- Lifecycle invariants enforced and tested (no Succeeded without finished_at, receipts reference executions, artifacts belong to receipts).
- Crash/restart test: execution left Running becomes Interrupted and is visible on startup.
- Artifact store prevents absolute-path persistence and symlink escape; checksum and size stored.

Non-functional
- Tests: unit + integration cover the required behaviors; total new tests >= 10 focused assertions.
- No unwrap/expect in production paths added; errors are Result-returning.
- Migrations versioned and rollback/error behavior defined.

Quality gate
- The authoritative locked quality gate must PASS after implementation before acceptance.
- No confirmed BLOCKER/HIGH from code or security judges.

Deliverables
- OpenSpec change artifacts (this directory)
- SQL migration files
- reprodeck-core::timeline and reprodeck-core::evidence modules with tests
- Documentation: design file added to docs/superpowers/specs/ and changelog entry
