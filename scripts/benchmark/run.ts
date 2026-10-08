// Benchmarks summary models (#8): summarizes the synthetic meetings in
// src-tauri/tests/fixtures/meetings/ with each model through the app's own
// summary code (the summary_eval example), at three lengths, and measures each
// run's peak memory. Each run is its own process under `/usr/bin/time -l`.
// Its maximum resident set size counts the model's memory-mapped weights and
// Metal's buffers. The peak memory footprint leaves out the mapped weights, so
// it's recorded but not compared with the budget.
//
//   npm run benchmark -- [--out results.jsonl] [model.gguf ...]
//   npm run benchmark -- --report results.jsonl
//
// Without models, it runs every GGUF in src-tauri/tests/models/ except the
// test model. Results go to docs/benchmarks/YYYYMMDD-summary-models.jsonl by
// default. Each run is appended as one line, with
// the machine's details, so runs from several Macs can share a file. Prints a
// Markdown report at the end, or for an existing file with `--report`.

import { spawnSync } from "node:child_process";
import { appendFileSync, readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The most memory the summary model may use during a call on a 16 GB Mac. */
export const BUDGET_BYTES = 10 * 1024 ** 3;

/** Meeting lengths: the fixtures as they are, and padded to an hour and two. */
export const PAD_MINUTES = [0, 60, 120];

/** What the room for the notes is, as `summarize::run::summary_tokens` sets it. */
export const summaryTokens = (context: number) => Math.min(Math.floor(context / 4), 4096);

export interface Machine {
  chip: string;
  memoryBytes: number;
  macos: string;
}

/** One run: summary_eval's `--json` output, with the machine and peak memory. */
export interface Run {
  model: string;
  fixture: string;
  pad_minutes: number;
  load_seconds: number;
  context_size: number;
  prompt_tokens: number;
  parts: number;
  first_text_seconds: number | null;
  total_seconds: number;
  output_tokens: number;
  writing_tokens_per_second: number | null;
  found: number;
  expected: number;
  machine: Machine;
  /** The mapped weights, Metal's buffers, and everything else resident. */
  peak_resident_bytes: number;
  /** The same without the mapped weights. */
  peak_footprint_bytes: number;
}

/** Reads a figure, such as "peak memory footprint", from `/usr/bin/time -l` output on macOS. */
export function parseTimeFigure(timeOutput: string, label: string): number {
  const line = timeOutput.split("\n").find((l) => l.trim().endsWith(label));
  const value = line ? Number(line.trim().split(/\s+/)[0]) : NaN;
  if (!Number.isFinite(value)) {
    throw new Error(`no ${label} in the time output`);
  }
  return value;
}

/**
 * The context the run created, in tokens. One pass creates a context of the
 * prompt plus the room for the notes. Condensing fills each part close to the
 * model's context size, so that's used as the estimate.
 */
export function contextUsed(run: Run): number {
  return run.parts > 0
    ? run.context_size
    : Math.min(run.prompt_tokens + summaryTokens(run.context_size), run.context_size);
}

/** A straight line through peak memory against the context used. */
export interface MemoryFit {
  /** Memory that doesn't grow with the context: weights and fixed buffers. */
  baseBytes: number;
  /** Memory each token of context adds: mostly the key-value cache. */
  bytesPerToken: number;
}

/** Least-squares fit of peak memory against context used. Needs two or more distinct contexts. */
export function fitMemory(runs: Run[]): MemoryFit | null {
  const points = runs.map((run) => [contextUsed(run), run.peak_resident_bytes] as const);
  const n = points.length;
  const meanX = points.reduce((sum, [x]) => sum + x, 0) / n;
  const meanY = points.reduce((sum, [, y]) => sum + y, 0) / n;
  const sxx = points.reduce((sum, [x]) => sum + (x - meanX) ** 2, 0);
  if (n < 2 || sxx === 0) {
    return null;
  }
  const sxy = points.reduce((sum, [x, y]) => sum + (x - meanX) * (y - meanY), 0);
  const bytesPerToken = sxy / sxx;
  return { baseBytes: meanY - bytesPerToken * meanX, bytesPerToken };
}

const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`;
const seconds = (value: number | null) => (value === null ? "" : `${value.toFixed(1)} s`);

/** Renders the runs as Markdown: one section per machine, one row per model. */
export function renderReport(runs: Run[]): string {
  const out: string[] = [];
  const machines = new Map<string, Run[]>();
  for (const run of runs) {
    const key = `${run.machine.chip}, ${gb(run.machine.memoryBytes)}, macOS ${run.machine.macos}`;
    machines.set(key, [...(machines.get(key) ?? []), run]);
  }
  for (const [machine, machineRuns] of machines) {
    out.push(`## ${machine}`, "");
    out.push(
      "| Model | Context limit | Peak memory (short / 1 h / 2 h) | Per 1K tokens of context | Peak at the limit (estimated) | Within 10 GB | Writing speed | First text (2 h) | Specifics kept |",
      "|---|---|---|---|---|---|---|---|---|",
    );
    const models = new Map<string, Run[]>();
    for (const run of machineRuns) {
      models.set(run.model, [...(models.get(run.model) ?? []), run]);
    }
    for (const [model, modelRuns] of models) {
      const limit = modelRuns[0].context_size;
      const peaks = PAD_MINUTES.map((pad) => {
        const atPad = modelRuns.filter((run) => run.pad_minutes === pad);
        return atPad.length === 0 ? "" : gb(Math.max(...atPad.map((run) => run.peak_resident_bytes)));
      }).join(" / ");
      const fit = fitMemory(modelRuns);
      const measuredMax = Math.max(...modelRuns.map((run) => run.peak_resident_bytes));
      const atLimit = fit ? Math.max(fit.baseBytes + fit.bytesPerToken * limit, measuredMax) : measuredMax;
      const speeds = modelRuns.flatMap((run) => run.writing_tokens_per_second ?? []);
      const speed = speeds.length ? `${(speeds.reduce((a, b) => a + b, 0) / speeds.length).toFixed(0)} tokens/s` : "";
      const longest = modelRuns.filter((run) => run.pad_minutes === Math.max(...PAD_MINUTES));
      const firstText = longest.length
        ? seconds(Math.max(...longest.map((run) => run.first_text_seconds ?? run.total_seconds)))
        : "";
      const found = modelRuns.reduce((sum, run) => sum + run.found, 0);
      const expected = modelRuns.reduce((sum, run) => sum + run.expected, 0);
      out.push(
        `| ${model} | ${limit.toLocaleString("en-US")} | ${peaks} | ${fit ? `${((fit.bytesPerToken * 1024) / 1024 ** 2).toFixed(0)} MB` : ""} | ${gb(atLimit)} | ${atLimit <= BUDGET_BYTES ? "Yes" : "No"} | ${speed} | ${firstText} | ${found} of ${expected} |`,
      );
    }
    out.push("");
  }
  return out.join("\n");
}

