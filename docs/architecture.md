# Architecture

ReproDeck is split into a local Rust engineering core, a thin Tauri adapter and a React desktop UI. The architecture is evidence-first: React and an AI model may request operations and present results, but only the Rust core can establish proof state or mutate an approved target.

## Product layers

```text
React desktop UI
      │
Thin Tauri commands
      │
┌──────────────────────────────────────────────┐
│ Project Intelligence                        │
│ Context Compiler                            │
│ Problem / Root-cause domain                 │
│ Optional AI provider                        │
│                                              │
│ Proof Engine                                │
│  ├─ safe runner / permissions               │
│  ├─ timeline / evidence / artifacts         │
│  ├─ Git shadow workspace                    │
│  ├─ Before / After verification             │
│  └─ conflict-safe Apply / recovery           │
│                                              │
│ SQLite / capsule / redaction / integrations │
└──────────────────────────────────────────────┘
```

## Project Intelligence

`project_intelligence` is a deterministic read-only discovery layer. It builds a persisted `ProjectProfile` for Git and non-Git folders. Scans are bounded, generated/vendor directories are skipped, symlinks are not followed, secret-like paths are excluded and Git ignore decisions are respected when available.

Discovery returns candidate commands but never runs them. This is an important trust boundary: opening an unknown repository cannot execute its `package.json`, build script or arbitrary executable.

## Context Compiler

`context_compiler` ranks repository text against a concrete investigation question and returns a bounded, inspectable `ContextPacket`. Snippets carry stable `ctx:*` IDs, source paths, line ranges, reasons, checksums and redacted content. The packet intentionally limits both file count and character count so local/small models receive focused evidence rather than a repository dump.

## Problem domain

The problem lifecycle is explicit:

`Signal → Suspected → Reproduced → RootCaused → FixProposed → Verified → Applied`

Evidence is required for states that assert reproduction, root cause and verification. This prevents UI or AI text from silently becoming product truth.

## AI boundary

AI is an optional reasoning adapter. The provider receives sanitized project facts and selected context snippets only after an explicit network confirmation. The model can propose hypotheses and next experiments, but cannot change permissions, verification state, Git state or Apply state.

## Proof Engine

The proven debugging engine owns executable + argv process execution, permissions, Git worktree isolation, timeline receipts, typed evidence, protected verification cycles, Before/After verdicts, diff review, conflict checks, Apply, Discard and crash recovery.

The original repository is inspected but not used as the fix workspace. ReproDeck creates a controlled worktree from a recorded base commit. Apply rejects incompatible HEAD or local changes rather than silently overwriting them. ReproDeck never commits or pushes automatically.

## Storage

SQLite stores project profiles, sessions and structured domain metadata. Large logs/evidence payloads use a content-addressed filesystem store with checksum-bearing records. `.reprodeck` is a versioned portable evidence archive with strict allow-list, path, hash and size validation.

## Adapter boundaries

`src-tauri` maps desktop calls to core functions and stable user-facing failures. It must stay thin. `src` renders returned state and local interaction state; it must not manufacture terminal output, diffs, evidence, root cause or verification results.

## Network boundary

The default product is local. GitHub and AI are opt-in integrations. Network/publishing actions require explicit confirmation. No telemetry or hidden repository upload is part of the architecture.
