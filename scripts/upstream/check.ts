// Reports newer versions of what Earshot pins: the needle engine and Whistle
// (scripts/fetch-native.sh), the summary models (src-tauri/src/models/mod.rs),
// and llama-cpp-2 (src-tauri/Cargo.lock). Prints a Markdown report, or nothing
// when everything is current. Run weekly by .github/workflows/upstream.yml:
//
//   node scripts/upstream/check.ts
//
// Pins are read from the files that define them, so this script never needs
// editing when a pin moves. A Hugging Face pin counts as outdated only when
// its pinned file changed on `main`; commits that touch only a README don't.

import { readFile } from "node:fs/promises";

/** A file pinned to a Hugging Face revision and checksum. */
export interface FilePin {
  name: string;
  repo: string;
  revision: string;
  path: string;
  sha256: string;
}

/** A crate pinned in Cargo.lock. */
export interface CratePin {
  name: string;
  version: string;
}

/** One row of the report. */
export interface Update {
  name: string;
  pinned: string;
  upstream: string;
}

/** Fetches JSON from a URL, as a POST when there's a body. Throws on any failure. */
export type FetchJson = (url: string, body?: unknown) => Promise<unknown>;

/** Native pins by their variable prefix in fetch-native.sh. `TEST_` pins aren't shipped. */
const NATIVE_NAMES: Record<string, string> = {
  NEEDLE: "needle engine",
  WHISTLE: "Whistle",
};

export function readNativePins(script: string): FilePin[] {
  const variables = new Map<string, string>();
  for (const [, name, value] of script.matchAll(/^(\w+)="([^"$]*)"/gm)) {
    variables.set(name, value);
  }
  const pins: FilePin[] = [];
  const entry = /^\s*"\$(\w+)_REPO \$\1_REV (\S+) \S+ ([0-9a-f]{64})"$/gm;
  for (const [, prefix, path, sha256] of script.matchAll(entry)) {
    if (prefix.startsWith("TEST_")) continue;
    const name = NATIVE_NAMES[prefix];
    if (!name) throw new Error(`fetch-native.sh pins ${prefix}, which this check doesn't know; add it to NATIVE_NAMES`);
    pins.push({
      name,
      repo: required(variables, `${prefix}_REPO`),
      revision: required(variables, `${prefix}_REV`),
      path,
      sha256,
    });
  }
  if (pins.length === 0) throw new Error("found no pins in fetch-native.sh; has its FILES format changed?");
  return pins;
}

export function readModelPins(registry: string): FilePin[] {
  const pins: FilePin[] = [];
  for (const [, body] of registry.matchAll(/^pub const \w+: KnownModel = KnownModel \{\n([\s\S]*?)^\};/gm)) {
    const field = (key: string) => {
      const match = body.match(new RegExp(`^\\s*${key}: "([^"]*)",`, "m"));
      if (!match) throw new Error(`a KnownModel in models/mod.rs has no ${key}`);
      return match[1];
    };
    pins.push({
      name: field("display_name"),
      repo: field("repo"),
      revision: field("revision"),
      path: field("file_name"),
      sha256: field("sha256"),
    });
  }
  if (pins.length === 0) throw new Error("found no KnownModel entries in models/mod.rs");
  return pins;
}

export function readCratePin(lock: string, name: string): CratePin {
  const match = lock.match(new RegExp(`^name = "${name}"\\nversion = "([^"]+)"`, "m"));
  if (!match) throw new Error(`Cargo.lock has no ${name}`);
  return { name, version: match[1] };
}

interface PathInfo {
  path: string;
  lfs?: { oid: string };
  lastCommit?: { id: string; title: string; date: string };
}

