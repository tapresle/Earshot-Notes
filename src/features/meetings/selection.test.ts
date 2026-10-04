import { describe, expect, it } from "vitest";
import { select } from "./selection";

const order = [5, 4, 3, 2, 1];

describe("select", () => {
  it("selects one meeting on a plain click", () => {
    expect(select(order, { ids: [5, 4], anchor: 5 }, 3, "single")).toEqual({ ids: [3], anchor: 3 });
  });

  it("adds and removes with toggle", () => {
    const added = select(order, { ids: [5], anchor: 5 }, 3, "toggle");
    expect(added).toEqual({ ids: [5, 3], anchor: 3 });
    expect(select(order, added, 5, "toggle")).toEqual({ ids: [3], anchor: 5 });
  });

  it("selects a range from the anchor in either direction", () => {
    expect(select(order, { ids: [4], anchor: 4 }, 2, "range").ids).toEqual([4, 3, 2]);
    expect(select(order, { ids: [2], anchor: 2 }, 4, "range").ids).toEqual([4, 3, 2]);
  });

  it("keeps the anchor so a second range click re-extends from it", () => {
    const first = select(order, { ids: [4], anchor: 4 }, 2, "range");
    expect(select(order, first, 5, "range").ids).toEqual([5, 4]);
  });

  it("falls back to a single selection when the anchor is gone", () => {
    expect(select(order, { ids: [], anchor: 99 }, 2, "range")).toEqual({ ids: [2], anchor: 2 });
  });
});
