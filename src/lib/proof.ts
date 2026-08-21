import type { ReproductionRun, ReproductionStep } from "../types";

export function activeCycleRuns(step: ReproductionStep, runs: ReproductionRun[]): ReproductionRun[] {
  return runs.filter(run => run.step_id === step.id && run.cycle === step.active_cycle);
}

export function latestRun(runs: ReproductionRun[], phase: ReproductionRun["phase"]): ReproductionRun | null {
  return runs.find(run => run.phase === phase) ?? null;
}

export function canRunAfter(before: ReproductionRun | null): boolean {
  return before?.status === "Failed";
}
