// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  checkAll,
  checkCrate,
  checkFile,
  compareVersions,
  readCratePin,
  readModelPins,
  readNativePins,
  renderReport,
  type FetchJson,
  type FilePin,
  type Reviewed,
} from "./check";

const repoFile = (path: string) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

// Responses recorded from the Hugging Face and crates.io APIs on 2026-10-06,
// trimmed to the fields the check reads.
const NEEDLE_CHANGED = [
  {
    type: "file",
    path: "macos-arm64/libneedle.a",
    lfs: { oid: "98da47c15e1065b4cdc7ddc55e825be78414d4586832db3becaeded39a373df4", size: 1534232 },
    lastCommit: {
      id: "80896a6582ec288abc3f39d0026e931975926c60",
      title: "Replace binaries from production build",
      date: "2026-10-05T18:01:38.000Z",
    },
  },
];
// The repository's `main` (a6adef1) is newer than the commit that last changed this file.
const QWEN_UNCHANGED = [
  {
    type: "file",
    path: "Qwen3-8B-Q4_K_M.gguf",
    lfs: { oid: "120307ba529eb2439d6c430d94104dabd578497bc7bfe7e322b5d9933b449bd4", size: 5027784512 },
    lastCommit: {
      id: "14d6a0c6f786c9914abcc33c4c95c81439574020",
      title: "Add files using upload-large-folder tool",
      date: "2025-05-13T21:11:41.000Z",
    },
  },
];
const crate = (version: string) => ({
  crate: { max_stable_version: version, newest_version: version, repository: "https://github.com/utilityai/llama-cpp-rs" },
});

const NEEDLE: FilePin = {
  name: "needle engine",
  repo: "Cactus-Compute/needle3",
  revision: "c7c415a3d1b3d929014bc6e866d51ebb971f7089",
  path: "macos-arm64/libneedle.a",
  sha256: "a3b9163abe7b4bd52487c4005506cb35c5163bb9b9587af4ae07ed7587de697d",
};
const QWEN: FilePin = {
  name: "Qwen3 8B",
  repo: "unsloth/Qwen3-8B-GGUF",
  revision: "a6adef130ffb23ddaf1a62fec9dced968c9bc482",
  path: "Qwen3-8B-Q4_K_M.gguf",
  sha256: "120307ba529eb2439d6c430d94104dabd578497bc7bfe7e322b5d9933b449bd4",
};
// A model the shipped registry pins, so reading the real file finds it.
const QWEN_3_5_9B: FilePin = {
  name: "Qwen3.5 9B",
  repo: "unsloth/Qwen3.5-9B-GGUF",
  revision: "99a1b2185534379e6e8b5ec869da25d3e7b3f73c",
  path: "Qwen3.5-9B-Q4_K_M.gguf",
  sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
};

/** Answers each URL with a recorded response, and records the requests. */
function recorded(responses: Record<string, unknown>) {
  const requests: { url: string; body?: unknown }[] = [];
  const fetchJson: FetchJson = async (url, body) => {
    requests.push({ url, body });
    if (!(url in responses)) throw new Error(`unexpected request to ${url}`);
    return responses[url];
  };
  return { fetchJson, requests };
}

const paths = (repo: string) => `https://huggingface.co/api/models/${repo}/paths-info/main`;
const CRATE_URL = "https://crates.io/api/v1/crates/llama-cpp-2";

