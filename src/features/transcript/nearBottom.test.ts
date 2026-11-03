import { describe, expect, it } from "vitest";
import { NEAR_BOTTOM_PX, isNearBottom } from "./nearBottom";

function pane(scrollTop: number, scrollHeight: number, clientHeight: number) {
  return { scrollTop, scrollHeight, clientHeight };
}

describe("isNearBottom", () => {
  it("is true at the very bottom", () => {
    expect(isNearBottom(pane(700, 1000, 300))).toBe(true);
  });

  it("is true within the tolerance", () => {
    expect(isNearBottom(pane(1000 - 300 - NEAR_BOTTOM_PX, 1000, 300))).toBe(true);
  });

  it("is false beyond the tolerance", () => {
    expect(isNearBottom(pane(1000 - 300 - NEAR_BOTTOM_PX - 1, 1000, 300))).toBe(false);
  });

  it("is true when the content doesn't fill the pane", () => {
    expect(isNearBottom(pane(0, 200, 300))).toBe(true);
  });

  it("is false while reading the top of a long transcript", () => {
    expect(isNearBottom(pane(0, 5000, 300))).toBe(false);
  });
});
