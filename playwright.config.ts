// Renders the preview scenes (src/preview) and compares them with the committed
// screenshots in docs/screenshots. See "Screenshots" in AGENTS.md.
import { defineConfig } from "@playwright/test";

const PORT = 1430;

export default defineConfig({
  testDir: "screenshots",
  // Each scene starts its own page, so they can run side by side.
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  snapshotPathTemplate: "docs/screenshots/{arg}-{projectName}{ext}",
  expect: {
    toHaveScreenshot: {
      // Each pixel may differ slightly in color, which absorbs anti-aliasing,
      // but no pixel may change outright: a pixel budget let a two-letter
      // label change pass.
      threshold: 0.2,
      maxDiffPixels: 0,
    },
  },
  use: {
    baseURL: `http://localhost:${PORT}`,
    browserName: "chromium",
    // The app window's default size, from src-tauri/tauri.conf.json.
    viewport: { width: 1200, height: 780 },
    deviceScaleFactor: 1,
    locale: "en-US",
    timezoneId: "America/Chicago",
  },
  projects: [
    { name: "light", use: { colorScheme: "light" } },
    { name: "dark", use: { colorScheme: "dark" } },
  ],
  webServer: {
    command: `npx vite --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}/preview.html`,
    reuseExistingServer: !process.env.CI,
  },
});
