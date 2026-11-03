/** How close to the bottom, in pixels, counts as "at the bottom" for following. */
export const NEAR_BOTTOM_PX = 32;

/** Whether the user is at the bottom of a scrolling pane, from its metrics. */
export function isNearBottom(pane: { scrollHeight: number; scrollTop: number; clientHeight: number }): boolean {
  return pane.scrollHeight - pane.scrollTop - pane.clientHeight <= NEAR_BOTTOM_PX;
}
