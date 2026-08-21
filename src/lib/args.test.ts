import { describe, expect, it } from "vitest";
import { formatArguments, parseArguments } from "./args";

describe("parseArguments", () => {
  it("keeps quoted values as one argv item", () => {
    expect(parseArguments('test --name "hello world"')).toEqual(["test", "--name", "hello world"]);
  });
  it("never expands shell operators", () => {
    expect(parseArguments('one && two')).toEqual(["one", "&&", "two"]);
  });
  it("rejects an unclosed quote", () => {
    expect(() => parseArguments('"unfinished')).toThrow(/Unclosed quote/);
  });

  it("round-trips editable argv including spaces, quotes and Windows paths", () => {
    const values = ["test", "--name", "tenant cache", "C:\\Program Files\\fixture", "say\"hello", ""];
    expect(parseArguments(formatArguments(values))).toEqual(values);
  });
});
