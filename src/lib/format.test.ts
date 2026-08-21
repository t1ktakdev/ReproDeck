import { describe, expect, it } from "vitest";
import { commandText, repoName } from "./format";

describe("format helpers", () => {
  it("extracts repository name across path separators", () => {
    expect(repoName("C:\\work\\demo")).toBe("demo");
    expect(repoName("/work/demo/")).toBe("demo");
  });
  it("quotes display-only argv without changing execution semantics", () => {
    expect(commandText("npm", ["test", "hello world"])).toBe('npm test "hello world"');
  });
});
