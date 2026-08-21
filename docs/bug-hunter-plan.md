# Smart Bug Hunter plan

The Smart Bug Hunter layer sits between Project Intelligence and Project Health. It does not use an LLM to decide whether a project is healthy.

## Deterministic planning

`bug_hunter::build_plan` ranks only commands already discovered by Project Intelligence. The current strategy is `diagnostics-first-v1`:

1. compiler/check commands;
2. type checks;
3. lint/static analysis;
4. deterministic tests;
5. builds.

Declared commands win ties over conventional candidates. Identical executable/argument pairs are deduplicated, `dev`/unknown commands remain manual-only, and the plan is bounded to eight automatic checks.

The explicit order is preserved by Project Health. This matters because discovery order is not necessarily diagnostic order.

## Failure grouping

A health run may emit many secondary errors from one root cause. `bug_hunter::analyze_failures` therefore groups failed/timed-out/error checks by a normalized first useful diagnostic. Stable compiler/type codes such as `E0308` or `TS2322` are preferred when present.

Blocked checks are never grouped as project bugs. They remain execution blockers because missing dependencies, safety rules or unavailable tools are prerequisites rather than evidence of faulty source code.

## Investigation contract

Every failure group contains:

- the health evidence IDs that support it;
- related active Project Problems;
- a root-path-free investigation query;
- a deterministic experiment sequence: reproduce, compile focused context, trace the first causal diagnostic, test one falsifiable hypothesis, then run broader regression checks.

The plan does **not** declare root cause or verification. Root-cause promotion still requires evidence, and `Verified` remains owned by the Before/After proof workflow.

## Safety boundary

Planning itself never executes project code. Execution remains behind the existing Project Health confirmation and disposable Git worktree boundary. This is working-tree isolation, not an OS sandbox.
