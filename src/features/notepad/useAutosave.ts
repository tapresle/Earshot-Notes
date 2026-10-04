import { useCallback, useEffect, useRef, useState } from "react";

export type SaveStatus = "saved" | "unsaved" | "saving" | "error";

/**
 * Debounces saves of the latest value. Saves run one at a time in order, so an
 * older value never lands after a newer one. A pending value is saved
 * immediately on unmount, for example when you switch meetings.
 */
export function useAutosave(save: (value: string) => Promise<unknown>, delayMs = 500) {
  const [status, setStatus] = useState<SaveStatus>("saved");
  const saveRef = useRef(save);
  const pending = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const queue = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    saveRef.current = save;
  });

  const flush = useCallback(() => {
    clearTimeout(timer.current);
    queue.current = queue.current.then(async () => {
      const value = pending.current;
      if (value === null) return;
      pending.current = null;
      setStatus("saving");
      try {
        await saveRef.current(value);
        setStatus(pending.current === null ? "saved" : "unsaved");
      } catch {
        // Keep the failed value unless a newer edit replaced it, so the next flush retries it.
        pending.current ??= value;
        setStatus("error");
      }
    });
    return queue.current;
  }, []);

  const schedule = useCallback(
    (value: string) => {
      pending.current = value;
      setStatus("unsaved");
      clearTimeout(timer.current);
      timer.current = setTimeout(flush, delayMs);
    },
    [flush, delayMs],
  );

  useEffect(() => () => void flush(), [flush]);

  return { status, schedule, flush };
}
