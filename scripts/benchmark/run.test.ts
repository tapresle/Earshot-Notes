// @vitest-environment node
import { describe, expect, it } from "vitest";
import { contextUsed, fitMemory, parsePeakFootprint, renderReport, type Run } from "./run";

const GB = 1024 ** 3;

const run = (overrides: Partial<Run>): Run => ({
  model: "model-a.gguf",
  fixture: "office-move.json",
  pad_minutes: 0,
  load_seconds: 1,
  context_size: 32_768,
  prompt_tokens: 2000,
  parts: 0,
  first_text_seconds: 1,
  total_seconds: 10,
  output_tokens: 500,
  writing_tokens_per_second: 50,
  found: 12,
  expected: 12,
  machine: { chip: "Apple M5 Max", memoryBytes: 128 * GB, macos: "27.0" },
  peak_footprint_bytes: 4 * GB,
  ...overrides,
});

describe("parsePeakFootprint", () => {
  it("reads the footprint from macOS time output", () => {
    const output = [
      "        6.99 real         1.12 user         0.80 sys",
      "          3532013568  maximum resident set size",
      "          3402891264  peak memory footprint",
    ].join("\n");

    expect(parsePeakFootprint(output)).toBe(3402891264);
  });

  it("fails when the footprint is missing", () => {
    expect(() => parsePeakFootprint("6.99 real")).toThrow(/peak memory footprint/);
  });
});

describe("contextUsed", () => {
  it("is the prompt plus the room for the notes for one pass", () => {
    expect(contextUsed(run({ prompt_tokens: 2000 }))).toBe(2000 + 4096);
    expect(contextUsed(run({ prompt_tokens: 2000, context_size: 8192 }))).toBe(2000 + 2048);
  });

  it("is the context size when the meeting was condensed", () => {
    expect(contextUsed(run({ prompt_tokens: 40_000, parts: 3, context_size: 16_384 }))).toBe(16_384);
  });
});

describe("fitMemory", () => {
  it("separates fixed memory from memory per token", () => {
    const fit = fitMemory([
      run({ prompt_tokens: 2000, peak_footprint_bytes: 3 * GB + 6096 * 100_000 }),
      run({ prompt_tokens: 20_000, peak_footprint_bytes: 3 * GB + 24_096 * 100_000 }),
    ]);

    expect(fit?.bytesPerToken).toBeCloseTo(100_000);
    expect(fit?.baseBytes).toBeCloseTo(3 * GB);
  });

  it("needs two different contexts", () => {
    expect(fitMemory([run({})])).toBeNull();
    expect(fitMemory([run({}), run({ fixture: "billing-incident.json" })])).toBeNull();
  });
});

describe("renderReport", () => {
  it("estimates the peak at the context limit and compares it with the budget", () => {
    // 3 GB fixed, plus 200,000 bytes a token: 3 GB + 32,768 * 200,000 is about 9.1 GB.
    const runs = [
      run({ prompt_tokens: 2000, peak_footprint_bytes: 3 * GB + 6096 * 200_000 }),
      run({ pad_minutes: 120, prompt_tokens: 20_000, peak_footprint_bytes: 3 * GB + 24_096 * 200_000 }),
      run({ model: "model-b.gguf", context_size: 8192, peak_footprint_bytes: 11 * GB }),
    ];

    const report = renderReport(runs);

    expect(report).toContain("## Apple M5 Max, 128.0 GB, macOS 27.0");
    expect(report).toMatch(/\| model-a\.gguf \| 32,768 \| 4\.1 GB \/ {2}\/ 7\.5 GB \| 195 MB \| 9\.1 GB \| Yes \|/);
    expect(report).toMatch(/\| model-b\.gguf \| 8,192 \| 11\.0 GB \/ {2}\/ {2}\| {2}\| 11\.0 GB \| No \|/);
  });
});
