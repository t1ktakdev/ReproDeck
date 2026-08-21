import type { BugHunterPlan, ContextPacket, ProjectCommand, ProjectHealthReport, ProjectProfile } from "../types";

export function formatProjectCommand(command: ProjectCommand): string {
  return [command.executable, ...command.args]
    .map(value => /\s/.test(value) ? JSON.stringify(value) : value)
    .join(" ");
}

export function projectStackSummary(profile: ProjectProfile, limit = 5): string {
  return profile.technologies
    .slice(0, Math.max(0, limit))
    .map(item => item.name)
    .join(" · ");
}

export function formatContextPacket(packet: ContextPacket): string {
  return [
    `Question: ${packet.query}`,
    `Files considered: ${packet.stats.files_considered}`,
    `Sensitive paths excluded: ${packet.stats.sensitive_files_excluded}`,
    "",
    ...packet.snippets.flatMap(snippet => [
      `--- ${snippet.id} | ${snippet.path}:${snippet.line_start}-${snippet.line_end} | score ${snippet.score} ---`,
      snippet.content,
      "",
    ]),
  ].join("\n");
}

export function runnableProjectCommands(commands: ProjectCommand[], limit = 8): ProjectCommand[] {
  const runnableKinds = new Set(["Build", "Test", "Lint", "Typecheck", "Check"]);
  return commands.filter(command => runnableKinds.has(command.kind)).slice(0, Math.max(0, limit));
}

export function healthCounts(report: ProjectHealthReport | null): { passed: number; failed: number; incomplete: number } {
  if (!report) return { passed: 0, failed: 0, incomplete: 0 };
  return {
    passed: report.checks.filter(check => check.status === "Passed").length,
    failed: report.checks.filter(check => check.status === "Failed").length,
    incomplete: report.checks.filter(check => ["Blocked", "TimedOut", "Error"].includes(check.status)).length,
  };
}

export function bugHunterRunOrder(selected: string[], plan: BugHunterPlan | null): string[] {
  const chosen = new Set(selected);
  const planned = plan?.steps.map(step => step.command_id).filter(id => chosen.has(id)) ?? [];
  const plannedSet = new Set(planned);
  return [...planned, ...selected.filter(id => !plannedSet.has(id))];
}

export type RegressionTier = "required" | "recommended" | "optional";
export type RegressionRecommendation = { command: ProjectCommand; tier: RegressionTier; reasons: string[] };

export function regressionRecommendations(profile: ProjectProfile, changedFiles: string[], primaryCommandId: string): RegressionRecommendation[] {
  const hasTestChange = changedFiles.some(path => /(^|[/\\])(__tests__|tests?|spec)([/\\]|\.|$)|\.(test|spec)\.[^.]+$/i.test(path));
  const hasSourceChange = changedFiles.some(path => !/(^|[/\\])(docs?|examples?)([/\\]|$)|\.(md|txt)$/i.test(path));
  return runnableProjectCommands(profile.commands, 24)
    .map(command => {
      if (command.id === primaryCommandId) return { command, tier: "required" as const, reasons: ["exact-reproduction"] };
      if (command.kind === "Test") return { command, tier: "recommended" as const, reasons: [hasTestChange ? "changed-tests" : "behavior-regression"] };
      if (["Check", "Typecheck", "Lint"].includes(command.kind) && hasSourceChange) return { command, tier: "required" as const, reasons: ["changed-source"] };
      if (command.kind === "Build") return { command, tier: "optional" as const, reasons: ["release-shape"] };
      return null;
    })
    .filter((item): item is RegressionRecommendation => item !== null)
    .sort((left, right) => ["required", "recommended", "optional"].indexOf(left.tier) - ["required", "recommended", "optional"].indexOf(right.tier))
    .slice(0, 8);
}
