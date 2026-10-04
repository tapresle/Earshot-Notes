import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { MeetingDetail } from "./lib/api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

type Args = Record<string, unknown>;

/** An in-memory stand-in for the Rust commands. */
function fakeBackend(seed: MeetingDetail[] = []) {
  const meetings = new Map(seed.map((m) => [m.id, { ...m }]));
  let nextId = Math.max(0, ...seed.map((m) => m.id)) + 1;
  const notFound = { kind: "notFound", message: "missing" };

  const handlers: Record<string, (args: Args) => unknown> = {
    list_meetings: () =>
      [...meetings.values()]
        .sort((a, b) => b.createdAt - a.createdAt)
        .map(({ id, title, createdAt, startedAt, endedAt }) => ({ id, title, createdAt, startedAt, endedAt })),
    create_meeting: () => {
      const meeting: MeetingDetail = {
        id: nextId++,
        title: "Untitled meeting",
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
        notesMarkdown: "",
        notesUpdatedAt: Date.now(),
      };
      meetings.set(meeting.id, meeting);
      return meeting;
    },
    get_meeting: ({ id }) => {
      const meeting = meetings.get(id as number);
      if (!meeting) throw notFound;
      return meeting;
    },
    rename_meeting: ({ id, title }) => {
      const meeting = meetings.get(id as number);
      if (!meeting) throw notFound;
      meeting.title = (title as string).trim() || "Untitled meeting";
      return meeting.title;
    },
    update_notes: ({ id, bodyMarkdown }) => {
      const meeting = meetings.get(id as number);
      if (!meeting) throw notFound;
      meeting.notesMarkdown = bodyMarkdown as string;
      return Date.now();
    },
    delete_meeting: ({ id }) => {
      if (!meetings.delete(id as number)) throw notFound;
    },
  };

  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => handlers[command]((args ?? {}) as Args));
  return meetings;
}

function seedMeeting(id: number, title: string, notesMarkdown = ""): MeetingDetail {
  const createdAt = Date.now() - id * 1000;
  return { id, title, createdAt, startedAt: null, endedAt: null, notesMarkdown, notesUpdatedAt: createdAt };
}

function sidebar() {
  return screen.getByRole("navigation", { name: "Meetings" });
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("App", () => {
  it("shows the first-meeting empty state when there are no meetings", async () => {
    fakeBackend();
    render(<App />);

    expect(await screen.findByRole("heading", { name: "Start your first meeting" })).toBeInTheDocument();
    expect(within(sidebar()).getByText("Your meetings appear here.")).toBeInTheDocument();
  });

  it("creates a meeting and opens it", async () => {
    fakeBackend();
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "New meeting" }));

    expect(await screen.findByLabelText("Meeting title")).toHaveValue("Untitled meeting");
    expect(within(sidebar()).getByRole("button", { name: /Untitled meeting/, current: "page" })).toBeInTheDocument();
  });

  it("opens a meeting with its saved notes", async () => {
    fakeBackend([seedMeeting(1, "Design sync", "- ship dark mode")]);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

    expect(await screen.findByLabelText("Meeting notes")).toHaveValue("- ship dark mode");
  });

  it("autosaves notes as you type", async () => {
    const meetings = fakeBackend([seedMeeting(1, "Standup")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Standup/ }));

    await user.type(await screen.findByLabelText("Meeting notes"), "Blocked on review");

    await waitFor(() => expect(meetings.get(1)?.notesMarkdown).toBe("Blocked on review"));
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
  });

  it("saves pending notes when you switch meetings", async () => {
    const meetings = fakeBackend([seedMeeting(1, "First"), seedMeeting(2, "Second")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /First/ }));
    await user.type(await screen.findByLabelText("Meeting notes"), "quick note");

    await user.click(within(sidebar()).getByRole("button", { name: /Second/ }));

    await waitFor(() => expect(meetings.get(1)?.notesMarkdown).toBe("quick note"));
  });

  it("renames a meeting and updates the sidebar", async () => {
    fakeBackend([seedMeeting(1, "Untitled meeting")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Untitled meeting/ }));

    const title = await screen.findByLabelText("Meeting title");
    await user.clear(title);
    await user.type(title, "Quarterly planning{Enter}");

    expect(await within(sidebar()).findByRole("button", { name: /Quarterly planning/ })).toBeInTheDocument();
  });

  it("deletes a meeting only after confirmation", async () => {
    const meetings = fakeBackend([seedMeeting(1, "Old sync")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Old sync/ }));

    await user.click(await screen.findByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(meetings.has(1)).toBe(true);

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete meeting" }));

    await waitFor(() => expect(meetings.has(1)).toBe(false));
    expect(within(sidebar()).queryByRole("button", { name: /Old sync/ })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Start your first meeting" })).toBeInTheDocument();
  });

  it("shows an error with a retry when meetings fail to load", async () => {
    fakeBackend([seedMeeting(1, "Recovered")]);
    vi.mocked(invoke).mockRejectedValueOnce({ kind: "internal", message: "disk I/O error" });
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("The app couldn't read or write your meetings.");
    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(await within(sidebar()).findByRole("button", { name: /Recovered/ })).toBeInTheDocument();
  });

  it("explains when an opened meeting no longer exists", async () => {
    const meetings = fakeBackend([seedMeeting(1, "Vanishing")]);
    const user = userEvent.setup();
    render(<App />);
    const item = await within(sidebar()).findByRole("button", { name: /Vanishing/ });
    meetings.delete(1);

    await user.click(item);

    expect(await screen.findByRole("alert")).toHaveTextContent("That meeting no longer exists.");
  });
});
