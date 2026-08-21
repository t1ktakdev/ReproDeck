# `.reprodeck` capsule format — version 1

A `.reprodeck` file is a ZIP container intended to preserve a sanitized debugging session without embedding the complete source repository.

Required top-level control files:

```text
manifest.json
checksums.json
```

Typical payload:

```text
session.json
environment.json
reproduction.json
timeline.json
evidence/index.json
evidence/artifacts/<artifact-id>
diffs/shadow.patch        # active workspace, when safe
diffs/verified.patch      # persisted reviewed patch after Apply, when available
```

`manifest.json` contains `format: "reprodeck"`, `version: 1`, creation time, session identity/title, a file table with path/SHA-256/size/media type, and a redaction/omission summary. `checksums.json` must exactly match the file hashes declared by the manifest.

## Export review

Before writing a capsule, the desktop UI asks the core for the exact file table and redaction/omission summary that would be exported. The user can cancel before choosing a destination. The real export rebuilds the payload from durable session data and applies the same rules.

## Import rules

Version 1 currently enforces:

- `.reprodeck` extension;
- at most 2048 ZIP entries;
- 32 MiB maximum uncompressed payload per entry;
- 256 MiB maximum aggregate uncompressed payload;
- no absolute paths, `..`, backslashes, drive-qualified names, nulls, directories or symlink entries;
- no duplicate, undeclared or missing entries;
- exact `checksums.json` ↔ manifest equality;
- exact payload byte size and SHA-256 verification;
- validation and persistence from the same open source file handle.

Unknown format names or versions are rejected instead of guessed.

## Privacy

The exporter does not include the entire repository. Structured session text and text evidence are redacted. Diffs referencing denied secret-like paths are omitted and recorded in the manifest redaction summary.