describe("reading pins", () => {
  it("reads the shipped native pins and skips the test model", () => {
    const pins = readNativePins(repoFile("scripts/fetch-native.sh"));

    expect(pins.map((p) => [p.name, p.repo, p.path])).toEqual([
      ["needle engine", "Cactus-Compute/needle3", "macos-arm64/libneedle.a"],
      ["Whistle", "Cactus-Compute/whistle", "whistle.cact"],
    ]);
    expect(pins.every((p) => /^[0-9a-f]{40}$/.test(p.revision) && /^[0-9a-f]{64}$/.test(p.sha256))).toBe(true);
  });

  it("refuses a native pin it can't name, so a new pin isn't silently skipped", () => {
    const script = [
      'OTHER_REPO="someone/other"',
      'OTHER_REV="0000000000000000000000000000000000000000"',
      `  "$OTHER_REPO $OTHER_REV file.bin $VENDOR/file.bin ${"a".repeat(64)}"`,
    ].join("\n");

    expect(() => readNativePins(script)).toThrow(/OTHER/);
  });

  it("reads every summary model in the registry", () => {
    const pins = readModelPins(repoFile("src-tauri/src/models/mod.rs"));

    expect(pins.length).toBeGreaterThanOrEqual(4);
    expect(pins).toContainEqual(QWEN_3_5_9B);
  });

  it("reads the locked llama-cpp-2 version", () => {
    expect(readCratePin(repoFile("src-tauri/Cargo.lock"), "llama-cpp-2").version).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("fails loudly when a file's format changes", () => {
    expect(() => readNativePins("")).toThrow();
    expect(() => readModelPins("")).toThrow();
    expect(() => readCratePin("", "llama-cpp-2")).toThrow();
  });
});

describe("checking Hugging Face files", () => {
  it("reports a pinned file that changed, with the commit that changed it", async () => {
    const { fetchJson, requests } = recorded({ [paths(NEEDLE.repo)]: NEEDLE_CHANGED });

    const update = await checkFile(NEEDLE, fetchJson);

    expect(requests).toEqual([{ url: paths(NEEDLE.repo), body: { paths: [NEEDLE.path], expand: true } }]);
    expect(update?.name).toBe("needle engine");
    expect(update?.upstream).toContain("2026-10-05");
    expect(update?.upstream).toContain(
      "https://huggingface.co/Cactus-Compute/needle3/commit/80896a6582ec288abc3f39d0026e931975926c60",
    );
  });

  it("ignores newer commits that don't touch the pinned file", async () => {
    const { fetchJson } = recorded({ [paths(QWEN.repo)]: QWEN_UNCHANGED });

    expect(await checkFile(QWEN, fetchJson)).toBeNull();
  });

  it("reports a pinned file that's gone from main", async () => {
    const { fetchJson } = recorded({ [paths(QWEN.repo)]: [] });

    expect((await checkFile(QWEN, fetchJson))?.upstream).toMatch(/No longer on `main`/);
  });
});

describe("checking crates", () => {
  it("reports a newer stable release", async () => {
    const { fetchJson } = recorded({ [CRATE_URL]: crate("0.1.160") });

    const update = await checkCrate({ name: "llama-cpp-2", version: "0.1.158" }, fetchJson);

    expect(update?.upstream).toContain("https://crates.io/crates/llama-cpp-2/0.1.160");
    expect(update?.upstream).toContain("https://github.com/utilityai/llama-cpp-rs/releases");
  });

  it("reports nothing when the locked version is current", async () => {
    const { fetchJson } = recorded({ [CRATE_URL]: crate("0.1.158") });

    expect(await checkCrate({ name: "llama-cpp-2", version: "0.1.158" }, fetchJson)).toBeNull();
  });

  it("compares versions numerically", () => {
    expect(compareVersions("0.1.160", "0.1.158")).toBe(1);
    expect(compareVersions("0.1.99", "0.1.158")).toBe(-1);
    expect(compareVersions("0.2.0", "0.2.0")).toBe(0);
  });
});

describe("reviewed versions", () => {
  const declined = (sha256: string, version: string): Reviewed => ({
    files: [{ repo: NEEDLE.repo, path: NEEDLE.path, sha256, reason: "no gain on the fixtures", date: "2026-10-08" }],
    crates: [{ name: "llama-cpp-2", version, reason: "waiting for a model that needs it", date: "2026-10-08" }],
  });

  it("doesn't report a file version that was reviewed and declined", async () => {
    const { fetchJson } = recorded({ [paths(NEEDLE.repo)]: NEEDLE_CHANGED });

    const reviewed = declined("98da47c15e1065b4cdc7ddc55e825be78414d4586832db3becaeded39a373df4", "0.0.0");
    expect(await checkFile(NEEDLE, fetchJson, reviewed)).toBeNull();
  });

  it("reports a file that changed again after the declined version", async () => {
    const { fetchJson } = recorded({ [paths(NEEDLE.repo)]: NEEDLE_CHANGED });

    expect(await checkFile(NEEDLE, fetchJson, declined("0".repeat(64), "0.0.0"))).not.toBeNull();
  });

  it("doesn't report a declined crate version, but reports a newer one", async () => {
    const { fetchJson } = recorded({ [CRATE_URL]: crate("0.1.160") });
    const pin = { name: "llama-cpp-2", version: "0.1.158" };

    expect(await checkCrate(pin, fetchJson, declined("0".repeat(64), "0.1.160"))).toBeNull();
    expect(await checkCrate(pin, fetchJson, declined("0".repeat(64), "0.1.159"))).not.toBeNull();
  });

  it("reads the committed reviewed list", () => {
    const reviewed = JSON.parse(repoFile("scripts/upstream/reviewed.json")) as Reviewed;

    expect(Array.isArray(reviewed.files) && Array.isArray(reviewed.crates)).toBe(true);
  });
});

describe("the report", () => {
  it("is empty when everything is current", async () => {
    const { fetchJson } = recorded({ [paths(QWEN.repo)]: QWEN_UNCHANGED, [CRATE_URL]: crate("0.1.158") });

    const updates = await checkAll([QWEN], [{ name: "llama-cpp-2", version: "0.1.158" }], fetchJson);

    expect(renderReport(updates)).toBe("");
  });

  it("lists each update in one table", async () => {
    const { fetchJson } = recorded({
      [paths(NEEDLE.repo)]: NEEDLE_CHANGED,
      [paths(QWEN.repo)]: QWEN_UNCHANGED,
      [CRATE_URL]: crate("0.1.160"),
    });

    const report = renderReport(
      await checkAll([NEEDLE, QWEN], [{ name: "llama-cpp-2", version: "0.1.158" }], fetchJson),
    );

    const rows = report.split("\n").filter((line) => line.startsWith("| ") && !line.startsWith("| Pin") && !line.startsWith("| ---"));
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatch(/^\| needle engine \|/);
    expect(rows[1]).toMatch(/^\| `llama-cpp-2` \|/);
  });

  it("keeps a commit title from breaking the table", () => {
    const report = renderReport([{ name: "Whistle", pinned: "a", upstream: "Fix a | b\nand c" }]);

    expect(report).toContain("| Whistle | a | Fix a \\| b and c |");
  });
});
