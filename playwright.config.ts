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
      // Macs on different macOS versions draw text edges slightly differently
      // (measured: up to 9 pixels per image between macOS 27 and CI's macOS 26
      // at this threshold; CI now checks on macOS 27). A two-letter label
      // change differs by 18 or more, and layout or clearly visible color
      // changes by thousands. Changes smaller than that, such as a subtle shade
      // of a color, can pass.
      threshold: 0.3,
      maxDiffPixels: 12,
      // Native select arrows differ between macOS builds by about 34 pixels
      // each, so the screenshots draw selects without them.
      stylePath: "screenshots/screenshot.css",
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
