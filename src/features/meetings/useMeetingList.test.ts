import { describe, expect, it } from "vitest";
import type { Meeting } from "../../lib/api";
import { appendPage } from "./useMeetingList";

function meeting(id: number): Meeting {
  return { id, title: `Meeting ${id}`, createdAt: 1000 - id, startedAt: null, endedAt: null };
}

describe("appendPage", () => {
  it("adds the page after the loaded meetings", () => {
    const page = { meetings: [meeting(3), meeting(4)], next: null };

    expect(appendPage([meeting(1), meeting(2)], page).map((m) => m.id)).toEqual([1, 2, 3, 4]);
  });

  it("skips meetings that are already loaded", () => {
    const page = { meetings: [meeting(2), meeting(3)], next: { createdAt: 997, id: 3 } };

    expect(appendPage([meeting(1), meeting(2)], page).map((m) => m.id)).toEqual([1, 2, 3]);
  });
});
