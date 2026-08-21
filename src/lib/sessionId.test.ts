import { describe, expect, it } from "vitest";
import { uniqueSessionId } from "./sessionId";

describe("uniqueSessionId", () => {
  it("keeps an unused id", () => {
    expect(uniqueSessionId("authorization-failure", ["other-session"])).toBe("authorization-failure");
  });

  it("uses the first free readable suffix", () => {
    expect(uniqueSessionId("authorization-failure", ["authorization-failure", "authorization-failure-2"])).toBe("authorization-failure-3");
  });

  it("keeps suffixed ids within the storage limit", () => {
    const base = "a".repeat(96);
    const result = uniqueSessionId(base, [base]);
    expect(result).toHaveLength(96);
    expect(result.endsWith("-2")).toBe(true);
  });
});
