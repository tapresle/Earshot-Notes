// One screenshot per scene in src/preview/scenes.ts, in light and dark mode.
// `npm run screenshots` updates the images that changed; `npm run screenshots:check`
// fails when the UI no longer matches them.
import { expect, test, type Page } from "@playwright/test";

/** The moment every scene is rendered at, in the configured time zone. */
const NOW = new Date("2026-10-06T15:30:00-05:00");

async function open(page: Page, scene: string) {
  await page.clock.setFixedTime(NOW);
  await page.goto(`/preview.html?scene=${scene}`);
}

function sidebar(page: Page) {
  return page.getByRole("navigation", { name: "Meetings" });
}

async function openMeeting(page: Page, title: string) {
  await sidebar(page).getByRole("button", { name: new RegExp(title) }).click();
  await expect(page.getByLabel("Meeting title")).toHaveValue(title);
}

test("empty", async ({ page }) => {
  await open(page, "empty");
  await expect(page.getByRole("heading", { name: "Start your first meeting" })).toBeVisible();
  await expect(page).toHaveScreenshot("empty.png");
});

test("setup", async ({ page }) => {
  await open(page, "setup");
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page).toHaveScreenshot("setup.png");
});

test("meeting", async ({ page }) => {
  await open(page, "meeting");
  await openMeeting(page, "Design sync");
  await expect(page.getByText("Thursday works.", { exact: false })).toBeVisible();
  await expect(page).toHaveScreenshot("meeting.png");
});

test("meeting menu", async ({ page }) => {
  await open(page, "meeting");
  await openMeeting(page, "Design sync");
  const meeting = sidebar(page).getByRole("button", { name: /^Launch planning,/ });
  const box = (await meeting.boundingBox())!;
  await page.mouse.click(box.x + 60, box.y + box.height / 2, { button: "right" });
  await expect(page.getByRole("menuitem", { name: "Export" })).toBeFocused();
  await expect(page).toHaveScreenshot("meeting-menu.png");
});

test("the meeting menu highlights an item only on hover or from the keyboard", async ({ page }) => {
  await open(page, "meeting");
  await openMeeting(page, "Design sync");
  const meeting = sidebar(page).getByRole("button", { name: /^Launch planning,/ });
  const exportItem = page.getByRole("menuitem", { name: "Export" });
  const background = () => exportItem.evaluate((item) => getComputedStyle(item).backgroundColor);
  const box = (await meeting.boundingBox())!;

  await page.mouse.click(box.x + 60, box.y + box.height / 2, { button: "right" });
  await expect(exportItem).toBeFocused();
  const plain = await background();
  expect(plain).toBe("rgba(0, 0, 0, 0)");

  await exportItem.hover();
  expect(await background()).not.toBe(plain);

  await page.keyboard.press("Escape");
  await page.mouse.move(0, 0);
  await meeting.focus();
  await page.keyboard.press("Shift+F10");
  await expect(exportItem).toBeFocused();
  expect(await background()).not.toBe(plain);
});

test("delete dialog", async ({ page }) => {
  await open(page, "meeting");
  await openMeeting(page, "Design sync");
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Delete “Design sync”?" })).toBeVisible();
  await expect(page).toHaveScreenshot("delete-dialog.png");
});

test("summary", async ({ page }) => {
  await open(page, "summary");
  await openMeeting(page, "Design sync");
  await page.getByRole("tab", { name: "Summary" }).click();
  await expect(page.getByRole("heading", { name: "Decisions" })).toBeVisible();
  await expect(page).toHaveScreenshot("summary.png");
});

test("chat", async ({ page }) => {
  await open(page, "chat");
  await openMeeting(page, "Design sync");
  await page.getByRole("tab", { name: "Chat" }).click();
  await expect(page.getByRole("button", { name: "Show 0:37 in the transcript" })).toBeVisible();
  await expect(page).toHaveScreenshot("chat.png");
});

test("recording", async ({ page }) => {
  await open(page, "recording");
  await openMeeting(page, "Customer call");
  await expect(page.getByText("Transcripts, mostly in Markdown.")).toBeVisible();
  await expect(page).toHaveScreenshot("recording.png");
});

test("settings", async ({ page }) => {
  await open(page, "settings");
  await sidebar(page).getByRole("button", { name: "Settings" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
  await expect(page.getByLabel("Record from")).toBeVisible();
  await expect(page).toHaveScreenshot("settings.png");
});

test("settings scroll inside their pane, not the window", async ({ page }) => {
  await open(page, "settings");
  await sidebar(page).getByRole("button", { name: "Settings" }).click();
  await expect(page.getByLabel("Record from")).toBeVisible();

  await page.locator(".workspace").evaluate((pane) => pane.scrollTo(0, pane.scrollHeight));

  const size = await page.evaluate(() => ({
    height: document.scrollingElement!.scrollHeight,
    window: window.innerHeight,
  }));
  expect(size.height).toBeLessThanOrEqual(size.window);
});

test("settings fill the window beside the meeting list as it resizes", async ({ page }) => {
  await open(page, "settings");
  await sidebar(page).getByRole("button", { name: "Settings" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Transcript" })).toHaveCount(0);

  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 780 });
    const sidebarBox = await page.locator(".sidebar").boundingBox();
    const workspace = await page.locator(".workspace").boundingBox();
    expect(workspace!.x).toBe(sidebarBox!.x + sidebarBox!.width);
    expect(workspace!.x + workspace!.width).toBe(width);
  }
});

test("the meeting list scrolls while New meeting, search, and Settings stay put", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 420 });
  await open(page, "meeting");
  const newMeeting = page.getByRole("button", { name: "New meeting" });
  const settings = sidebar(page).getByRole("button", { name: "Settings" });
  const list = page.locator(".sidebar-list");
  await expect(newMeeting).toBeVisible();
  const before = { top: await newMeeting.boundingBox(), bottom: await settings.boundingBox() };

  const scrolled = await list.evaluate((pane) => {
    pane.scrollTo(0, pane.scrollHeight);
    return pane.scrollTop;
  });

  expect(scrolled, "the list should overflow at this height").toBeGreaterThan(0);
  expect(await newMeeting.boundingBox()).toEqual(before.top);
  expect(await settings.boundingBox()).toEqual(before.bottom);
  await expect(page.getByRole("searchbox")).toBeInViewport();
  await expect(settings).toBeInViewport();
});
