// Builds the soundtrack: the music, cut on its bar lines to fit the video's
// scenes, and a click, typing, or chime for each cue the renderer recorded.
import { spawnSync } from "node:child_process";
import { DURATION } from "./timing.mjs";

const T0 = 0.438; // The video's first beat.

// The track runs at 100 BPM. Slowed to 96 BPM, three of its beats fill one
// beat of the video's 128 BPM grid, so 3 track bars fill 4 video bars and
// every scene cut lands on a downbeat. To lengthen the video by 4 bars, add
// 3 bars of the track.
const STRETCH = 0.96;
const bar = (k) => 0.575 + k * 2.4; // The track's bar lines, in track seconds.
const SEGMENTS = [
  // [from, to] in track seconds, played back to back.
  [bar(38), bar(48)], // Two full bars, the breakdown, and the drums' return at 15.44 s.
  [bar(21), bar(40)], // The main groove for the rest of the demo.
  [bar(52), 134.65], // The last bars, ending with the video.
];
const CROSSFADE = 0.03; // At each join, centered on the bar line.
// The music dips under the demo, from the first click to the comparison.
const DUCK = { from: 15.4, to: 60.4, by: 0.15 };

// name: gain in dB, and where the sound is loudest (the start is trimmed to it, so it lands on the cue).
const SOUNDS = {
  click: { gain: 5, loudest: 0.1 },
  chime: { gain: -4, loudest: 0 },
  type: { gain: 0, loudest: 0.1 },
};

/**
 * Mixes the soundtrack into `out` (.wav or .m4a).
 * `cues` come from the renderer, `paths` from fetchAudio, and `gainDb` sets the overall level.
 */
export function mix({ cues, paths, out, gainDb = 0 }) {
  const inputs = ["-i", paths.music];
  const filters = [];
  SEGMENTS.forEach(([a, b], i) => {
    const lo = a - (i ? CROSSFADE / 2 : 0);
    const hi = b + (i < SEGMENTS.length - 1 ? CROSSFADE / 2 : 0);
    filters.push(`[0:a]atrim=${lo.toFixed(4)}:${hi.toFixed(4)},asetpts=N/SR/TB,atempo=${STRETCH}[seg${i}]`);
  });
  let joined = "[seg0]";
  for (let i = 1; i < SEGMENTS.length; i++) {
    filters.push(`${joined}[seg${i}]acrossfade=d=${CROSSFADE}:c1=tri:c2=tri[join${i}]`);
    joined = `[join${i}]`;
  }
  const duck = `1-${DUCK.by}*clip((t-${DUCK.from})/0.8,0,1)*clip((${DUCK.to}-t)/0.8,0,1)`;
  filters.push(
    `${joined}afade=t=in:d=0.05,adelay=${Math.round(T0 * 1000)}:all=1,` +
      `afade=t=out:st=${(DURATION - 2).toFixed(3)}:d=2,volume='${duck}':eval=frame,` +
      "aresample=48000,aformat=channel_layouts=stereo[music]",
  );

  const effects = cues.map((cue, i) => {
    const { gain, loudest } = SOUNDS[cue.name];
    const n = i + 1;
    inputs.push("-i", paths[cue.name]);
    let chain = `[${n}:a]aresample=48000,aformat=channel_layouts=stereo`;
    if (cue.name === "type") {
      // Loops the typing for as long as the text takes to type.
      const length = cue.t1 - cue.t;
      chain +=
        `,aloop=loop=-1:size=2e6,atrim=${loudest}:${(loudest + length).toFixed(3)},asetpts=N/SR/TB,` +
        `afade=t=out:st=${Math.max(0, length - 0.15).toFixed(3)}:d=0.15`;
    } else {
      chain += `,atrim=${loudest},asetpts=N/SR/TB`;
      // Keeps the click's body and drops its single-sample spike, which made the mix peak.
      if (cue.name === "click") chain += ",highpass=f=250,lowpass=f=5000";
    }
    filters.push(`${chain},volume=${gain}dB,adelay=${Math.round(cue.t * 1000)}:all=1[fx${n}]`);
    return `[fx${n}]`;
  });
  filters.push(`${effects.join("")}amix=inputs=${effects.length}:normalize=0[effects]`);
  filters.push("[music][effects]amix=inputs=2:normalize=0:duration=first[mix]");
  // A static gain, never loudnorm or a limiter: both made the loud parts sound gritty.
  filters.push(`[mix]atrim=0:${DURATION},volume=${gainDb.toFixed(2)}dB[out]`);

  const codec = out.endsWith(".wav") ? ["-c:a", "pcm_f32le"] : ["-c:a", "aac", "-b:a", "256k"];
  ffmpeg([...inputs, "-filter_complex", filters.join(";"), "-map", "[out]", "-ar", "48000", ...codec, out]);
}

/** Returns a file's integrated loudness (LUFS) and true peak (dBTP). */
export function loudness(file) {
  const result = spawnSync("ffmpeg", ["-hide_banner", "-i", file, "-af", "ebur128=peak=true", "-f", "null", "-"], {
    encoding: "utf8",
  });
  const summary = result.stderr.slice(result.stderr.lastIndexOf("Summary:"));
  const integrated = Number(summary.match(/I:\s+(-?[\d.]+) LUFS/)?.[1]);
  const peak = Number(summary.match(/Peak:\s+(-?[\d.]+) dBFS/)?.[1]);
  if (Number.isNaN(integrated)) throw new Error(`Couldn't measure the loudness of ${file}.`);
  return { integrated, peak };
}

/** Runs ffmpeg and fails with its error output. */
export function ffmpeg(args) {
  const result = spawnSync("ffmpeg", ["-y", "-v", "error", ...args], { encoding: "utf8" });
  if (result.status !== 0) throw new Error(`ffmpeg failed:\n${result.stderr}`);
}
