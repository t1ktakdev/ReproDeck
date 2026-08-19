Design: Timeline + Evidence Foundation

Overview
- Single persistent SQLite-backed timeline for sessions, actions, executions, receipts, and artifacts.
- Implement as reprodeck-core::timeline and reprodeck-core::evidence modules (no new crate).
- Strong redaction/sanitization boundary for any runtime metadata crossing persistence.

Schema sketch (finalized after repro-architect review):
- sessions(id TEXT PRIMARY KEY, kind TEXT, meta JSON, created_at INTEGER)
- actions(id TEXT PRIMARY KEY, session_id TEXT, parent_id TEXT NULL, kind TEXT, meta JSON, state TEXT, created_at INTEGER)
- executions(id TEXT PRIMARY KEY, action_id TEXT, status TEXT, started_at INTEGER, finished_at INTEGER NULL, duration_ms INTEGER NULL)
- receipts(id TEXT PRIMARY KEY, execution_id TEXT, summary TEXT, stdout_preview TEXT, stderr_preview TEXT, created_at INTEGER)
- artifacts(id TEXT PRIMARY KEY, receipt_id TEXT, store_key TEXT, checksum TEXT, size INTEGER, media_type TEXT, created_at INTEGER)

Artifact store
- Implement a controlled artifact store rooted under configured storage_dir; artifacts referenced by opaque store_key (no absolute paths persisted).
- Store content-addressed checksum (sha256) and size; block path traversal and symlink escape.

Redaction & metadata
- Do not persist raw env or secrets. Provide sanitization hooks that produce typed metadata before persistence.
- Extensible metadata fields must declare version, allowed keys, and size limits.

API surface (library-level):
- create_session(kind, meta) -> session_id
- append_action(session_id, parent_id?, kind, meta) -> action_id
- start_execution(action_id) -> execution_id
- finish_execution(execution_id, status, stdout_path?, stderr_path?, artifact_refs?) -> receipt_id
- attach_artifact(receipt_id, reader, media_type) -> artifact_id
- list_timeline(session_id, cursor?, limit?)
- get_action(action_id), get_execution(execution_id), get_artifact(artifact_id)

Lifecycle invariants
- Action: Created -> Running -> Succeeded | Failed | Cancelled | Interrupted
- Execution cannot be Succeeded without finished_at timestamp
- Receipts must reference an existing execution
- Artifacts must belong to a receipt

Crash/Restart
- Running executions become Interrupted on startup if not finished; provide explicit recovery path.

Testing
- TDD: migrations, create/read, lifecycle, restart recovery, truncation/preview behavior, redaction tests, artifact containment tests, pagination stability.
