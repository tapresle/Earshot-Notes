import { useEffect, useState } from "react";
import { startOfDay } from "./groupByDay";

/** Milliseconds from `now` until the next local midnight. */
function msUntilNextDay(now: number): number {
  const next = new Date(startOfDay(now));
  next.setDate(next.getDate() + 1);
  return next.getTime() - now;
}

/**
 * The start of the current local day, updated at midnight. Timers can run late
 * after the computer sleeps, so it also checks when the window regains focus or
 * becomes visible.
 */
export function useToday(): number {
  const [today, setToday] = useState(() => startOfDay(Date.now()));

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const update = () => {
      setToday(startOfDay(Date.now()));
      clearTimeout(timer);
      timer = setTimeout(update, msUntilNextDay(Date.now()));
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") update();
    };

    timer = setTimeout(update, msUntilNextDay(Date.now()));
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, []);

  return today;
}
