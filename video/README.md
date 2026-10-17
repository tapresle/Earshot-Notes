# Highlight video

A script-driven highlight video of Earshot Notes, rendered from the app itself. It drives the real UI through the preview build (`preview.html` and the fake backend in `src/testing/fakeBackend.ts`), so each regeneration shows the app as it is in your checkout.

Nothing here is part of the app. The app build, the website, linting, type checks, tests, and the screenshot checks all leave this folder alone. Its output and downloaded audio go to `video/out/` and `video/.cache/`, which git ignores.

## Requirements

- `npm install` in the repository root. The renderer uses the repository's Playwright and Vite.
- ffmpeg, for encoding and mixing: `brew install ffmpeg`.
- A network connection on the first run, to download the audio (see [Audio](#audio)).

## Make the video

To render the video from the code you have checked out, run this from the repository root:

```bash
node video/make.mjs
```

It takes about 15 minutes and writes these files to `video/out/`:

- `earshot-notes-highlight.mp4`: 1920 by 1080, 30 frames per second, about 45 MB.
- `earshot-notes-highlight-small.mp4`: 1280 by 720, about 10 MB, for attaching or sending from a phone.
- `review-sheet.png`: one frame every 4.5 seconds, to check the whole video at a glance.

To check a scene without rendering everything, capture stills at the seconds you want. They go to `video/out/stills/`:

```bash
node video/make.mjs stills 16.5,31.4,54.9
```

To change only the audio, remix onto the last render:

```bash
node video/make.mjs audio
```

## Update it for a release

1. Check out the commit you're releasing, usually `main`.
2. Compare the README's feature list and the release notes with the scenes below. Add, change, or drop scenes to match, and keep every claim true of the app.
3. Check the scenes you changed with `stills`.
4. Run `node video/make.mjs`, then watch the video and look at `review-sheet.png`.

When the app's UI changes, a scene can fail to find a button or heading. The error names the locator, which lives in `render.mjs`.

## How it fits together

| File | What it holds |
|---|---|
| `make.mjs` | The command. It renders, mixes, and packages. |
| `render.mjs` | What happens inside the app and when: clicks, typing, the transcript, summary, and chat it streams in, and the camera's zoom keys. Each scene is a numbered section. |
| `stage.html` | Everything around the app: the intro, the problem scene, the headlines (`HEADS`) and chips (`CHIPS`) over the demo, the comparison table, the outro, and the end card credit. Scene start times are in `T`. |
| `mix.mjs` | The soundtrack: the music cut on its bar lines, the sound effect for each cue, and the loudness measurement. |
| `audio.mjs` | The music and sound effect sources, pinned by SHA-256. |
| `timing.mjs` | The video's length and frame rate. |

### Timing

Everything sits on a 128 BPM grid: `bar(k)` is the start of bar `k`, 1.875 seconds long, from the first beat at 0.438 seconds. Scene cuts land on bar lines, so they land on the music's downbeats too.

- **The demo:** from `bar(6)` to `bar(32)`, one headline every 4 bars. Each headline names one feature, and `render.mjs` shows it in that window.
- **The comparison:** from `bar(32)`.
- **The outro:** from `bar(38)`.

To add a scene, lengthen the video in whole multiples of 4 bars (7.5 seconds), and change these together:

1. `DURATION` in `timing.mjs`.
2. `T`, and the outro's `bar()` times, in `stage.html`.
3. The actions and camera keys in `render.mjs`, for the scenes after the new one.
4. The music in `mix.mjs`: add 3 track bars to a segment in `SEGMENTS` for every 4 video bars (see the comment there), and move `DUCK.to` to the comparison's start.

### How the renderer drives the app

The renderer runs Vite on port 1431, opens `stage.html` with the app in an iframe, and steps time one frame at a time. Each frame:

1. It runs the actions due by then. The actions click and type through Playwright, and emit Tauri events such as `transcript:segment`, `summary:text`, and `chat:done` through the preview's mocked IPC.
2. It renders the stage at that time.
3. It captures a screenshot and pipes it to ffmpeg.

The stage's `render(t)` is a pure function of time, so a frame looks the same however long the capture takes.

The page clock is fixed at October 6, 2026, so the meeting list's dates don't change between renders. The demo's people and companies are invented. Keep them that way: never use names from real meetings.

## Audio

The music and sound effects aren't committed. `audio.mjs` downloads them from Freesound on the first run and checks each file's SHA-256. If a file changes upstream, the script stops: listen to the new file, check that its license still allows use, and then update the pin.

- **Music:** "Lo-fi Music Guitar (Short version)" by Migfus20, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), from [Freesound](https://freesound.org/people/Migfus20/sounds/578131). It's slowed from 100 to 96 BPM and rearranged on its bar lines.
- **Sound effects (CC0 1.0):** "Mouse Click" by Sillyslimeball, "Keyboard Typing Tapping.wav" by Psykophobia, and "Chime Notification" by Jofae, all from Freesound.

The music's license requires credit. The end card shows it, and when you post the video, put this in the description too:

```text
Music: "Lo-fi Music Guitar (Short version)" by Migfus20, licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). https://freesound.org/people/Migfus20/sounds/578131. Slowed and edited.
```

The soundtrack's level is set by ear: `make.mjs` measures the mix and applies a static gain to reach `TARGET_LUFS`, -21.8 LUFS. Don't use loudnorm or a limiter. Both made the loud parts sound gritty. If the mix peaks above -1 dBFS, the script warns you; lower that sound effect's gain in `mix.mjs`.
