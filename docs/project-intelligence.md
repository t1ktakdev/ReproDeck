# Project Intelligence and evidence-first agent design

This document defines the product boundary for the next ReproDeck architecture. The central rule is: **understanding a project is read-only; proving a problem or a fix requires recorded evidence.**

## 1. Project Passport

`project_intelligence` performs bounded local discovery. It may read ordinary repository files and Git metadata, but it must not execute repository code, install dependencies or start services.

The resulting `ProjectProfile` contains declared metadata, Git state, languages, technology signals, candidate commands, entrypoints, tests, documentation, CI files, deterministic review signals and scan statistics.

Candidate commands are data only (`executable + argv + source + confidence`). Their presence in the profile is not permission to execute them.

## 2. Bounded discovery

Discovery has hard scan/read limits, ignores common generated/vendor directories, does not follow symlinks and respects Git ignore decisions when a Git repository is available. Secret-like paths are excluded before file content is read.

A truncated scan is represented explicitly in profile statistics and becomes a review signal. ReproDeck must not pretend the resulting profile is exhaustive.

## 3. Signals are not bugs

Static discovery can produce `Info`, `Review` or `Warning` signals. These are leads, not confirmed defects. For example, TODO/FIXME markers are deliberately described as maintenance leads.

The problem domain separates confidence states:

`Signal → Suspected → Reproduced → RootCaused → FixProposed → Verified → Applied`

Evidence is mandatory for transitions that assert reproduction, root cause or verification.

## 4. Context Compiler

A language model should not receive the repository wholesale. `context_compiler` turns a question into a bounded ranked packet:

- path and content matches are ranked deterministically;
- project anchors and tests can receive small relevance boosts;
- only bounded line windows are selected;
- each snippet has an immutable-looking `ctx:*` identifier, path, line range and checksum;
- the returned packet includes selection statistics and truncation flags;
- secret-like, ignored, symlink, binary and oversized candidates are skipped;
- text is redacted before it becomes model context.

The context packet shown in the UI is inspectable and can be copied. The filesystem root path is not included in the text sent to the model.

## 5. AI boundary

The current provider is OpenAI-compatible and optional. A model receives sanitized project facts, the user question and selected `ctx:*` snippets. The investigation prompt requires:

- observations separated from hypotheses;
- exact context IDs for supporting evidence;
- ranked hypotheses rather than one fabricated certainty;
- a statement of what would disprove a hypothesis;
- a deterministic next check or experiment;
- no claim that a fix is verified.

AI output is never accepted as a verification verdict. Local and cloud endpoints use the same boundary; network access requires an explicit confirmation flag.

## 6. From investigation to proof

The intended next integration is:

1. a user or deterministic signal creates a problem candidate;
2. Context Compiler helps investigate it;
3. a deterministic check is selected or authored;
4. the existing Proof Engine creates an isolated worktree and runs the check;
5. a failing Before result moves the problem to `Reproduced` with evidence;
6. a proposed patch is made only inside the shadow workspace;
7. After and regression checks produce verification evidence;
8. `Verified` is a core verdict, never an AI declaration;
9. Apply remains explicit and conflict-safe.

## 7. Small-model strategy

ReproDeck does not claim that a small local model becomes equivalent to a frontier model. The engineering goal is to reduce how much raw intelligence the model must spend on navigation and bookkeeping. Project discovery, context selection, secret filtering, command execution, Git isolation, receipts, evidence IDs and verification are deterministic responsibilities of ReproDeck.

This lets a smaller model focus on a narrow causal question over a compact evidence packet.

## 8. Future expansion

The next production layers should add framework-specific check planners, runtime/browser observation adapters, repository symbol/import indexing, evidence-backed root-cause graph persistence, agent tool orchestration through the existing permission engine, and regression policy profiles. Each new layer must preserve the read-only discovery and independent-verification boundaries above.
