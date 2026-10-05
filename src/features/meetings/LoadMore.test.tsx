import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LoadMore } from "./LoadMore";

type Callback = (entries: { isIntersecting: boolean }[]) => void;

/** A stand-in for the browser's IntersectionObserver that tests can trigger. */
function stubObserver() {
  const observers: { callback: Callback; disconnected: boolean }[] = [];
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      private entry: { callback: Callback; disconnected: boolean };
      constructor(callback: Callback) {
        this.entry = { callback, disconnected: false };
        observers.push(this.entry);
      }
      observe() {}
      disconnect() {
        this.entry.disconnected = true;
      }
    },
  );
  return {
    /** Reports the marker as visible to every live observer. */
    scrollIntoView: () => observers.filter((o) => !o.disconnected).forEach((o) => o.callback([{ isIntersecting: true }])),
    live: () => observers.filter((o) => !o.disconnected).length,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("LoadMore", () => {
  it("loads more when it scrolls into view", () => {
    const observer = stubObserver();
    const onLoadMore = vi.fn();
    render(<LoadMore hasMore loading={false} error={null} onLoadMore={onLoadMore} />);

    observer.scrollIntoView();

    expect(onLoadMore).toHaveBeenCalledTimes(1);
  });

  it("stops watching while a page loads, after an error, and at the end", () => {
    const observer = stubObserver();
    const onLoadMore = vi.fn();
    const { rerender } = render(<LoadMore hasMore loading error={null} onLoadMore={onLoadMore} />);
    expect(observer.live()).toBe(0);

    rerender(<LoadMore hasMore loading={false} error="disk I/O error" onLoadMore={onLoadMore} />);
    expect(observer.live()).toBe(0);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't load more meetings. disk I/O error");

    rerender(<LoadMore hasMore={false} loading={false} error={null} onLoadMore={onLoadMore} />);
    expect(observer.live()).toBe(0);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();

    observer.scrollIntoView();
    expect(onLoadMore).not.toHaveBeenCalled();
  });

  it("shows progress on the button while loading", () => {
    render(<LoadMore hasMore loading error={null} onLoadMore={() => {}} />);

    expect(screen.getByRole("button", { name: "Loading…" })).toBeDisabled();
  });
});
