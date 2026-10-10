/** How long a segment stays highlighted after a citation shows it. */
const HIGHLIGHT_MS = 2000;

/** The element id of a transcript segment in the transcript panel. */
export const segmentElementId = (segmentId: number) => `segment-${segmentId}`;

/**
 * Scrolls the transcript panel to a segment, focuses it, and highlights it
 * briefly. A segment removed since it was cited falls back to the one that
 * starts nearest `startMs`. Returns whether a segment was shown.
 */
export function showSegment(segmentId: number, startMs: number): boolean {
  const target = document.getElementById(segmentElementId(segmentId)) ?? nearest(startMs);
  if (!target) return false;
  target.scrollIntoView?.({ block: "center" });
  target.focus({ preventScroll: true });
  target.classList.add("is-cited");
  window.setTimeout(() => target.classList.remove("is-cited"), HIGHLIGHT_MS);
  return true;
}

function nearest(startMs: number): HTMLElement | null {
  let best: HTMLElement | null = null;
  let bestDistance = Infinity;
  for (const element of document.querySelectorAll<HTMLElement>("[data-start-ms]")) {
    const distance = Math.abs(Number(element.dataset.startMs) - startMs);
    if (distance < bestDistance) {
      best = element;
      bestDistance = distance;
    }
  }
  return best;
}
