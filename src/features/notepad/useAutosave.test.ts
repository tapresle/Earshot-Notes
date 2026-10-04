import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAutosave } from "./useAutosave";

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useAutosave", () => {
  it("saves only the latest value after the delay", async () => {
    const save = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => useAutosave(save, 500));

    act(() => {
      result.current.schedule("a");
      result.current.schedule("ab");
    });
    expect(result.current.status).toBe("unsaved");
    await act(() => vi.advanceTimersByTimeAsync(500));

    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("ab");
    expect(result.current.status).toBe("saved");
  });

  it("reports an error and retries the failed value on the next flush", async () => {
    const save = vi.fn().mockRejectedValueOnce(new Error("disk full")).mockResolvedValue(undefined);
    const { result } = renderHook(() => useAutosave(save, 500));

    act(() => result.current.schedule("notes"));
    await act(() => vi.advanceTimersByTimeAsync(500));
    expect(result.current.status).toBe("error");

    await act(() => result.current.flush());

    expect(save).toHaveBeenLastCalledWith("notes");
    expect(result.current.status).toBe("saved");
  });

  it("saves edits made during an in-flight save afterward, in order", async () => {
    let finishFirst: () => void = () => {};
    const save = vi
      .fn()
      .mockImplementationOnce(() => new Promise<void>((resolve) => (finishFirst = resolve)))
      .mockResolvedValue(undefined);
    const { result } = renderHook(() => useAutosave(save, 500));

    act(() => result.current.schedule("first"));
    await act(() => vi.advanceTimersByTimeAsync(500));
    act(() => result.current.schedule("second"));
    await act(() => vi.advanceTimersByTimeAsync(500));
    expect(save).toHaveBeenCalledTimes(1);

    await act(async () => finishFirst());

    expect(save.mock.calls).toEqual([["first"], ["second"]]);
    expect(result.current.status).toBe("saved");
  });

  it("saves a pending value immediately on unmount", async () => {
    const save = vi.fn().mockResolvedValue(undefined);
    const { result, unmount } = renderHook(() => useAutosave(save, 500));

    act(() => result.current.schedule("unsaved edit"));
    unmount();
    await vi.runAllTimersAsync();

    expect(save).toHaveBeenCalledWith("unsaved edit");
  });
});
