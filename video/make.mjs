// Makes the highlight video from this checkout. See README.md.
//   node video/make.mjs                    the whole video, about 15 minutes
//   node video/make.mjs stills 16.5,54.9   PNGs at those seconds, to check a scene quickly
//   node video/make.mjs audio              remixes the soundtrack onto the last render
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { fetchAudio } from "./audio.mjs";
import { ffmpeg, loudness, mix } from "./mix.mjs";
import { render } from "./render.mjs";
import { DURATION } from "./timing.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, "out");
const SILENT = join(OUT, "silent.mp4");
const CUES = join(OUT, "cues.json");
const NAME = "earshot-notes-highlight";
// The soundtrack's loudness, set once by ear. A static gain reaches it, so
// regenerating a longer or busier video keeps the same level.
const TARGET_LUFS = -21.8;

const [mode = "video", times] = process.argv.slice(2);

if (spawnSync("ffmpeg", ["-version"]).error) {
  console.error("This needs ffmpeg. Install it with: brew install ffmpeg");
  process.exit(1);
}
mkdirSync(OUT, { recursive: true });

if (mode === "stills") {
  if (!times) throw new Error("List the seconds to capture, such as: node video/make.mjs stills 16.5,54.9");
  const dir = join(OUT, "stills");
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir);
  await render(dir, times.split(",").map(Number));
  console.log(`Wrote stills to ${dir}`);
} else if (mode === "video" || mode === "audio") {
  if (mode === "video") {
    console.log("Rendering frames");
    const cues = await render(SILENT);
    writeFileSync(CUES, JSON.stringify(cues, null, 1));
  } else if (!existsSync(SILENT) || !existsSync(CUES)) {
    throw new Error("There's no render to remix yet. Run: node video/make.mjs");
  }
  await soundtrack();
} else {
  throw new Error(`Unknown mode "${mode}". Use no mode for the video, "stills", or "audio".`);
}

async function soundtrack() {
  const cues = JSON.parse(readFileSync(CUES, "utf8"));
  const paths = await fetchAudio(join(HERE, ".cache"));

  console.log("Mixing the soundtrack");
  const probe = join(OUT, "probe.wav");
  mix({ cues, paths, out: probe });
  const gainDb = TARGET_LUFS - loudness(probe).integrated;
  rmSync(probe);
  const audio = join(OUT, "soundtrack.m4a");
  mix({ cues, paths, out: audio, gainDb });
  const level = loudness(audio);
  console.log(`  ${level.integrated} LUFS, peaks at ${level.peak} dBFS`);
  if (level.peak > -1) console.warn("  Warning: the mix peaks above -1 dBFS and may distort. Lower a sound effect's gain in mix.mjs.");

  const full = join(OUT, `${NAME}.mp4`);
  ffmpeg(["-i", SILENT, "-i", audio, "-map", "0:v", "-map", "1:a", "-c", "copy", "-t", String(DURATION), "-movflags", "+faststart", full]);
  // A copy small enough to attach or send from a phone.
  const small = join(OUT, `${NAME}-small.mp4`);
  ffmpeg(["-i", full, "-vf", "scale=1280:-2", "-c:v", "libx264", "-preset", "slow", "-crf", "23",
    "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart", small]);
  // One frame every 4.5 s, to review the whole video at a glance.
  const sheet = join(OUT, "review-sheet.png");
  ffmpeg(["-i", full, "-vf", "fps=1/4.5,scale=480:-1,tile=6x3:padding=4", "-frames:v", "1", "-update", "1", sheet]);

  console.log(`Wrote ${full}\n      ${small}\n      ${sheet}`);
}
