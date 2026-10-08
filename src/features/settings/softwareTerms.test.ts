import { describe, expect, it } from "vitest";
import { missingTerms, SOFTWARE_TERMS } from "./softwareTerms";

describe("missingTerms", () => {
  it("skips terms already there, ignoring case and spaces", () => {
    expect(missingTerms([" Async ", "json"], ["async", "JSON", "webhook"])).toEqual(["webhook"]);
  });

  it("returns every term for an empty vocabulary", () => {
    expect(missingTerms([], ["async", "JSON"])).toEqual(["async", "JSON"]);
  });
});

describe("SOFTWARE_TERMS", () => {
  it("lists each term once", () => {
    const lower = SOFTWARE_TERMS.map((term) => term.toLowerCase());
    expect(new Set(lower).size).toBe(lower.length);
  });
});
