ReproDeck Architecture (core storage and recovery)

Overview
- ReproDeck core stores two distinct categories of persistent data:
  1. Recovery DB (global): manages crash-recovery state for shadow workspaces and pending cleanup. Location: application-managed data directory (platform default, e.g. %LOCALAPPDATA%/reprodeck or $XDG_DATA_HOME/reprodeck). File: recovery.db
  2. Project DB (per-session/project): stored inside project-managed location (preferably .reprodeck within a separate managed directory or next to project as configured). Contains sessions, timeline_events, command_executions, evidence metadata and links to artifact files.

Why two databases
- Recovery DB must be accessible before opening any project DB: after a crash, we need to know which repos have pending cleanup without trusting project workspace. It stores minimal global state to recover shadow worktrees.
- Project DB contains richer domain data tied to a session and is owned by that project/session lifecycle.

Physical locations
- Recovery DB: $APPDATA/reprodeck/recovery.db (or OS equivalent). Owned by ReproDeck process and survives across sessions.
- Project DB: placed under application-managed directory per session (e.g. $APPDATA/reprodeck/artifacts/<repo-hash>/project.db) or optionally next to project when configured. Avoid writing arbitrary files into the user's repo root.

Lifecycle ownership
- Recovery DB: ReproDeck process owns creation, cleanup, and migration lifecycle.
- Project DB: created when a session starts and migrated by ReproDeck. Removal of a project/session should not automatically delete recovery entries for that repo, but recovery entries referencing missing repo paths will be ignored or cleaned by operator action.

Crash behavior
- Recovery DB: on crash, ReproDeck will read recovery.db to find AppliedCleanupPending entries and attempt retry_cleanup on startup or via user action.
- Project DB: if process crashes during a migration, the transaction-based migration ensures either previous schema/version remains intact or migration completes fully. Project DB corruption is surfaced as typed errors and requires operator action.

Migration semantics
- Recovery DB migrations: applied automatically on startup; small and atomic. The migration runner opens a Rust transaction per migration, applies SQL, updates schema_version within same transaction, commits.
- Project DB migrations: same runner and semantics; migrations are Rust-owned transactions. SQL migration scripts must not include their own BEGIN/COMMIT — runner enforces transaction boundaries.

Deletion and cleanup
- Deleting a project/session: should remove project DB and artifacts; recovery entries referencing the repo should be either removed or marked; ReproDeck will not implicitly remove user repository content.

Notes
- Do not create files inside user's repository working tree for ReproDeck metadata. Use app-managed directories for recovery and artifacts.

Upstream references and rationale
- Git delta/patch formats and machine-safe output: use `git diff -z --name-status` for NUL-delimited paths to handle arbitrary filenames (see: https://git-scm.com/docs/git-diff).
- Rust process handling: prefer direct spawning via std::process::Command, stream reads in separate threads to avoid deadlocks; check docs: https://doc.rust-lang.org/std/process/index.html.
 - Windows process-tree handling: full reliable termination of grandchildren requires Job Objects; creating a Job Object and assigning processes is the robust approach on Windows (see Microsoft Job Objects docs: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects). Current implementation uses Job Objects (KILL_ON_JOB_CLOSE) and is covered by unit tests validating process-tree termination semantics on Windows.
