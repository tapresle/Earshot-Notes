import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { MeetingDetail, TranscriptSegment } from "./lib/api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

type Args = Record<string, unknown>;

type Listener = (event: { payload: unknown }) => void;
const listeners = new Map<string, Set<Listener>>();

/** Delivers a backend event to the UI, like the Rust side's `emit`. */
function emit(event: string, payload: unknown) {
  act(() => listeners.get(event)?.forEach((listener) => listener({ payload })));
}

/** An in-memory stand-in for the Rust commands. */
function fakeBackend(seed: MeetingDetail[] = [], segments: TranscriptSegment[] = []) {
  const meetings = new Map(seed.map((m) => [m.id, { ...m }]));
  let nextId = Math.max(0, ...seed.map((m) => m.id)) + 1;
  const notFound = { kind: "notFound", message: "missing" };
  const state = {
    recording: null as number | null,
    startError: null as unknown,
    unavailable: [] as { source: "me" | "them"; message: string }[],
  };

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
      if (state.recording === id) throw { kind: "conflict", message: "stop recording before deleting this meeting" };
      if (!meetings.delete(id as number)) throw notFound;
    },
    active_recording: () => state.recording,
    start_recording: ({ meetingId }) => {
      if (state.startError) throw state.startError;
      if (state.recording !== null) throw { kind: "conflict", message: "another meeting is already recording; stop it first" };
      state.recording = meetingId as number;
      return { unavailable: state.unavailable };
    },
    stop_recording: () => {
      const stopped = state.recording;
      state.recording = null;
      return stopped;
    },
    list_transcript: ({ meetingId }) => segments.filter((s) => s.meetingId === meetingId),
  };

  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => handlers[command]((args ?? {}) as Args));
  return Object.assign(meetings, { state });
}

