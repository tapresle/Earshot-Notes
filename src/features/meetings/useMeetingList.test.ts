import { act, renderHook, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Meeting, MeetingPage } from "../../lib/api";
import { appendPage, useMeetingList } from "./useMeetingList";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

function meeting(id: number): Meeting {
  return { id, title: `Meeting ${id}`, createdAt: 1000 - id, startedAt: null, endedAt: null };
}

/** `list_meetings` calls in order, each answered when the test says so. */
function pendingPages() {
  const calls: { args: Record<string, unknown>; answer: (page: MeetingPage) => Promise<void> }[] = [];
  vi.mocked(invoke).mockImplementation(
    (_command, args) =>
      new Promise((resolve) => {
        calls.push({ args: args as Record<string, unknown>, answer: (page) => act(async () => resolve(page)) });
      }),
  );
  return calls;
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

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

describe("useMeetingList", () => {
  it("shows only the newest query's results when answers arrive out of order", async () => {
    const calls = pendingPages();
    const { result, rerender } = renderHook(({ query }) => useMeetingList(query), { initialProps: { query: "a" } });
    rerender({ query: "ab" });
    await waitFor(() => expect(calls).toHaveLength(2));

    await calls[1].answer({ meetings: [meeting(2)], next: null });
    await calls[0].answer({ meetings: [meeting(1)], next: null });

    expect(result.current.list).toMatchObject({ kind: "ready", meetings: [meeting(2)] });
  });

  it("waits for a new search before loading more", async () => {
    const calls = pendingPages();
    const { result, rerender } = renderHook(({ query }) => useMeetingList(query), { initialProps: { query: "" } });
    await waitFor(() => expect(calls).toHaveLength(1));
    await calls[0].answer({ meetings: [meeting(1)], next: { createdAt: 999, id: 1 } });

    rerender({ query: "budget" });
    await waitFor(() => expect(calls).toHaveLength(2));
    act(() => result.current.loadMore());

    expect(calls).toHaveLength(2);
    await calls[1].answer({ meetings: [meeting(5)], next: null });
    expect(result.current.list).toMatchObject({ kind: "ready", meetings: [meeting(5)], next: null });
  });

  it("asks for the page after the cursor, with the same query", async () => {
    const calls = pendingPages();
    const { result } = renderHook(() => useMeetingList("budget"));
    await waitFor(() => expect(calls).toHaveLength(1));
    await calls[0].answer({ meetings: [meeting(1)], next: { createdAt: 999, id: 1 } });

    act(() => result.current.loadMore());
    act(() => result.current.loadMore());

    expect(calls).toHaveLength(2);
    expect(calls[1].args).toEqual({ query: "budget", after: { createdAt: 999, id: 1 }, limit: 50 });
    await calls[1].answer({ meetings: [meeting(2)], next: null });
    expect(result.current.list).toMatchObject({ kind: "ready", meetings: [meeting(1), meeting(2)], next: null });
  });
});
