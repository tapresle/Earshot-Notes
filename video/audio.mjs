// The music and sound effects, downloaded from Freesound on first use and
// checked against pinned SHA-256 checksums. They aren't committed.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const SOURCES = {
  music: {
    file: "lofi-guitar.mp3",
    url: "https://cdn.freesound.org/previews/578/578131_12295155-hq.mp3",
    sha256: "db6cdc7b0f093d0fc2b8d42a61a5b9ec82dbda222c8cd3c8e7316ca7fbdae252",
    credit: "“Lo-fi Music Guitar (Short version)” by Migfus20, CC BY 4.0, https://freesound.org/people/Migfus20/sounds/578131",
  },
  click: {
    file: "click.mp3",
    url: "https://cdn.freesound.org/previews/333/333575_5889445-hq.mp3",
    sha256: "0b4758ebba48028f2805f30ffd2c65a776f09d2140e1508b20ef0243b2734454",
    credit: "“Mouse Click” by Sillyslimeball, CC0 1.0, https://freesound.org/people/Sillyslimeball/sounds/333575",
  },
  chime: {
    file: "chime.mp3",
    url: "https://cdn.freesound.org/previews/380/380482_6512973-hq.mp3",
    sha256: "31e06d1c9339980fa31c4cdec89811846576b69499218964efbc77c8f9eab83e",
    credit: "“Chime Notification” by Jofae, CC0 1.0, https://freesound.org/people/Jofae/sounds/380482",
  },
  type: {
    file: "typing.mp3",
    url: "https://cdn.freesound.org/previews/250/250390_10476-hq.mp3",
    sha256: "16e2ab55f41722349a8d3396cd1f306f81e1ab45de92c8682baaa7ef58173e9d",
    credit: "“Keyboard Typing Tapping.wav” by Psykophobia, CC0 1.0, https://freesound.org/people/Psykophobia/sounds/250390",
  },
};

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

/** Downloads any missing or changed file into `dir` and returns each sound's path by name. */
export async function fetchAudio(dir) {
  mkdirSync(dir, { recursive: true });
  const paths = {};
  for (const [name, source] of Object.entries(SOURCES)) {
    const path = join(dir, source.file);
    if (!existsSync(path) || sha256(readFileSync(path)) !== source.sha256) {
      console.log(`Downloading ${source.file} from ${source.url}`);
      const response = await fetch(source.url, { headers: { "User-Agent": "earshot-notes-video" } });
      if (!response.ok) throw new Error(`Couldn't download ${source.url}: HTTP ${response.status}.`);
      const bytes = Buffer.from(await response.arrayBuffer());
      const actual = sha256(bytes);
      if (actual !== source.sha256) {
        throw new Error(
          `${source.url} changed: its SHA-256 is ${actual}, not ${source.sha256}. ` +
            "Listen to the new file and check its license before updating the pin in audio.mjs.",
        );
      }
      writeFileSync(path, bytes);
    }
    paths[name] = path;
  }
  return paths;
}
