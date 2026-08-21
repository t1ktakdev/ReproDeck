# ReproDeck Bench

ReproDeck Bench measures workflow outcomes, not model style. It is designed to compare the same model in two conditions:

1. a baseline agent/model workflow;
2. the same model with ReproDeck evidence, causal experiment and exact-patch verification.

No comparative results are published in this repository yet. A valid comparison must use the same fixture revision, model build, sampling settings, time budget and machine class.

## Metrics

- correct root cause identified;
- patch fixes the original reproduction;
- every required regression passes;
- false fix proposed or applied;
- original repository mutated before explicit Apply;
- claims unsupported by captured evidence;
- elapsed time;
- input/output tokens and provider cost when the provider reports them.

## Run the deterministic baseline audit

From PowerShell on Windows:

```powershell
.\scripts\run-reprodeck-bench.ps1
```

This creates a temporary copy of the auth/cache fixture, runs its three declared commands, checks that the original Git state is unchanged, and writes a machine-readable local result under `bench-results/`. It does not call a model and therefore does not claim model quality.

## Add a case

Add a manifest under `bench/cases/` with a stable ID, fixture generator, expected baseline results, success criterion and required regressions. The bug must be deterministic and the manifest must not reveal its root cause to the participant. Keep the answer key outside the prompt presented to either condition.

For an agent comparison, save both transcripts and receipts, then have an evaluator score the manifest metrics without knowing which condition produced them.
