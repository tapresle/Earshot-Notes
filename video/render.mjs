// Drives the app's preview build through the demo and captures every frame.
// The stage (stage.html) draws everything around the app: titles, camera,
// cursor, and the intro, comparison, and outro scenes. This file decides what
// happens inside the app, and when.
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { DURATION, FPS } from "./timing.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..");
const PORT = 1431; // The screenshot tests use 1430, so both can run at once.

const BEAT = 60 / 128, T0 = 0.438;
const bar = (k) => T0 + k * 4 * BEAT;
const MEETING = 18; // The id the fake backend gives the first new meeting in the "meeting" scene.
const REC_START = 16.1; // Recording starts with the new meeting.

/** Starts Vite on the checkout, which serves the app's preview build. */
async function startVite() {
  const proc = spawn("npx", ["vite", "--port", String(PORT), "--strictPort"], { cwd: REPO, stdio: "ignore" });
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(`http://localhost:${PORT}/preview.html`)).ok) return proc;
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  proc.kill();
  throw new Error(`Vite didn't start on port ${PORT}. Is another server using it?`);
}

/**
 * Renders the video to `out` (an .mp4 with no audio), or, given `stillTimes`,
 * writes a PNG for each of those seconds into the folder `out`.
 * Returns the sound cues: clicks, typing, and chimes, in seconds.
 */
