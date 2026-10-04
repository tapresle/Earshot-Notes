import { describe, expect, it } from "vitest";
import type { Meeting } from "../../lib/api";
import { groupByDay } from "./groupByDay";

function meeting(id: number, createdAt: Date): Meeting {
  return { id, title: `Meeting ${id}`, createdAt: createdAt.getTime(), startedAt: null, endedAt: null };
}

const now = new Date(2026, 9, 3, 15, 0).getTime();

describe("groupByDay", () => {
  it("labels today and yesterday", () => {
    const groups = groupByDay(
      [meeting(3, new Date(2026, 9, 3, 10)), meeting(2, new Date(2026, 9, 3, 9)), meeting(1, new Date(2026, 9, 2, 23))],
      now,
      "en-US",
    );

    expect(groups.map((g) => [g.label, g.meetings.map((m) => m.id)])).toEqual([
      ["Today", [3, 2]],
      ["Yesterday", [1]],
    ]);
  });

  it("labels older days in the current year without the year", () => {
    const groups = groupByDay([meeting(1, new Date(2026, 8, 28, 12))], now, "en-US");

    expect(groups[0].label).toBe("Mon, Sep 28");
  });

  it("includes the year for earlier years", () => {
    const groups = groupByDay([meeting(1, new Date(2025, 11, 31, 12))], now, "en-US");

    expect(groups[0].label).toBe("Wed, Dec 31, 2025");
  });

  it("returns no groups for no meetings", () => {
    expect(groupByDay([], now)).toEqual([]);
  });
});
