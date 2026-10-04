export type SelectMode = "single" | "toggle" | "range";

export interface Selection {
  ids: number[];
  /** Where the next range selection starts. */
  anchor: number | null;
}

/** Finder-style selection: a click selects one, Cmd-click toggles, Shift-click selects a range. */
export function select(order: number[], current: Selection, clicked: number, mode: SelectMode): Selection {
  switch (mode) {
    case "single":
      return { ids: [clicked], anchor: clicked };
    case "toggle":
      return {
        ids: current.ids.includes(clicked) ? current.ids.filter((id) => id !== clicked) : [...current.ids, clicked],
        anchor: clicked,
      };
    case "range": {
      const from = order.indexOf(current.anchor ?? clicked);
      const to = order.indexOf(clicked);
      if (from === -1 || to === -1) return { ids: [clicked], anchor: clicked };
      const [start, end] = from <= to ? [from, to] : [to, from];
      return { ids: order.slice(start, end + 1), anchor: current.anchor ?? clicked };
    }
  }
}
