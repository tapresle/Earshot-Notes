import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useToday } from "./useToday";

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 3, 23, 59));
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useToday", () => {
  it("moves to the next day at local midnight", () => {
    const { result } = renderHook(() => useToday());
    expect(result.current).toBe(new Date(2026, 9, 3).getTime());

    act(() => vi.advanceTimersByTime(60_000));
    expect(result.current).toBe(new Date(2026, 9, 4).getTime());

    act(() => vi.advanceTimersByTime(24 * 60 * 60_000));
    expect(result.current).toBe(new Date(2026, 9, 5).getTime());
  });

  it("checks the day when the window regains focus", () => {
    const { result } = renderHook(() => useToday());

    // A sleeping computer moves the clock without firing timers.
    vi.setSystemTime(new Date(2026, 9, 6, 9, 0));
    expect(result.current).toBe(new Date(2026, 9, 3).getTime());

    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(result.current).toBe(new Date(2026, 9, 6).getTime());
  });
});