function sh(command: string, args: string[]): string {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed: ${result.stderr}`);
  }
  return result.stdout.trim();
}

function machine(): Machine {
  return {
    chip: sh("sysctl", ["-n", "machdep.cpu.brand_string"]),
    memoryBytes: Number(sh("sysctl", ["-n", "hw.memsize"])),
    macos: sh("sw_vers", ["-productVersion"]),
  };
}

function readRuns(path: string): Run[] {
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line) as Run);
}

function main() {
  const root = fileURLToPath(new URL("../../", import.meta.url));
  const tauri = join(root, "src-tauri");
  const args = process.argv.slice(2);
  const option = (name: string) => {
    const at = args.indexOf(name);
    return at === -1 ? undefined : args.splice(at, 2)[1];
  };
  const report = option("--report");
  if (report) {
    process.stdout.write(renderReport(readRuns(report)));
    return;
  }
  const today = new Date().toISOString().slice(0, 10).replaceAll("-", "");
  const out = option("--out") ?? join(root, `docs/benchmarks/${today}-summary-models.jsonl`);
  const modelsDir = join(tauri, "tests/models");
  const models = args.length
    ? args
    : readdirSync(modelsDir)
        .filter((name) => name.endsWith(".gguf") && !name.startsWith("smollm2"))
        .sort()
        .map((name) => join(modelsDir, name));
  const fixturesDir = join(tauri, "tests/fixtures/meetings");
  const fixtures = readdirSync(fixturesDir)
    .filter((name) => name.endsWith(".json"))
    .sort()
    .map((name) => join(fixturesDir, name));

  sh("cargo", ["build", "--release", "--example", "summary_eval", "--manifest-path", join(tauri, "Cargo.toml")]);
  const binary = join(tauri, "target/release/examples/summary_eval");
  const thisMachine = machine();
  for (const model of models) {
    for (const pad of PAD_MINUTES) {
      for (const fixture of fixtures) {
        process.stderr.write(`${basename(model)}, ${basename(fixture)}, padded ${pad} minutes\n`);
        const result = spawnSync(
          "/usr/bin/time",
          ["-l", binary, "--json", "--noisy", "--pad", String(pad), "--model", model, fixture],
          { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
        );
        if (result.status !== 0) {
          throw new Error(`summary_eval failed: ${result.stderr}`);
        }
        const run: Run = {
          ...(JSON.parse(result.stdout.trim()) as Omit<
            Run,
            "machine" | "peak_resident_bytes" | "peak_footprint_bytes"
          >),
          machine: thisMachine,
          peak_resident_bytes: parseTimeFigure(result.stderr, "maximum resident set size"),
          peak_footprint_bytes: parseTimeFigure(result.stderr, "peak memory footprint"),
        };
        appendFileSync(out, `${JSON.stringify(run)}\n`);
      }
    }
  }
  process.stdout.write(renderReport(readRuns(out)));
}

if (import.meta.main) {
  main();
}
