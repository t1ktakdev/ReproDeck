import { describe, expect, it } from "vitest";
import { activeCycleRuns, canRunAfter, latestRun } from "./proof";
import type { ReproductionRun, ReproductionStep } from "../types";

const step: ReproductionStep = { id: "step", session_id: "s", ordering: 0, executable: "test", args: [], expected_exit_code: 0, active_cycle: 2, created_at: 1 };
const run = (phase: "Before" | "After", cycle: number, status: string): ReproductionRun => ({ id: `${phase}-${cycle}`, step_id: "step", phase, action_id: "a", receipt_id: null, exit_code: status === "Passed" ? 0 : 1, status, cycle, created_at: cycle });

describe("verification UI state", () => {
  it("ignores historical cycles when selecting active proof", () => {
    const current = activeCycleRuns(step, [run("Before", 1, "Failed"), run("Before", 2, "Failed"), run("After", 2, "Passed")]);
    expect(current).toHaveLength(2);
    expect(latestRun(current, "After")?.status).toBe("Passed");
  });

  it("enables After only after a failing Before", () => {
    expect(canRunAfter(null)).toBe(false);
    expect(canRunAfter(run("Before", 2, "Passed"))).toBe(false);
    expect(canRunAfter(run("Before", 2, "Failed"))).toBe(true);
  });
});
