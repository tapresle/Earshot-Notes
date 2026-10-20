// Reports newer versions of what Earshot pins: the needle engine and Whistle
// (scripts/fetch-native.sh), the summary models (src-tauri/src/models/mod.rs),
// whisper.cpp (scripts/build-whisper.sh), and llama-cpp-2 (src-tauri/Cargo.lock).
// Prints a Markdown report, or nothing when everything is current. Run weekly
// by .github/workflows/upstream.yml:
//
//   node scripts/upstream/check.ts
//
// Pins are read from the files that define them, so this script never needs
// editing when a pin moves. A Hugging Face pin counts as outdated only when
// its pinned file changed on `main`; commits that touch only a README don't.
// Upstream versions already reviewed and declined are listed in
// scripts/upstream/reviewed.json and aren't reported again.

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

/** A GitHub repository pinned to a release tag and its commit. */
export interface ReleasePin {
  name: string;
  /** `owner/name` on GitHub. */
  repo: string;
  tag: string;
  commit: string;
}

/** One row of the report. */
export interface Update {
  name: string;
  pinned: string;
  upstream: string;
}

/** Upstream versions reviewed and declined, from reviewed.json. */
export interface Reviewed {
  /** Each with the upstream file's SHA-256. */
  files: { repo: string; path: string; sha256: string; reason: string; date: string }[];
  crates: { name: string; version: string; reason: string; date: string }[];
  releases: { repo: string; tag: string; reason: string; date: string }[];
}

export const NOTHING_REVIEWED: Reviewed = { files: [], crates: [], releases: [] };

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

export function readReleasePin(script: string): ReleasePin {
  const variable = (name: string) => {
    const match = script.match(new RegExp(`^${name}="([^"$]*)"`, "m"));
    if (!match) throw new Error(`build-whisper.sh doesn't set ${name}; has its format changed?`);
    return match[1];
  };
  const url = variable("WHISPER_REPO");
  const repo = url.match(/^https:\/\/github\.com\/([\w.-]+\/[\w.-]+?)(?:\.git)?$/)?.[1];
  if (!repo) throw new Error(`build-whisper.sh's WHISPER_REPO isn't a GitHub repository: ${url}`);
  const tag = variable("WHISPER_TAG");
  if (!/^v\d+(\.\d+)*$/.test(tag)) throw new Error(`build-whisper.sh's WHISPER_TAG isn't a version tag: ${tag}`);
  const commit = variable("WHISPER_COMMIT");
  if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error(`build-whisper.sh's WHISPER_COMMIT isn't a full commit: ${commit}`);
  return { name: "whisper.cpp", repo, tag, commit };
}

interface PathInfo {
  path: string;
  lfs?: { oid: string };
  lastCommit?: { id: string; title: string; date: string };
}

/** Reports a pinned file whose content on `main` differs from the pinned checksum. */
export async function checkFile(
  pin: FilePin,
  fetchJson: FetchJson,
  reviewed: Reviewed = NOTHING_REVIEWED,
): Promise<Update | null> {
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
  const declined = reviewed.files.some(
    (r) => r.repo === pin.repo && r.path === pin.path && r.sha256 === info.lfs?.oid,
  );
  if (declined) return null;
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
export async function checkCrate(
  pin: CratePin,
  fetchJson: FetchJson,
  reviewed: Reviewed = NOTHING_REVIEWED,
): Promise<Update | null> {
  const { crate } = (await fetchJson(`https://crates.io/api/v1/crates/${pin.name}`)) as CrateInfo;
  if (compareVersions(crate.max_stable_version, pin.version) <= 0) return null;
  if (reviewed.crates.some((r) => r.name === pin.name && r.version === crate.max_stable_version)) return null;
  const releases = crate.repository ? `, see [the releases](${crate.repository}/releases)` : "";
  return {
    name: `\`${pin.name}\``,
    pinned: `\`${pin.version}\``,
    upstream: `[\`${crate.max_stable_version}\`](https://crates.io/crates/${pin.name}/${crate.max_stable_version})${releases}`,
  };
}

interface Release {
  tag_name: string;
  html_url: string;
  published_at: string;
}

/** Reports a repository whose latest release is newer than the pinned tag. GitHub never picks a prerelease as latest. */
export async function checkRelease(
  pin: ReleasePin,
  fetchJson: FetchJson,
  reviewed: Reviewed = NOTHING_REVIEWED,
): Promise<Update | null> {
  const release = (await fetchJson(`https://api.github.com/repos/${pin.repo}/releases/latest`)) as Release;
  const version = (tag: string) => tag.replace(/^v/, "");
  if (compareVersions(version(release.tag_name), version(pin.tag)) <= 0) return null;
  if (reviewed.releases.some((r) => r.repo === pin.repo && r.tag === release.tag_name)) return null;
  return {
    name: pin.name,
    pinned: `[\`${pin.tag}\`](https://github.com/${pin.repo}/tree/${pin.commit}) (\`${short(pin.commit)}\`)`,
    upstream: `[\`${release.tag_name}\`](${release.html_url}), released on ${release.published_at.slice(0, 10)}`,
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
export async function checkAll(
  files: FilePin[],
  releases: ReleasePin[],
  crates: CratePin[],
  fetchJson: FetchJson,
  reviewed: Reviewed = NOTHING_REVIEWED,
): Promise<Update[]> {
  const updates: Update[] = [];
  for (const pin of files) {
    const update = await checkFile(pin, fetchJson, reviewed);
    if (update) updates.push(update);
  }
  for (const pin of releases) {
    const update = await checkRelease(pin, fetchJson, reviewed);
    if (update) updates.push(update);
  }
  for (const pin of crates) {
    const update = await checkCrate(pin, fetchJson, reviewed);
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
    // crates.io and GitHub refuse requests without a user agent.
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
  const releases = [readReleasePin(await read("scripts/build-whisper.sh"))];
  const crates = [readCratePin(await read("src-tauri/Cargo.lock"), "llama-cpp-2")];
  const reviewed = JSON.parse(await read("scripts/upstream/reviewed.json")) as Reviewed;
  process.stdout.write(renderReport(await checkAll(files, releases, crates, fetchJson, reviewed)));
}

if (import.meta.main) {
  await main();
}
