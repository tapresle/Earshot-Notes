// Shared by the renderer and the mixer. The stage (stage.html) keeps its own
// copy of the beat grid, because it runs in the browser: change both together.

/** The video's length in seconds. The mixer fades the music out over its last 2 s. */
export const DURATION = 82.6;
export const FPS = 30;