export async function render(out, stillTimes = null) {
  const mode = stillTimes ? "stills" : "video";
  const lastFrame = stillTimes ? Math.round(Math.max(...stillTimes) * FPS) : Math.round(DURATION * FPS);
  const vite = await startVite();
  const browser = await chromium.launch();
  const sfx = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, colorScheme: "dark", deviceScaleFactor: 1 });
    page.on("console", (m) => m.type() === "error" && console.log("console:", m.text()));
    page.on("pageerror", (e) => console.log("pageerror:", e.message));
    await page.clock.setFixedTime(new Date("2026-10-06T15:30:00-05:00"));
    await page.route(`http://localhost:${PORT}/__stage.html`, (route) =>
      route.fulfill({ contentType: "text/html", body: readFileSync(join(HERE, "stage.html"), "utf8") }),
    );
    await page.goto(`http://localhost:${PORT}/__stage.html`);
    const app = page.frames().find((f) => f.url().includes("preview.html"));
    await app.getByRole("button", { name: "New meeting" }).waitFor();
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(500);
    // The two Recording settings: record new meetings, and summarize on stop.
    await app.evaluate(async () => {
      await window.__TAURI_INTERNALS__.invoke("set_record_new_meetings", { enabled: true });
      await window.__TAURI_INTERNALS__.invoke("set_summarize_on_stop", { enabled: true });
    });

    // ---------- helpers ----------
    const emit = (event, payload) =>
      app.evaluate(([e, p]) => window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event: e, payload: p }), [event, payload]);
    const invoke = (cmd, args) => app.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
    const rectOf = (loc) => loc.evaluate((el) => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
    const stage = (fn, arg) => page.evaluate(fn, arg);
    let cursor = { x: 640, y: 520 };
    const actions = [];
    const streams = [];
    const at = (t, run) => actions.push({ t, run });
    const during = (t0, t1, tick) => streams.push({ t0, t1, tick, last: -1 });

    /** Moves the pointer to a spot on an element, arriving at t + d. */
    function move(t, d, locFn, dx = 0.5, dy = 0.5) {
      at(t, async () => {
        const r = await rectOf(locFn());
        const to = { x: r.x + r.w * dx, y: r.y + r.h * dy };
        await stage(([a, b]) => window.stage.cursor.push(a, b), [{ t, ...cursor }, { t: t + d, ...to }]);
        cursor = to;
      });
    }
    /** Clicks where the pointer is; `act` performs the click in the app. */
    function click(t, locFn, act = (l) => l.evaluate((el) => el.click()), sound = "click") {
      at(t, async () => {
        await stage((k) => window.stage.clicks.push(k), { t, ...cursor });
        sfx.push({ t, name: sound });
        await act(locFn());
      });
    }
    /** Types `text` into a field between t0 and t1, through React's value setter. */
    function type(t0, t1, locFn, text, prefix = "") {
      sfx.push({ t: t0, t1, name: "type" });
      during(t0, t1, async (p) => {
        const n = Math.round(text.length * p);
        await locFn().evaluate((el, value) => {
          const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
          Object.getOwnPropertyDescriptor(proto, "value").set.call(el, value);
          el.dispatchEvent(new Event("input", { bubbles: true }));
          el.setSelectionRange(value.length, value.length);
        }, prefix + text.slice(0, n));
      });
    }
    /** Streams `text` into an event between t0 and t1, a few characters per frame. */
    function stream(t0, t1, event, text) {
      const tokens = text.match(/\s*(?:#+ |- )?\S+/g);
      let sent = 0;
      during(t0, t1, async (p) => {
        const n = Math.round(tokens.length * p);
        if (n > sent) { await emit(event, { meetingId: MEETING, text: tokens.slice(sent, n).join("") }); sent = n; }
      });
    }
    const focusEl = (l) => l.evaluate((el) => { window.focus(); el.focus(); });

    // ---------- locators ----------
    const btn = (name, exact = true) => () => app.getByRole("button", { name, exact });
    const notes = () => app.getByPlaceholder("Write notes during the meeting. Markdown works.");
    const title = () => app.getByLabel("Meeting title");
    const tab = (name) => () => app.getByRole("tab", { name });
    const chatBox = () => app.locator(".chat textarea");

    // ---------- camera ----------
    const full = { S: 1.06, X: (1920 - 1200 * 1.06) / 2, Y: 196 };
    const cams = [
      [bar(6), full], [18.4, full],
      [19.6, { S: 1.55, X: 20, Y: 190 }], [24.3, { S: 1.55, X: 20, Y: 190 }],
      [25.2, { S: 1.75, X: -380, Y: 190 }], [29.6, { S: 1.75, X: -380, Y: 190 }],
      [30.3, full], [32.4, full],
      [33.3, { S: 1.6, X: -30, Y: 190 }], [37.6, { S: 1.6, X: -30, Y: 190 }],
      [38.3, full], [38.9, full],
      [39.7, { S: 1.5, X: 100, Y: 190 }], [45.2, { S: 1.5, X: 100, Y: 190 }],
      [45.9, full], [46.4, full],
      [47.2, { S: 1.4, X: 288, Y: 190 }],
    ];
    await stage((list) => list.forEach(([t, c]) => window.stage.key(t, c)), cams);

    // ---------- 1. new meeting and recording ----------
    at(bar(6) + 1.2, () => stage((k) => window.stage.cursor.push(k), { t: bar(6) + 1.2, x: 640, y: 520 }));
    move(15.45, 0.55, btn("New meeting"));
    click(16.05, btn("New meeting"));
    move(16.45, 0.45, title, 0.25, 0.5);
    click(16.95, title, (l) => l.evaluate((el) => { window.focus(); el.focus(); el.select(); }));
    type(17.05, 17.55, title, "Customer call");
    // Saves the title, so the meeting list shows it.
    at(17.7, () => title().evaluate((el) => el.blur()));
    move(17.75, 0.6, btn("Stop recording"), 0.5, 3);

    const LINES = [
      ["them", 18.5, 19.5, "Thanks for making time today."],
      ["me", 19.8, 20.8, "Of course. Where should we start?"],
      ["them", 21.1, 22.7, "The import flow. We have three years of call transcripts."],
      ["me", 23.0, 23.6, "Audio files, or text?"],
      ["them", 23.9, 25.6, "Markdown, mostly. Speaker labels need to survive the import."],
      ["me", 25.9, 27.3, "Got it. I can have a prototype ready by Friday."],
      ["them", 27.6, 28.8, "Perfect. Ted can test it the week after."],
    ];
    const segIds = [];
    const talk = [];
    LINES.forEach(([source, t0, t1, text], i) => {
      const id = 500 + i;
      const startMs = Math.round((t0 - REC_START) * 1000);
      segIds.push({ id, startMs });
      talk.push({ src: source, t0: t0 - 0.2, t1 });
      const words = text.split(" ");
      let shown = 0;
      during(t0, t1, async (p) => {
        const n = Math.max(1, Math.min(words.length - 1, Math.floor(words.length * p)));
        if (n !== shown) { shown = n; await emit("transcript:preview", { meetingId: MEETING, source, startMs, text: words.slice(0, n).join(" ") }); }
      });
      at(t1 + 0.1, async () => {
        await emit("transcript:preview", { meetingId: MEETING, source, startMs, text: "" });
        await emit("transcript:segment", { id, meetingId: MEETING, source, startMs, endMs: startMs + 1500, text, language: "en" });
      });
    });
    at(17.5, () => stage((t) => { window.stage.talk = t; }, talk));

    // ---------- 2. notes ----------
    move(24.3, 0.45, notes, 0.08, 0.04);
    click(24.8, notes, focusEl);
    type(24.9, 29.6, notes, "- Three years of Markdown transcripts\n- Keep the speaker labels\n- Prototype by Friday");

    // ---------- 3. summary ----------
    move(30.0, 0.5, btn("Stop recording"));
    click(30.6, btn("Stop recording"));
    // Summarize on stop: the summary starts on its own, and the view switches to it.
    at(31.0, () => emit("summary:progress", { meetingId: MEETING, stage: "loading" }));
    move(31.1, 0.6, tab("Summary"), 0.5, 4);
    at(31.9, () => emit("summary:progress", { meetingId: MEETING, stage: "reading", part: 1, parts: 1 }));
    at(32.6, () => emit("summary:progress", { meetingId: MEETING, stage: "writing" }));
    const SUMMARY = `## Import flow

  - The customer has three years of call transcripts, mostly in Markdown.
  - Speaker labels must survive the import.

  ## Testing

  - Ted tests the prototype the week after it's ready.

  ## Next steps

  - Build an import prototype by Friday (You)
  - Test the prototype the week after (Ted)`;
    stream(32.7, 36.3, "summary:text", SUMMARY);
    at(36.45, async () => {
      sfx.push({ t: 36.45, name: "chime" });
      await emit("summary:done", {
        meetingId: MEETING,
        title: "Transcript import kickoff",
        summary: { id: 90, meetingId: MEETING, uptoMs: 12_000, isFinal: true, bodyMarkdown: SUMMARY, modelId: "qwen3.5-4b-q4_k_m", createdAt: Date.now() },
      });
      // The meeting renames itself from its summary: point at it in the list.
      const item = app.getByRole("navigation", { name: "Meetings" }).getByRole("button", { name: /Transcript import kickoff/ });
      await item.waitFor({ timeout: 2000 }).catch(() => {});
      const r = await rectOf(item).catch(() => null);
      if (r) await stage((b) => window.stage.focusBox.push(b), { t0: 36.7, t1: 37.8, ...r });
    });

    // ---------- 4. chat ----------
    move(37.95, 0.45, tab("Chat"));
    click(38.5, tab("Chat"));
    move(38.75, 0.45, chatBox, 0.2, 0.5);
    click(39.25, chatBox, focusEl);
    const QUESTION = "When is the prototype due?";
    type(39.3, 40.2, chatBox, QUESTION);
    at(40.45, async () => {
      sfx.push({ t: 40.45, name: "click" });
      await chatBox().evaluate((el) => el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    });
    at(40.5, () => emit("chat:progress", { meetingId: MEETING, stage: "loading" }));
    at(40.95, () => emit("chat:progress", { meetingId: MEETING, stage: "writing" }));
    const ANSWER = "Friday. You offered to have a prototype ready by Friday [S1], and Ted will test it the week after [S2].";
    stream(41.0, 42.5, "chat:text", ANSWER);
    at(42.65, async () => {
      sfx.push({ t: 42.65, name: "chime" });
      const base = { meetingId: MEETING, createdAt: Date.now() };
      await emit("chat:done", {
        meetingId: MEETING,
        question: { ...base, id: 900, role: "user", bodyMarkdown: QUESTION, sources: [], modelId: null },
        answer: {
          ...base, id: 901, role: "assistant", bodyMarkdown: ANSWER, modelId: "qwen3.5-4b-q4_k_m",
          sources: [
            { kind: "transcript", label: "S1", segmentId: segIds[5].id, startMs: segIds[5].startMs },
            { kind: "transcript", label: "S2", segmentId: segIds[6].id, startMs: segIds[6].startMs },
          ],
        },
      });
    });
    const source = () => app.getByRole("button", { name: /^Show .* in the transcript$/ }).first();
    move(43.2, 0.5, source);
    click(43.8, source);

    // ---------- 5. settings ----------
    at(45.0, () => invoke("set_vocabulary", { terms: [{ term: "Ted", heardAs: ["Tad", "Todd"] }] }));
    move(45.5, 0.5, () => app.getByRole("navigation", { name: "Meetings" }).getByRole("button", { name: "Settings" }));
    click(46.05, () => app.getByRole("navigation", { name: "Meetings" }).getByRole("button", { name: "Settings" }));
    // Scroll to the transcription models, then to the local AI model.
    const headingTop = (id) => app.locator(".workspace").evaluate((el, hid) => {
      const h = el.querySelector(`#${hid}`);
      return Math.min(el.scrollHeight - el.clientHeight, h.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop - 24);
    }, id);
    const ease = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
    const scrollStep = (t0, t1, from, to) => {
      let a = null, b = null;
      during(t0, t1, async (p) => {
        a ??= from === null ? 0 : await headingTop(from);
        b ??= await headingTop(to);
        await app.locator(".workspace").evaluate((el, y) => { el.scrollTop = y; }, a + (b - a) * ease(p));
      });
    };
    scrollStep(47.6, 48.8, null, "transcription-heading");
    scrollStep(49.9, 51.3, "transcription-heading", "local-model-heading");

    // ---------- 6. summary instructions ----------
    scrollStep(52.9, 54.1, "local-model-heading", "summaries-heading");
    move(54.3, 0.45, btn("Customize"));
    click(54.8, btn("Customize"));
    const editor = () => app.getByLabel("Your summary instructions");
    let base = null;
    at(55.0, async () => { base = await editor().inputValue(); });
    move(55.1, 0.45, editor, 0.3, 0.62);
    click(55.6, editor, focusEl);
    const ADDED = "\n- Start with a one-line recap I can forward to my manager.";
    sfx.push({ t: 55.7, t1: 57.6, name: "type" });
    during(55.7, 57.6, async (p) => {
      const value = base + ADDED.slice(0, Math.round(ADDED.length * p));
      await editor().evaluate((el, v) => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set.call(el, v);
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.setSelectionRange(v.length, v.length);
      }, value);
    });
    move(57.8, 0.45, btn("Save"));
    click(58.3, btn("Save"));

    // ---------- run ----------
    actions.sort((a, b) => a.t - b.t);
    let ffmpeg = null;
    if (mode === "video") {
      ffmpeg = spawn("ffmpeg", ["-y", "-v", "error", "-f", "image2pipe", "-framerate", String(FPS), "-c:v", "png", "-i", "-",
        "-c:v", "libx264", "-preset", "slow", "-crf", "14", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out], { stdio: ["pipe", "inherit", "inherit"] });
    }
    const cdp = await page.context().newCDPSession(page);
    let ai = 0;
    const started = Date.now();
    for (let f = 0; f <= lastFrame; f++) {
      const t = f / FPS;
      while (ai < actions.length && actions[ai].t <= t) await actions[ai++].run();
      for (const s of streams) {
        if (t >= s.t0 && s.last < 1) {
          const p = Math.min(1, (t - s.t0) / (s.t1 - s.t0));
          if (p !== s.last) { await s.tick(p); s.last = p; }
        }
      }
      const want = mode === "video" || stillTimes.some((st) => Math.round(st * FPS) === f);
      if (!want) continue;
      await page.evaluate((tt) => window.stage.render(tt), t);
      await app.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
      const buf = Buffer.from(data, "base64");
      if (ffmpeg) {
        if (!ffmpeg.stdin.write(buf)) await new Promise((r) => ffmpeg.stdin.once("drain", r));
        if (f % 300 === 0) console.log(`  frame ${f} of ${lastFrame} (${((Date.now() - started) / 1000).toFixed(0)} s)`);
      } else {
        writeFileSync(join(out, `t${t.toFixed(2)}.png`), buf);
      }
    }
    if (ffmpeg) {
      ffmpeg.stdin.end();
      const code = await new Promise((r) => ffmpeg.on("close", r));
      if (code !== 0) throw new Error(`ffmpeg failed to encode the frames (exit code ${code}).`);
    }
    return sfx.sort((a, b) => a.t - b.t);
  } finally {
    await browser.close();
    vite.kill();
  }
}
