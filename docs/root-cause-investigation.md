# Root-cause Investigation Cases

Bug Hunter identifies reproducible failure groups. ReproDeck turns one group into a durable engineering investigation without granting that surface authority to modify the original repository.

## Evidence flow

The intended flow is:

`Observed failure -> focused evidence -> falsifiable hypothesis -> isolated intervention -> causal experiment -> protected verification handoff -> Apply`

An Investigation Case owns only the first five stages. **It has no Apply API.** A successful causal experiment may support a hypothesis, but the existing session verification workflow still owns `Verified Fix` and Apply.

## Persistence

Migration 9 adds `investigation_cases` and `investigation_workspaces`. Migration 10 adds the separate verification handoff, exact proof and regression-contract records.

A case is bound to:

- the exact Project Health run;
- the exact failure cluster;
- the recorded base commit;
- the deterministic command that failed;
- the evidence IDs observed in that run.

The case also stores the exact baseline summary, bounded stdout/stderr excerpt, exit code, duration and completion time. It survives later health reruns. A passing rerun may clear an active Project Problem, but it does not erase the historical investigation.

## Evidence-bound hypotheses

A case accepts at most three hypotheses. Each hypothesis must include a statement, a falsifier and the smallest next experiment.

Every reference is classified as `Supports`, `Neutral` or `Contradicts`. New evidence starts as `Neutral`; the UI never converts proximity into causal support. Evidence references are checked against IDs already recorded by the case, duplicate/conflicting classifications are rejected, and unknown IDs are recorded as rejected citations. If a model cites unknown evidence, accepted confidence is capped at 25%. A hypothesis with no supporting evidence is capped at 40%.

Focused source evidence stores its source path, language, bounded excerpt, relevance score, checksum and capture time. Secret-like files remain excluded by Context Compiler policy.

Confidence is advisory. It never changes repository state or verification state.

## Fix Workspace

The Fix Workspace is a persistent Git worktree and shadow branch created from the case's recorded base commit. It can be opened in an editor and recovered if the temporary worktree directory disappears while the branch remains.

The investigation surface supports:

- create/recover workspace;
- checkpoint edits on the shadow branch;
- review the checkpointed binary-safe Git patch;
- run a causal experiment;
- discard the workspace.

It deliberately does **not** expose Apply.

## Causal experiment

An experiment requires:

1. the original baseline result was a deterministic `Failed` check;
2. an explicit hypothesis exists;
3. the Fix Workspace has a checkpointed non-empty intervention;
4. explicit execution confirmation.

ReproDeck reruns the exact recorded command in the Fix Workspace. It snapshots both the original repository and the Fix Workspace before and after execution using HEAD, porcelain status and a SHA-256 of the tracked diff.

`SupportsHypothesis` requires all of the following:

- baseline = Failed;
- experiment = Passed with the same expected exit code;
- original repository unchanged;
- the command itself did not mutate tracked/worktree Git state during the run.

A pass is still labelled **Supports hypothesis**, not `Root cause proven` and not `Verified Fix`. Failed or integrity-violating experiments record `Contradicted` or `Inconclusive` rather than silently returning to `Candidate`.

## Verification handoff

After a supported experiment, the inspector offers a protected verification handoff. It pre-fills the Before/After session wizard with the exact recorded executable, arguments, expected exit code and evidence context. Core also captures the exact checkpointed binary patch, validates its source commit and experiment receipt, hashes it and preflights it against a distinct clean verification worktree.

The user still reviews and creates the session explicitly. The patch is staged but not activated until the active Before run proves the baseline failure. Core then applies and checkpoints those exact bytes in the verification worktree. A passing After records a proof bound to the source commit and working state, reproduction command and criterion, shadow commit, file set and patch SHA-256. Any subsequent source, command, criterion or patch change blocks Apply.

ReproDeck also suggests a bounded deterministic regression set, ranked as Required, Recommended and Optional. A user may promote a check but cannot silently demote one. Every Required check must have a passing receipt for the verified patch SHA-256 before the backend reports Ready to Apply.

## Deterministic fixture

Use **Try Demo** on an empty Home screen, or run `scripts\create-demo-fixture.ps1`. The current auth/cache fixture exercises multiple symptoms, an evidence-bound hypothesis, a tenant-scoped cache-key intervention, exact patch transfer and Required regressions. See [demo.md](demo.md) for the complete flow and integrity assertions.
