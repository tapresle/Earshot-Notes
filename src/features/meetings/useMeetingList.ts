import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, type Meeting, type MeetingCursor, type MeetingPage } from "../../lib/api";

export type MeetingListState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      meetings: Meeting[];
      /** Where the next page starts, or null when every meeting is loaded. */
      next: MeetingCursor | null;
      loadingMore: boolean;
      /** Why the last attempt to load more failed. */
      moreError: string | null;
    };

/** Adds a page to the meetings already loaded, skipping any that are already there. */
export function appendPage(meetings: Meeting[], page: MeetingPage): Meeting[] {
  const loaded = new Set(meetings.map((m) => m.id));
  return [...meetings, ...page.meetings.filter((m) => !loaded.has(m.id))];
}

/** The sidebar's meetings, loaded a page at a time, newest first. */
export function useMeetingList() {
  const [list, setList] = useState<MeetingListState>({ kind: "loading" });
  // Bumped on every reload, so a response for an older load is ignored.
  const generation = useRef(0);
  const loadingMore = useRef(false);

  const loadFirstPage = useCallback(() => {
    const current = ++generation.current;
    loadingMore.current = false;
    api.listMeetings().then(
      (page) => {
        if (current !== generation.current) return;
        setList({ kind: "ready", meetings: page.meetings, next: page.next, loadingMore: false, moreError: null });
      },
      (error) => {
        if (current !== generation.current) return;
        setList({ kind: "error", message: errorMessage(error) });
      },
    );
  }, []);

  useEffect(loadFirstPage, [loadFirstPage]);

  /** Starts over from the first page, such as after a failed load. */
  const reload = useCallback(() => {
    setList({ kind: "loading" });
    loadFirstPage();
  }, [loadFirstPage]);

  const next = list.kind === "ready" ? list.next : null;
  const loadMore = useCallback(() => {
    if (next === null || loadingMore.current) return;
    const current = generation.current;
    loadingMore.current = true;
    setList((state) => (state.kind === "ready" ? { ...state, loadingMore: true, moreError: null } : state));
    api.listMeetings(next).then(
      (page) => {
        if (current !== generation.current) return;
        loadingMore.current = false;
        setList((state) =>
          state.kind === "ready"
            ? { ...state, meetings: appendPage(state.meetings, page), next: page.next, loadingMore: false }
            : state,
        );
      },
      (error) => {
        if (current !== generation.current) return;
        loadingMore.current = false;
        setList((state) =>
          state.kind === "ready" ? { ...state, loadingMore: false, moreError: errorMessage(error) } : state,
        );
      },
    );
  }, [next]);

  /** Changes the loaded meetings in place, such as after a rename or delete. */
  const update = useCallback((change: (meetings: Meeting[]) => Meeting[]) => {
    setList((state) => (state.kind === "ready" ? { ...state, meetings: change(state.meetings) } : state));
  }, []);

  return { list, reload, loadMore, update };
}