function segment(id: number, meetingId: number, source: "me" | "them", startMs: number, text: string): TranscriptSegment {
  return { id, meetingId, source, startMs, endMs: startMs + 2000, text, language: "en" };
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
  listeners.clear();
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    const set = listeners.get(event) ?? new Set<Listener>();
    set.add(handler as Listener);
    listeners.set(event, set);
    return () => {
      set.delete(handler as Listener);
    };
  });
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

  it("starts and stops recording, and marks the meeting in the sidebar", async () => {
    const backend = fakeBackend([seedMeeting(1, "Design sync")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

    await user.click(await screen.findByRole("button", { name: "Start recording" }));

    expect(await screen.findByRole("button", { name: "Stop recording" })).toBeInTheDocument();
    expect(within(sidebar()).getByRole("button", { name: /Design sync/ })).toHaveTextContent("Recording");
    expect(backend.state.recording).toBe(1);

    await user.click(screen.getByRole("button", { name: "Stop recording" }));

    expect(await screen.findByRole("button", { name: "Start recording" })).toBeInTheDocument();
    expect(backend.state.recording).toBeNull();
  });

  it("explains why recording couldn't start", async () => {
    const backend = fakeBackend([seedMeeting(1, "Design sync")]);
    backend.state.startError = { kind: "unavailable", message: "no microphone is available" };
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

    await user.click(await screen.findByRole("button", { name: "Start recording" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("No microphone is available.");
    expect(screen.getByRole("button", { name: "Start recording" })).toBeInTheDocument();
  });

  it("shows the saved transcript with speakers and times", async () => {
    fakeBackend(
      [seedMeeting(1, "Design sync")],
      [segment(2, 1, "them", 65_000, "Agreed, ship it."), segment(1, 1, "me", 3_000, "Let's lock the copy.")],
    );
    const user = userEvent.setup();
    render(<App />);

    await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

    const transcript = screen.getByRole("complementary", { name: "Transcript" });
    const items = await within(transcript).findAllByRole("listitem");
    expect(items.map((item) => item.textContent)).toEqual(["Me 0:03Let's lock the copy.", "Them 1:05Agreed, ship it."]);
  });

  it("adds live segments for the open meeting only", async () => {
    fakeBackend([seedMeeting(1, "Open"), seedMeeting(2, "Other")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Open/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    const transcript = screen.getByRole("complementary", { name: "Transcript" });
    await within(transcript).findByText(/Listening/);

    emit("transcript:segment", segment(10, 2, "me", 0, "Not for this meeting"));
    emit("transcript:segment", segment(11, 1, "me", 1000, "Hello everyone"));

    expect(await within(transcript).findByText("Hello everyone")).toBeInTheDocument();
    expect(within(transcript).queryByText("Not for this meeting")).not.toBeInTheDocument();
  });

  it("shows transcript empty states", async () => {
    fakeBackend([seedMeeting(1, "Quiet")]);
    const user = userEvent.setup();
    render(<App />);
    const transcript = screen.getByRole("complementary", { name: "Transcript" });

    expect(within(transcript).getByText("Open a meeting to see its transcript.")).toBeInTheDocument();
    await user.click(await within(sidebar()).findByRole("button", { name: /Quiet/ }));

    expect(await within(transcript).findByText(/Start recording to transcribe/)).toBeInTheDocument();
  });

  it("restores the recording state when the window reloads", async () => {
    const backend = fakeBackend([seedMeeting(1, "Still going")]);
    backend.state.recording = 1;
    const user = userEvent.setup();
    render(<App />);

    await user.click(await within(sidebar()).findByRole("button", { name: /Still going/ }));

    expect(await screen.findByRole("button", { name: "Stop recording" })).toBeInTheDocument();
  });

  it("won't delete a meeting that is recording", async () => {
    const backend = fakeBackend([seedMeeting(1, "Live")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Live/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    await screen.findByRole("button", { name: "Stop recording" });

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete meeting" }));

    expect(await screen.findByText("Stop recording before deleting this meeting.")).toBeInTheDocument();
    expect(backend.has(1)).toBe(true);
  });

  it("names both sources while recording", async () => {
    fakeBackend([seedMeeting(1, "Call")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));

    await user.click(await screen.findByRole("button", { name: "Start recording" }));

    expect(await screen.findByText("Recording your microphone and system audio")).toBeInTheDocument();
  });

  it("keeps recording and explains when one source is unavailable", async () => {
    const backend = fakeBackend([seedMeeting(1, "Call")]);
    backend.state.unavailable = [{ source: "them", message: "system audio couldn't start: tap failed" }];
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));

    await user.click(await screen.findByRole("button", { name: "Start recording" }));

    expect(await screen.findByText("Recording your microphone")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("System audio couldn't start: tap failed.");
  });

  it("shows a problem reported during the recording", async () => {
    fakeBackend([seedMeeting(1, "Call")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    await screen.findByRole("button", { name: "Stop recording" });

    emit("recording:issue", { meetingId: 2, source: "me", message: "not this meeting" });
    emit("recording:issue", { meetingId: 1, source: "me", message: "the microphone stopped: device unplugged" });

    expect(await screen.findByRole("alert")).toHaveTextContent("The microphone stopped: device unplugged.");
    expect(screen.getByText("Recording system audio")).toBeInTheDocument();
    expect(screen.queryByText(/not this meeting/i)).not.toBeInTheDocument();
  });

  it("hints at the system audio permission until the other side is heard", async () => {
    fakeBackend([seedMeeting(1, "Call")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    const transcript = screen.getByRole("complementary", { name: "Transcript" });

    expect(await within(transcript).findByText(/Not seeing the other side/)).toBeInTheDocument();
    emit("transcript:segment", segment(5, 1, "them", 1000, "Can you hear me?"));

    await waitFor(() => expect(within(transcript).queryByText(/Not seeing the other side/)).not.toBeInTheDocument());
  });

  it("suggests headphones before recording starts", async () => {
    fakeBackend([seedMeeting(1, "Call")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));

    expect(await screen.findByText(/Use headphones for the best transcript/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Start recording" }));

    await screen.findByRole("button", { name: "Stop recording" });
    expect(screen.queryByText(/Use headphones/)).not.toBeInTheDocument();
  });
});
