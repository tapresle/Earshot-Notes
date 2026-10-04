import { describe, expect, it } from "vitest";
import { formatOffset } from "./formatOffset";

describe("formatOffset", () => {
  it.each([
    [0, "0:00"],
    [999, "0:00"],
    [65_000, "1:05"],
    [3_599_000, "59:59"],
    [3_600_000, "1:00:00"],
    [3_725_000, "1:02:05"],
    [-500, "0:00"],
  ])("formats %i ms as %s", (ms, expected) => {
    expect(formatOffset(ms)).toBe(expected);
  });
});