/** Reports a pinned file whose content on `main` differs from the pinned checksum. */
export async function checkFile(pin: FilePin, fetchJson: FetchJson): Promise<Update | null> {
  const url = `https://huggingface.co/api/models/${pin.repo}/paths-info/main`;
  const [info] = (await fetchJson(url, { paths: [pin.path], expand: true })) as PathInfo[];
  const pinned = `\`${pin.path}\` at [\`${short(pin.revision)}\`](https://huggingface.co/${pin.repo}/tree/${pin.revision})`;
  if (!info) {
    return {
      name: pin.name,
      pinned,
      upstream: `No longer on \`main\`. See [the history](https://huggingface.co/${pin.repo}/commits/main).`,
    };
  }
  if (info.lfs?.oid === pin.sha256) return null;
  const commit = info.lastCommit;
  return {
    name: pin.name,
    pinned,
    upstream: commit
      ? `Changed on ${commit.date.slice(0, 10)} in [\`${short(commit.id)}\`](https://huggingface.co/${pin.repo}/commit/${commit.id}): ${commit.title}`
      : `Changed. See [the history](https://huggingface.co/${pin.repo}/commits/main).`,
  };
}

interface CrateInfo {
  crate: { max_stable_version: string; repository?: string };
}

/** Reports a crate whose newest stable release is newer than the locked version. */
export async function checkCrate(pin: CratePin, fetchJson: FetchJson): Promise<Update | null> {
  const { crate } = (await fetchJson(`https://crates.io/api/v1/crates/${pin.name}`)) as CrateInfo;
  if (compareVersions(crate.max_stable_version, pin.version) <= 0) return null;
  const releases = crate.repository ? `, see [the releases](${crate.repository}/releases)` : "";
  return {
    name: `\`${pin.name}\``,
    pinned: `\`${pin.version}\``,
    upstream: `[\`${crate.max_stable_version}\`](https://crates.io/crates/${pin.name}/${crate.max_stable_version})${releases}`,
  };
}

export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.split(".").map(Number);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const diff = (x[i] ?? 0) - (y[i] ?? 0);
    if (diff !== 0) return Math.sign(diff);
  }
  return 0;
}

/** The tracking issue's body, or an empty string when there's nothing to report. */
export function renderReport(updates: Update[]): string {
  if (updates.length === 0) return "";
  const cell = (text: string) => text.replaceAll("|", "\\|").replaceAll("\n", " ");
  return [
    "The weekly upstream check (`.github/workflows/upstream.yml`) found newer versions of what Earshot pins. The update policy in `AGENTS.md` says when a release takes each one. This issue updates itself each week and closes when everything is current.",
    "",
    "| Pin | Pinned | Upstream |",
    "| --- | --- | --- |",
    ...updates.map((u) => `| ${cell(u.name)} | ${cell(u.pinned)} | ${cell(u.upstream)} |`),
    "",
  ].join("\n");
}

/** Checks every pin, one request at a time; there are only a handful. */
export async function checkAll(files: FilePin[], crates: CratePin[], fetchJson: FetchJson): Promise<Update[]> {
  const updates: Update[] = [];
  for (const pin of files) {
    const update = await checkFile(pin, fetchJson);
    if (update) updates.push(update);
  }
  for (const pin of crates) {
    const update = await checkCrate(pin, fetchJson);
    if (update) updates.push(update);
  }
  return updates;
}

function required(variables: Map<string, string>, name: string): string {
  const value = variables.get(name);
  if (!value) throw new Error(`fetch-native.sh doesn't set ${name}`);
  return value;
}

function short(revision: string): string {
  return revision.slice(0, 7);
}

const fetchJson: FetchJson = async (url, body) => {
  const response = await fetch(url, {
    method: body === undefined ? "GET" : "POST",
    // crates.io refuses requests without a user agent.
    headers: { "User-Agent": "Earshot upstream check (github.com/tapresle/Earshot-Notes)", "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${url} returned ${response.status}`);
  return response.json();
};

async function main() {
  const root = new URL("../../", import.meta.url);
  const read = (path: string) => readFile(new URL(path, root), "utf8");
  const files = [
    ...readNativePins(await read("scripts/fetch-native.sh")),
    ...readModelPins(await read("src-tauri/src/models/mod.rs")),
  ];
  const crates = [readCratePin(await read("src-tauri/Cargo.lock"), "llama-cpp-2")];
  process.stdout.write(renderReport(await checkAll(files, crates, fetchJson)));
}

if (import.meta.main) {
  await main();
}
