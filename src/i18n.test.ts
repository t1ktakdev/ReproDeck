import { describe, expect, it } from "vitest";
import { translationCoverage } from "./i18n";

describe("translations", () => {
  it("keeps English and Russian dictionaries in sync", () => {
    expect(translationCoverage()).toEqual({ missingInEnglish: [], missingInRussian: [] });
  });
});
