import { useEffect, useState } from "react";

/** `value`, once it has stopped changing for `delayMs`. An empty string comes through at once. */
export function useDebounced(value: string, delayMs: number): string {
  const [settled, setSettled] = useState(value);

  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), value === "" ? 0 : delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);

  return settled;
}
