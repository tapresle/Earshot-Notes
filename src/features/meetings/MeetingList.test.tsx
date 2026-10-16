import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Meeting } from "../../lib/api";
import { MeetingList } from "./MeetingList";

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 3, 23, 59));
});

afterEach(() => {
  vi.useRealTimers();
});

describe("MeetingList", () => {
  it("relabels days after midnight without remounting", () => {
    const meeting: Meeting = {
      id: 1,
      title: "Standup",
      createdAt: new Date(2026, 9, 3, 10, 0).getTime(),
      startedAt: null,
      endedAt: null,
    };
    render(
      <MeetingList
        meetings={[meeting]}
        selectedId={null}
        selectedIds={[]}
        recordingMeetingId={null}
        onSelect={() => {}}
        onExport={() => {}}
        onDelete={() => {}}
        onCreate={() => {}}
        creating={false}
      />,
    );
    expect(screen.getByRole("heading", { name: "Today" })).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByRole("heading", { name: "Yesterday" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Today" })).not.toBeInTheDocument();
  });
});
