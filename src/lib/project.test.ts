import { describe, expect, it } from "vitest";
import { bugHunterRunOrder, formatContextPacket, formatProjectCommand, healthCounts, projectStackSummary, regressionRecommendations, runnableProjectCommands } from "./project";
import type { BugHunterPlan, ContextPacket, ProjectHealthReport, ProjectProfile } from "../types";

const baseProfile: ProjectProfile = {
  schema_version: 1,
  fingerprint: "project:test",
  root_path: "C:/repo",
  name: "sample",
  version: null,
  description: null,
  analyzed_at: 0,
  git: null,
  languages: [],
  technologies: [
    { name: "React", category: "framework", evidence: ["package.json"] },
    { name: "Tauri", category: "desktop", evidence: ["Cargo.toml"] },
    { name: "Rust", category: "language", evidence: ["Cargo.toml"] },
  ],
  commands: [],
  entrypoints: [],
  test_paths: [],
  documentation: [],
  ci_files: [],
  signals: [],
  stats: {
    files_seen: 0,
    source_files: 0,
    test_files: 0,
    documentation_files: 0,
    sensitive_files_excluded: 0,
    skipped_large_files: 0,
    todo_markers: 0,
    scan_truncated: false,
  },
};

describe("project presentation helpers", () => {
  it("quotes command arguments that contain whitespace", () => {
    expect(formatProjectCommand({
      id: "cmd:1",
      label: "test",
      kind: "Test",
      executable: "node",
      args: ["script.js", "hello world"],
      source: "fixture",
      confidence: "Declared",
    })).toBe('node script.js "hello world"');
  });

  it("builds a bounded stack summary", () => {
    expect(projectStackSummary(baseProfile, 2)).toBe("React · Tauri");
  });

  it("formats inspectable context without adding the local root path", () => {
    const packet: ContextPacket = {
      root_path: "C:/Users/private/repo",
      query: "why does refresh fail?",
      snippets: [{
        id: "ctx:123:src:auth.ts",
        path: "src/auth.ts",
        language: "TypeScript",
        score: 91,
        reasons: ["query match"],
        line_start: 10,
        line_end: 12,
        content: "return refresh();",
        checksum: "abc",
        truncated: false,
      }],
      stats: {
        files_considered: 42,
        files_ranked: 4,
        sensitive_files_excluded: 2,
        skipped_large_or_binary: 1,
        selected_chars: 17,
        candidate_scan_truncated: false,
        packet_truncated: false,
      },
    };
    const text = formatContextPacket(packet);
    expect(text).toContain("ctx:123:src:auth.ts");
    expect(text).toContain("Sensitive paths excluded: 2");
    expect(text).not.toContain(packet.root_path);
  });
  it("selects only deterministic Project Health commands", () => {
    const commands = [
      { id: "test", label: "test", kind: "Test" as const, executable: "npm", args: ["test"], source: "package.json", confidence: "Declared" as const },
      { id: "dev", label: "dev", kind: "Dev" as const, executable: "npm", args: ["run", "dev"], source: "package.json", confidence: "Declared" as const },
      { id: "lint", label: "lint", kind: "Lint" as const, executable: "npm", args: ["run", "lint"], source: "package.json", confidence: "Declared" as const },
    ];
    expect(runnableProjectCommands(commands).map(command => command.id)).toEqual(["test", "lint"]);
  });

  it("preserves the deterministic Bug Hunter plan order for selected checks", () => {
    const plan = {
      steps: [
        { command_id: "check" },
        { command_id: "test" },
        { command_id: "build" },
      ],
    } as BugHunterPlan;
    expect(bugHunterRunOrder(["build", "check"], plan)).toEqual(["check", "build"]);
  });

  it("counts passed, failed and incomplete health checks separately", () => {
    const report = {
      checks: [
        { status: "Passed" },
        { status: "Failed" },
        { status: "Blocked" },
        { status: "TimedOut" },
      ],
    } as ProjectHealthReport;
    expect(healthCounts(report)).toEqual({ passed: 1, failed: 1, incomplete: 2 });
  });

  it("ranks exact reproduction before deterministic regression checks", () => {
    const profile = { ...baseProfile, commands: [
      { id: "test:target", label: "target test", kind: "Test" as const, executable: "npm", args: ["test", "--", "target"], source: "package.json", confidence: "Declared" as const },
      { id: "typecheck", label: "typecheck", kind: "Typecheck" as const, executable: "npm", args: ["run", "typecheck"], source: "package.json", confidence: "Declared" as const },
      { id: "build", label: "build", kind: "Build" as const, executable: "npm", args: ["run", "build"], source: "package.json", confidence: "Declared" as const },
      { id: "dev", label: "dev", kind: "Dev" as const, executable: "npm", args: ["run", "dev"], source: "package.json", confidence: "Declared" as const },
    ] };
    const ranked = regressionRecommendations(profile, ["src/cache.ts"], "test:target");
    expect(ranked.map(item => [item.command.id, item.tier])).toEqual([
      ["test:target", "required"],
      ["typecheck", "required"],
      ["build", "optional"],
    ]);
  });

});
