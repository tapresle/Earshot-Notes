import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { CONSENT_NOTICE } from "./features/recording/consent";
import type { MeetingDetail, TranscriptSegment } from "./lib/api";
import { createFakeBackend, savedSummary, seedMeeting, segment, testModel } from "./testing/fakeBackend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

type Listener = (event: { payload: unknown }) => void;
const listeners = new Map<string, Set<Listener>>();

/** Delivers a backend event to the UI, like the Rust side's `emit`. */
function emit(event: string, payload: unknown) {
  act(() => listeners.get(event)?.forEach((listener) => listener({ payload })));
}

/** The shared fake backend, answering the UI's `invoke` calls. */
function fakeBackend(seed: MeetingDetail[] = [], segments: TranscriptSegment[] = []) {
  const backend = createFakeBackend(seed, segments);
  vi.mocked(invoke).mockImplementation(backend.handle);
  return backend;
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

  describe("search", () => {
    function searchBox() {
      return within(sidebar()).getByRole("searchbox", { name: "Search meetings" });
    }

    it("finds meetings by title, notes, or transcript", async () => {
      fakeBackend(
        [seedMeeting(1, "Budget review"), seedMeeting(2, "Planning", "- move the budget"), seedMeeting(3, "Standup"), seedMeeting(4, "Retro")],
        [segment(1, 3, "them", 0, "The budget is final.")],
      );
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /Retro/ });

      await user.type(searchBox(), "budg");

      await waitFor(() => expect(within(sidebar()).queryByRole("button", { name: /Retro/ })).not.toBeInTheDocument());
      for (const title of ["Budget review", "Planning", "Standup"]) {
        expect(within(sidebar()).getByRole("button", { name: new RegExp(title) })).toBeInTheDocument();
      }
    });

    it("waits for typing to pause before searching", async () => {
      const backend = fakeBackend([seedMeeting(1, "Design sync")]);
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /Design sync/ });

      await user.type(searchBox(), "design");

      await waitFor(() => expect(backend.state.queries).toContain("design"));
      expect(backend.state.queries).toEqual([null, "design"]);
    });

    it("says when nothing matches, and clears with Escape", async () => {
      fakeBackend([seedMeeting(1, "Design sync")]);
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /Design sync/ });

      await user.type(searchBox(), "zebra");

      expect(await within(sidebar()).findByText("No meetings match “zebra”.")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "Pick a meeting" })).toBeInTheDocument();

      await user.keyboard("{Escape}");

      expect(searchBox()).toHaveValue("");
      expect(await within(sidebar()).findByRole("button", { name: /Design sync/ })).toBeInTheDocument();
    });

    it("pages through search results", async () => {
      const backend = fakeBackend([
        ...Array.from({ length: 55 }, (_, i) => seedMeeting(i + 1, `Sync ${i + 1}`)),
        seedMeeting(100, "Other"),
      ]);
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /^Sync 1,/ });

      await user.type(searchBox(), "sync");
      await waitFor(() => expect(backend.state.queries).toContain("sync"));
      await user.click(await within(sidebar()).findByRole("button", { name: "Show more meetings" }));

      expect(await within(sidebar()).findByRole("button", { name: /^Sync 55,/ })).toBeInTheDocument();
      expect(within(sidebar()).queryByRole("button", { name: /Other/ })).not.toBeInTheDocument();
      expect(backend.state.queries[backend.state.queries.length - 1]).toBe("sync");
    });

    it("clears the search when you create a meeting", async () => {
      fakeBackend([seedMeeting(1, "Design sync")]);
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /Design sync/ });
      await user.type(searchBox(), "zebra");
      await within(sidebar()).findByText("No meetings match “zebra”.");

      await user.click(screen.getByRole("button", { name: "New meeting" }));

      expect(searchBox()).toHaveValue("");
      expect(await within(sidebar()).findByRole("button", { name: /Untitled meeting/, current: "page" })).toBeInTheDocument();
      expect(await within(sidebar()).findByRole("button", { name: /Design sync/ })).toBeInTheDocument();
    });
  });

  describe("paging", () => {
    const many = () => Array.from({ length: 60 }, (_, i) => seedMeeting(i + 1, `Meeting ${i + 1}`));
    const listed = () => within(sidebar()).queryAllByRole("button", { name: /^Meeting \d+,/ });

    it("loads the newest page first and the rest on request", async () => {
      fakeBackend(many());
      const user = userEvent.setup();
      render(<App />);

      expect(await within(sidebar()).findByRole("button", { name: /^Meeting 1,/ })).toBeInTheDocument();
      expect(listed()).toHaveLength(50);
      expect(within(sidebar()).queryByRole("button", { name: /^Meeting 51,/ })).not.toBeInTheDocument();

      await user.click(within(sidebar()).getByRole("button", { name: "Show more meetings" }));

      expect(await within(sidebar()).findByRole("button", { name: /^Meeting 60,/ })).toBeInTheDocument();
      expect(listed()).toHaveLength(60);
      expect(within(sidebar()).queryByRole("button", { name: "Show more meetings" })).not.toBeInTheDocument();
    });

    it("doesn't offer more when every meeting fits on one page", async () => {
      fakeBackend([seedMeeting(1, "Only one")]);
      render(<App />);

      expect(await within(sidebar()).findByRole("button", { name: /Only one/ })).toBeInTheDocument();
      expect(within(sidebar()).queryByRole("button", { name: "Show more meetings" })).not.toBeInTheDocument();
    });

    it("keeps the loaded meetings and offers a retry when the next page fails", async () => {
      const backend = fakeBackend(many());
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /^Meeting 1,/ });
      backend.state.listError = { kind: "internal", message: "disk I/O error" };

      await user.click(within(sidebar()).getByRole("button", { name: "Show more meetings" }));

      expect(await within(sidebar()).findByRole("alert")).toHaveTextContent(
        "Couldn't load more meetings. The app couldn't read or write your meetings.",
      );
      expect(listed()).toHaveLength(50);

      backend.state.listError = null;
      await user.click(within(sidebar()).getByRole("button", { name: "Show more meetings" }));

      expect(await within(sidebar()).findByRole("button", { name: /^Meeting 60,/ })).toBeInTheDocument();
      expect(within(sidebar()).queryByRole("alert")).not.toBeInTheDocument();
    });

    it("keeps a new meeting at the top while paging", async () => {
      fakeBackend(many());
      const user = userEvent.setup();
      render(<App />);
      await within(sidebar()).findByRole("button", { name: /^Meeting 1,/ });

      await user.click(screen.getByRole("button", { name: "New meeting" }));
      await screen.findByLabelText("Meeting title");
      await user.click(within(sidebar()).getByRole("button", { name: "Show more meetings" }));

      await within(sidebar()).findByRole("button", { name: /^Meeting 60,/ });
      const titles = within(sidebar())
        .getAllByRole("button", { name: /^(Meeting \d+|Untitled meeting),/ })
        .map((item) => item.querySelector(".meeting-title")?.textContent);
      expect(titles[0]).toBe("Untitled meeting");
      expect(titles).toHaveLength(61);
      expect(new Set(titles).size).toBe(61);
    });
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
    expect(items.map((item) => item.textContent)).toEqual(["Me 0:03Let's lock the copy.", "Meeting Participants 1:05Agreed, ship it."]);
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

  it("removes an echo line it already showed", async () => {
    fakeBackend([seedMeeting(1, "Open")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Open/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    const transcript = screen.getByRole("complementary", { name: "Transcript" });
    await within(transcript).findByText(/Listening/);

    emit("transcript:segment", segment(20, 1, "me", 1300, "I'll send the strings tonight"));
    emit("transcript:segment", segment(21, 1, "them", 1000, "I'll send the strings tonight."));
    expect(await within(transcript).findByText("I'll send the strings tonight")).toBeInTheDocument();
    emit("transcript:removed", { meetingId: 2, ids: [21] });
    emit("transcript:removed", { meetingId: 1, ids: [20] });

    await waitFor(() => expect(within(transcript).queryByText("I'll send the strings tonight")).not.toBeInTheDocument());
    expect(within(transcript).getByText("I'll send the strings tonight.")).toBeInTheDocument();
  });

  it("replaces a line it already showed when it arrives again without echo", async () => {
    fakeBackend([seedMeeting(1, "Open")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Open/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    const transcript = screen.getByRole("complementary", { name: "Transcript" });
    await within(transcript).findByText(/Listening/);

    emit("transcript:segment", segment(30, 1, "me", 1300, "we moved the invoices yesterday, okay, send the totals"));
    expect(await within(transcript).findByText(/we moved the invoices/)).toBeInTheDocument();
    emit("transcript:segment", segment(30, 1, "me", 1300, "okay, send the totals"));

    expect(await within(transcript).findByText("okay, send the totals")).toBeInTheDocument();
    expect(within(transcript).queryByText(/we moved the invoices/)).not.toBeInTheDocument();
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

  it("opens a meeting the menu bar started recording", async () => {
    const backend = fakeBackend([seedMeeting(1, "Design sync")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

    // What the menu bar's Start recording does: a new meeting, recording.
    const meeting = (await backend.handle("create_meeting", { title: "" })) as { id: number };
    backend.state.recording = meeting.id;
    emit("recording:changed", { meetingId: meeting.id });

    await waitFor(() => expect(screen.getByLabelText("Meeting title")).toHaveValue("Untitled meeting"));
    expect(within(sidebar()).getByRole("button", { name: /Untitled meeting/, current: "page" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Stop recording" })).toBeInTheDocument();
  });

  it("shows a recording stopped from the menu bar", async () => {
    const backend = fakeBackend([seedMeeting(1, "Call")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    await screen.findByRole("button", { name: "Stop recording" });

    backend.state.recording = null;
    emit("recording:changed", { meetingId: null });

    expect(await screen.findByRole("button", { name: "Start recording" })).toBeInTheDocument();
    expect(screen.getByLabelText("Meeting title")).toHaveValue("Call");
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
    const backend = fakeBackend([seedMeeting(1, "Call")]);
    backend.state.systemAudioPermission = null;
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));
    await user.click(await screen.findByRole("button", { name: "Start recording" }));
    const transcript = screen.getByRole("complementary", { name: "Transcript" });

    expect(await within(transcript).findByText(/Allow Earshot Notes in System Settings/)).toBeInTheDocument();
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

  it("reminds you about recording consent before recording starts", async () => {
    fakeBackend([seedMeeting(1, "Call")]);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));

    expect(await screen.findByText(CONSENT_NOTICE)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Start recording" }));

    await screen.findByRole("button", { name: "Stop recording" });
    expect(screen.queryByText(CONSENT_NOTICE)).not.toBeInTheDocument();
  });

  describe("summaries", () => {
    async function openMeeting(title: string) {
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: new RegExp(title) }));
      await screen.findByRole("tab", { name: "My notes" });
      return user;
    }

    it("explains how to add a model and imports one", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      vi.mocked(open).mockResolvedValue("/Users/me/Downloads/gemma-4-E2B-it-Q4_K_M.gguf");
      const user = await openMeeting("Sync");

      await user.click(screen.getByRole("tab", { name: "Summary" }));
      expect(await screen.findByRole("heading", { name: "Add a summary model" })).toBeInTheDocument();
      await user.click(await screen.findByRole("button", { name: "Import model file" }));

      expect(await screen.findByText(/No summary yet/)).toBeInTheDocument();
      expect(backend.state.model?.filePath).toBe("/Users/me/Downloads/gemma-4-E2B-it-Q4_K_M.gguf");
    });

    it("leaves things unchanged when the file picker is cancelled", async () => {
      fakeBackend([seedMeeting(1, "Sync")]);
      vi.mocked(open).mockResolvedValue(null);
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      await user.click(await screen.findByRole("button", { name: "Import model file" }));

      expect(screen.getByRole("heading", { name: "Add a summary model" })).toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("explains a file that isn't a model", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.importError = { kind: "invalid", message: "that file isn't a GGUF model" };
      vi.mocked(open).mockResolvedValue("/tmp/notes.gguf");
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      await user.click(await screen.findByRole("button", { name: "Import model file" }));

      expect(await screen.findByRole("alert")).toHaveTextContent("That file isn't a GGUF model.");
    });

    it("streams a summary and then shows the saved notes", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.model = testModel();
      const user = await openMeeting("Sync");

      await user.click(screen.getByRole("button", { name: "Summarize" }));

      expect(screen.getByRole("tab", { name: "Summary" })).toHaveAttribute("aria-selected", "true");
      expect(await screen.findByText("Loading the model…")).toBeInTheDocument();
      emit("summary:progress", { meetingId: 1, stage: "reading", part: 1, parts: 3 });
      expect(screen.getByText("Reading the transcript, part 1 of 3…")).toBeInTheDocument();
      emit("summary:progress", { meetingId: 1, stage: "writing" });
      emit("summary:text", { meetingId: 1, text: "## Decisions\n- Ship " });
      emit("summary:text", { meetingId: 1, text: "Friday" });
      expect(screen.getByRole("heading", { name: "Decisions" })).toBeInTheDocument();
      expect(screen.getByText("Ship Friday")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Summarizing…" })).toBeDisabled();

      emit("summary:done", { meetingId: 1, summary: savedSummary(1, "## Decisions\n- Ship Friday") });

      expect(await screen.findByText("Summary of the whole meeting")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Summarize" })).toBeEnabled();
    });

    it("shows the title a summary gives an untitled meeting", async () => {
      const backend = fakeBackend([seedMeeting(1, "Untitled meeting")]);
      backend.state.model = testModel();
      const user = await openMeeting("Untitled meeting");
      await user.click(screen.getByRole("button", { name: "Summarize" }));

      emit("summary:done", { meetingId: 1, summary: savedSummary(1, "- Cut travel"), title: "Budget review" });

      await waitFor(() => expect(screen.getByLabelText("Meeting title")).toHaveValue("Budget review"));
      expect(within(sidebar()).getByRole("button", { name: /Budget review/ })).toBeInTheDocument();
    });

    it("names a meeting in the sidebar while another meeting is open", async () => {
      fakeBackend([seedMeeting(1, "Untitled meeting"), seedMeeting(2, "Standup")]);
      await openMeeting("Standup");

      emit("summary:done", { meetingId: 1, summary: savedSummary(1, "- Cut travel"), title: "Budget review" });

      expect(await within(sidebar()).findByRole("button", { name: /Budget review/ })).toBeInTheDocument();
      expect(screen.getByLabelText("Meeting title")).toHaveValue("Standup");
    });

    it("keeps a title you're typing when a summary names the meeting", async () => {
      const backend = fakeBackend([seedMeeting(1, "Untitled meeting")]);
      const user = await openMeeting("Untitled meeting");
      const field = screen.getByLabelText("Meeting title");
      await user.clear(field);
      await user.type(field, "Mine");

      emit("summary:done", { meetingId: 1, summary: savedSummary(1, "- Cut travel"), title: "Budget review" });
      await within(sidebar()).findByRole("button", { name: /Budget review/ });
      expect(field).toHaveValue("Mine");
      await user.tab();

      expect(await within(sidebar()).findByRole("button", { name: /Mine/ })).toBeInTheDocument();
      expect(backend.get(1)?.title).toBe("Mine");
    });

    it("labels the button and the result as partial while recording", async () => {
      const backend = fakeBackend([seedMeeting(1, "Live")]);
      backend.state.model = testModel();
      backend.state.recording = 1;
      backend.state.summaries.set(1, savedSummary(1, "- so far", false));
      const user = await openMeeting("Live");

      expect(screen.getByRole("button", { name: "Summarize so far" })).toBeInTheDocument();
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      expect(await screen.findByText("Partial summary, up to 12:34 into the meeting")).toBeInTheDocument();
    });

    it("cancels a running summary", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.model = testModel();
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("button", { name: "Summarize" }));

      await user.click(await screen.findByRole("button", { name: "Cancel" }));
      emit("summary:failed", { meetingId: 1, message: "the summary was cancelled", cancelled: true });

      expect(backend.state.cancelled).toBe(true);
      expect(await screen.findByText("Summary cancelled.")).toBeInTheDocument();
    });

    it("shows why a summary couldn't start", async () => {
      const backend = fakeBackend([seedMeeting(1, "Empty")]);
      backend.state.model = testModel();
      backend.state.summarizeError = {
        kind: "conflict",
        message: "there's nothing to summarize yet. Record the meeting or write some notes first",
      };
      const user = await openMeeting("Empty");

      await user.click(screen.getByRole("button", { name: "Summarize" }));

      expect(await screen.findByText(/There's nothing to summarize yet/)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Summarize" })).toBeEnabled();
    });

    it("keeps unsaved notes when switching tabs", async () => {
      fakeBackend([seedMeeting(1, "Sync")]);
      const user = await openMeeting("Sync");

      await user.type(screen.getByLabelText("Meeting notes"), "draft idea");
      await user.click(screen.getByRole("tab", { name: "Summary" }));
      await user.click(screen.getByRole("tab", { name: "My notes" }));

      expect(screen.getByLabelText("Meeting notes")).toHaveValue("draft idea");
    });

    it("resumes showing a summary that was already running", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.model = testModel();
      backend.state.summarizing = 1;
      const user = await openMeeting("Sync");

      await user.click(screen.getByRole("tab", { name: "Summary" }));

      expect(await screen.findByText("Loading the model…")).toBeInTheDocument();
    });

    it("downloads the recommended model with progress", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      expect(await screen.findByText("Recommended")).toBeInTheDocument();
      await user.click(within(screen.getByRole("group", { name: "Gemma 4 E2B" })).getByRole("button", { name: "Download (3.1 GB)" }));

      expect(backend.state.downloading).toBe(true);
      emit("model:download-progress", {
        modelId: "gemma-4-e2b-it-q4_k_m",
        phase: "downloading",
        received: 1_553_369_136,
        total: 3_106_738_272,
      });
      expect(screen.getByRole("status")).toHaveTextContent("1.6 GB of 3.1 GB (50%)");
      expect(screen.getByRole("progressbar", { name: "Downloading Gemma 4 E2B" })).toHaveAttribute("value", "1553369136");
      emit("model:download-progress", { modelId: "gemma-4-e2b-it-q4_k_m", phase: "verifying" });
      expect(screen.getByText("Verifying the download…")).toBeInTheDocument();

      emit("model:download-done", { model: testModel() });

      expect(await screen.findByText(/No summary yet/)).toBeInTheDocument();
    });

    it("lists every model with a description and recommends only Gemma 4 E2B", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      expect(await screen.findByText("High-quality summaries. Slower, and uses more memory.")).toBeInTheDocument();
      expect(screen.getByText("Fast and light. A good fit for most meetings.")).toBeInTheDocument();
      expect(screen.getAllByText("Recommended")).toHaveLength(1);
      const qwen = screen.getByRole("group", { name: "Qwen3 8B" });

      await user.click(within(qwen).getByRole("button", { name: "Download (5.0 GB)" }));

      expect(backend.state.downloadedId).toBe("qwen3-8b-q4_k_m");
    });

    it("shows where each model comes from and opens its Hugging Face page", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      const qwen = await screen.findByRole("group", { name: "Qwen3 8B" });
      expect(qwen).toHaveTextContent("unsloth/Qwen3-8B-GGUF - Q4_K_M - a6adef1");
      await user.click(within(qwen).getByRole("button", { name: "unsloth/Qwen3-8B-GGUF" }));

      expect(backend.state.openedModelPage).toBe("qwen3-8b-q4_k_m");
    });

    it("explains when a model's page can't be opened", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.openModelPageError = { kind: "unavailable", message: "couldn't open the model's Hugging Face page" };
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      await user.click(await screen.findByRole("button", { name: "unsloth/gemma-4-E2B-it-GGUF" }));

      expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't open the model's Hugging Face page");
    });

    it("offers to resume a partial download", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.partialBytes = 1_200_000_000;
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      expect(await screen.findByRole("button", { name: "Resume download (1.2 of 3.1 GB)" })).toBeInTheDocument();
    });

    it("cancels a download and explains a failed one", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));
      await user.click(within(await screen.findByRole("group", { name: "Gemma 4 E2B" })).getByRole("button", { name: "Download (3.1 GB)" }));

      await user.click(await screen.findByRole("button", { name: "Cancel" }));
      expect(backend.state.downloadCancelled).toBe(true);
      backend.state.downloading = false;
      backend.state.partialBytes = 400_000_000;
      emit("model:download-failed", { modelId: "gemma-4-e2b-it-q4_k_m", message: "the download was cancelled", cancelled: true });
      expect(await screen.findByRole("button", { name: "Resume download (0.4 of 3.1 GB)" })).toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();

      emit("model:download-failed", {
        modelId: "gemma-4-e2b-it-q4_k_m",
        message: "there isn't enough free disk space for this model",
        cancelled: false,
      });
      expect(await screen.findByRole("alert")).toHaveTextContent("There isn't enough free disk space for this model.");
    });

    it("picks up a download already in progress", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.downloading = true;
      backend.state.partialBytes = 310_673_828;
      const user = await openMeeting("Sync");

      await user.click(screen.getByRole("tab", { name: "Summary" }));

      expect(await screen.findByText(/0.3 GB of 3.1 GB \(10%\)/)).toBeInTheDocument();
    });
  });

  describe("export", () => {
    it("exports one meeting to the file the user picks", async () => {
      const backend = fakeBackend([seedMeeting(1, "Design sync")]);
      vi.mocked(save).mockResolvedValue("/Users/me/Documents/design.md");
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

      await user.click(await screen.findByRole("button", { name: "Export" }));

      expect(vi.mocked(save)).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "2026-10-03-meeting-1.md" }));
      expect(await screen.findByText("/Users/me/Documents/design.md")).toBeInTheDocument();
      expect(backend.state.savedTo).toBe("/Users/me/Documents/design.md");
    });

    it("does nothing when the save dialog is cancelled", async () => {
      const backend = fakeBackend([seedMeeting(1, "Design sync")]);
      vi.mocked(save).mockResolvedValue(null);
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: /Design sync/ }));

      await user.click(await screen.findByRole("button", { name: "Export" }));

      expect(backend.state.savedTo).toBeNull();
      expect(screen.queryByText(/Saved to/)).not.toBeInTheDocument();
    });

    it("exports all meetings from Settings", async () => {
      const backend = fakeBackend([seedMeeting(1, "One"), seedMeeting(2, "Two")]);
      vi.mocked(open).mockResolvedValue("/Users/me/Exports");
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Export all meetings" }));

      expect(await screen.findByText(/Exported 2 meetings to/)).toBeInTheDocument();
      expect(backend.state.exports).toEqual([{ ids: null, folder: "/Users/me/Exports" }]);
    });

    it("exports every meeting from Settings while a search filters the sidebar", async () => {
      const backend = fakeBackend([seedMeeting(1, "One"), seedMeeting(2, "Two")]);
      vi.mocked(open).mockResolvedValue("/out");
      const user = userEvent.setup();
      render(<App />);
      await user.type(await screen.findByRole("searchbox", { name: "Search meetings" }), "nothing matches this");
      await within(sidebar()).findByText(/No meetings match/);

      expect(within(sidebar()).queryByRole("button", { name: /Export all/ })).not.toBeInTheDocument();
      await user.click(within(sidebar()).getByRole("button", { name: "Settings" }));
      await user.click(await screen.findByRole("button", { name: "Export all meetings" }));

      expect(await screen.findByText(/Exported 2 meetings to/)).toBeInTheDocument();
      expect(backend.state.exports).toEqual([{ ids: null, folder: "/out" }]);
    });

    it("selects meetings with Cmd-click and exports the selection", async () => {
      const backend = fakeBackend([seedMeeting(1, "One"), seedMeeting(2, "Two"), seedMeeting(3, "Three")]);
      vi.mocked(open).mockResolvedValue("/tmp/out");
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: /One/ }));

      await user.keyboard("{Meta>}");
      await user.click(within(sidebar()).getByRole("button", { name: /Three/ }));
      await user.keyboard("{/Meta}");

      expect(within(sidebar()).getByText("2 meetings selected")).toBeInTheDocument();
      expect(within(sidebar()).getByRole("button", { name: /^One, .*, selected$/ })).toBeInTheDocument();
      expect(within(sidebar()).getByRole("button", { name: /^Two, [^,]*$/ })).toBeInTheDocument();
      await user.click(within(sidebar()).getByRole("button", { name: "Export selected" }));
      expect(await within(sidebar()).findByText(/Exported 2 meetings/)).toBeInTheDocument();
      expect(backend.state.exports[0].ids).toEqual([1, 3]);
      // The open meeting doesn't change on a Cmd-click.
      expect(screen.getByLabelText("Meeting title")).toHaveValue("One");
    });

    it("selects a range with Shift-click and clears it", async () => {
      fakeBackend([seedMeeting(1, "One"), seedMeeting(2, "Two"), seedMeeting(3, "Three")]);
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: /One/ }));

      await user.keyboard("{Shift>}");
      await user.click(within(sidebar()).getByRole("button", { name: /Three/ }));
      await user.keyboard("{/Shift}");

      expect(within(sidebar()).getByText("3 meetings selected")).toBeInTheDocument();
      await user.click(within(sidebar()).getByRole("button", { name: "Clear" }));
      expect(within(sidebar()).queryByText(/selected/)).not.toBeInTheDocument();
    });

    it("lists meetings that failed to export", async () => {
      const backend = fakeBackend([seedMeeting(1, "One"), seedMeeting(2, "Two")]);
      backend.state.exportFailures = [{ title: "Two", message: "permission denied" }];
      vi.mocked(open).mockResolvedValue("/locked");
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Export all meetings" }));

      expect(await screen.findByText(/Exported 1 meeting to/)).toBeInTheDocument();
      expect(screen.getByText("Couldn't export 1 meeting:")).toBeInTheDocument();
      expect(screen.getByText("Two: permission denied")).toBeInTheDocument();
    });

    it("says when there are no meetings to export", async () => {
      fakeBackend();
      vi.mocked(open).mockResolvedValue("/out");
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Export all meetings" }));

      expect(await screen.findByText("There are no meetings to export yet.")).toBeInTheDocument();
    });
  });

  describe("settings", () => {
    async function openSettings(backendSeed: MeetingDetail[] = [seedMeeting(1, "Sync")]) {
      const backend = fakeBackend(backendSeed);
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      await screen.findByRole("heading", { level: 1, name: "Settings" });
      return { backend, user };
    }

    it("opens from the sidebar and closes when a meeting is chosen", async () => {
      const { user } = await openSettings();

      await user.click(within(sidebar()).getByRole("button", { name: /Sync/ }));

      expect(screen.queryByRole("heading", { level: 1, name: "Settings" })).not.toBeInTheDocument();
      expect(await screen.findByLabelText("Meeting title")).toHaveValue("Sync");
    });

    describe("open at login", () => {
      async function openWith(launchAtLogin: { enabled: boolean; needsApproval: boolean; available: boolean }) {
        const backend = fakeBackend([seedMeeting(1, "Sync")]);
        backend.state.launchAtLogin = launchAtLogin;
        const user = userEvent.setup();
        render(<App />);
        await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
        return { backend, user };
      }

      it("is off by default and turns on and off", async () => {
        const { backend, user } = await openWith({ enabled: false, needsApproval: false, available: true });
        const option = await screen.findByRole("checkbox", { name: "Open at login" });

        expect(option).not.toBeChecked();
        await user.click(option);
        await waitFor(() => expect(backend.state.launchAtLogin.enabled).toBe(true));
        expect(option).toBeChecked();
        await user.click(option);
        await waitFor(() => expect(backend.state.launchAtLogin.enabled).toBe(false));
      });

      it("explains when macOS is waiting for approval", async () => {
        await openWith({ enabled: true, needsApproval: true, available: true });

        expect(await screen.findByRole("checkbox", { name: "Open at login" })).toBeChecked();
        expect(screen.getByText(/System Settings > General > Login Items/)).toBeInTheDocument();
      });

      it("can't be turned on where the system has no login items", async () => {
        await openWith({ enabled: false, needsApproval: false, available: false });

        expect(await screen.findByRole("checkbox", { name: "Open at login" })).toBeDisabled();
        expect(screen.getByText(/isn't available on this system/)).toBeInTheDocument();
      });

      it("shows why a change failed", async () => {
        const { backend, user } = await openWith({ enabled: false, needsApproval: false, available: true });
        backend.state.launchAtLoginError = { kind: "unavailable", message: "couldn't change opening at login: denied" };

        await user.click(await screen.findByRole("checkbox", { name: "Open at login" }));

        expect(await screen.findByText(/couldn't change opening at login: denied/i)).toHaveAttribute("role", "alert");
      });
    });

    describe("vocabulary", () => {
      // Invented names only.
      async function openWith(setup: (state: ReturnType<typeof fakeBackend>["state"]) => void = () => {}) {
        const backend = fakeBackend([seedMeeting(1, "Sync")]);
        setup(backend.state);
        const user = userEvent.setup();
        render(<App />);
        await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
        await screen.findByRole("button", { name: "Add term" });
        return { backend, user };
      }

      it("shows the saved terms and their mishearings", async () => {
        await openWith((state) => {
          state.vocabulary = [{ term: "Bluefin Ledger", heardAs: ["blue fin ledger", "bluefinn ledger"] }];
        });

        expect(screen.getByLabelText("Term 1")).toHaveValue("Bluefin Ledger");
        expect(screen.getByLabelText("Heard as, term 1")).toHaveValue("blue fin ledger, bluefinn ledger");
      });

      it("adds a term and reports the transcripts it corrected", async () => {
        const { backend, user } = await openWith((state) => (state.vocabularyUpdated = 3));

        await user.click(screen.getByRole("button", { name: "Add term" }));
        await user.type(screen.getByLabelText("Term 1"), "Corvina");
        await user.type(screen.getByLabelText("Heard as, term 1"), "corvena, core vina");
        await user.click(screen.getByRole("button", { name: "Save vocabulary" }));

        expect(await screen.findByRole("status")).toHaveTextContent("Saved. Corrected 3 transcript lines.");
        expect(backend.state.vocabulary).toEqual([{ term: "Corvina", heardAs: ["corvena", "core vina"] }]);
        expect(screen.getByLabelText("Heard as, term 1")).toHaveValue("corvena, core vina");
      });

      it("removes a term and skips empty rows when saving", async () => {
        const { backend, user } = await openWith((state) => {
          state.vocabulary = [
            { term: "Bluefin Ledger", heardAs: [] },
            { term: "Corvina", heardAs: [] },
          ];
        });

        await user.click(screen.getByRole("button", { name: "Remove term 1" }));
        await user.click(screen.getByRole("button", { name: "Add term" }));
        await user.click(screen.getByRole("button", { name: "Save vocabulary" }));

        expect(await screen.findByRole("status")).toHaveTextContent("Saved");
        expect(backend.state.vocabulary).toEqual([{ term: "Corvina", heardAs: [] }]);
      });

      it("adds the software terms that are missing, for you to save", async () => {
        const { backend, user } = await openWith((state) => {
          state.vocabulary = [{ term: "Async", heardAs: ["a sink"] }];
        });

        await user.click(screen.getByRole("button", { name: "Add software terms" }));

        expect(screen.getByRole("status")).toHaveTextContent(
          "Added 27 software terms. Save the vocabulary to keep them.",
        );
        expect(screen.getByLabelText("Term 1")).toHaveValue("Async");
        expect(screen.getByLabelText("Term 2")).toHaveValue("await");
        expect(screen.getByLabelText("Heard as, term 2")).toHaveValue("");
        expect(backend.state.vocabulary).toHaveLength(1);

        await user.click(screen.getByRole("button", { name: "Save vocabulary" }));

        expect(await screen.findByRole("status")).toHaveTextContent("Saved");
        expect(backend.state.vocabulary).toHaveLength(28);
        expect(backend.state.vocabulary[0]).toEqual({ term: "Async", heardAs: ["a sink"] });

        await user.click(screen.getByRole("button", { name: "Add software terms" }));

        expect(screen.getByRole("status")).toHaveTextContent("Every software term is already in your vocabulary.");
      });

      it("explains why a vocabulary can't be saved", async () => {
        const { user } = await openWith((state) => {
          state.vocabularyError = {
            kind: "invalid",
            message: 'the mishearing "cor-vena" can only contain letters, numbers, and spaces',
          };
        });

        await user.click(screen.getByRole("button", { name: "Add term" }));
        await user.type(screen.getByLabelText("Term 1"), "Corvina");
        await user.click(screen.getByRole("button", { name: "Save vocabulary" }));

        expect(await screen.findByRole("alert")).toHaveTextContent(
          'The mishearing "cor-vena" can only contain letters, numbers, and spaces.',
        );
      });
    });

    describe("check for updates", () => {
      async function openWith(setup: (state: ReturnType<typeof fakeBackend>["state"]) => void) {
        const backend = fakeBackend([seedMeeting(1, "Sync")]);
        setup(backend.state);
        const user = userEvent.setup();
        render(<App />);
        await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
        await screen.findByLabelText("Record from");
        return { backend, user };
      }

      it("isn't offered while switched off", async () => {
        const { backend } = await openWith((state) => (state.updateCheckEnabled = false));

        expect(screen.queryByRole("heading", { name: "Updates" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Check for updates" })).not.toBeInTheDocument();
        expect(backend.state.updateChecks).toBe(0);
      });

      it("contacts GitHub only when asked", async () => {
        const { backend, user } = await openWith((state) => (state.updateCheckEnabled = true));

        expect(await screen.findByRole("button", { name: "Check for updates" })).toBeInTheDocument();
        expect(backend.state.updateChecks).toBe(0);
        await user.click(screen.getByRole("button", { name: "Check for updates" }));
        await waitFor(() => expect(backend.state.updateChecks).toBe(1));
      });

      it("shows a newer release's notes and opens its page", async () => {
        const { backend, user } = await openWith((state) => {
          state.updateCheckEnabled = true;
          state.updateCheck = {
            current: "0.1.6",
            latest: "0.2.0",
            newer: true,
            notes: "### Improved\n\n- **Faster summaries.** (#90)",
          };
        });

        await user.click(await screen.findByRole("button", { name: "Check for updates" }));

        expect(await screen.findByText("Earshot Notes 0.2.0 is available. You have 0.1.6.")).toBeInTheDocument();
        expect(screen.getByRole("heading", { name: "Improved" })).toBeInTheDocument();
        expect(screen.getByText("Faster summaries.")).toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Open download page" }));
        expect(backend.state.openedReleasePage).toBe("0.2.0");
      });

      it("says when you have the latest version", async () => {
        await openWith((state) => (state.updateCheckEnabled = true));

        await userEvent.click(await screen.findByRole("button", { name: "Check for updates" }));

        expect(await screen.findByText("You have the latest version, 0.1.6.")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Open download page" })).not.toBeInTheDocument();
      });

      it("explains a failed check", async () => {
        await openWith((state) => {
          state.updateCheckEnabled = true;
          state.updateCheckError = {
            kind: "unavailable",
            message: "couldn't reach GitHub, check your internet connection: dns lookup failed",
          };
        });

        await userEvent.click(await screen.findByRole("button", { name: "Check for updates" }));

        expect(await screen.findByRole("alert")).toHaveTextContent(
          "Couldn't reach GitHub, check your internet connection: dns lookup failed.",
        );
      });
    });

    it("chooses a microphone", async () => {
      const { backend, user } = await openSettings();
      const select = await screen.findByLabelText("Record from");

      expect(within(select).getByRole("option", { name: "System default (MacBook Pro Microphone)" })).toBeInTheDocument();
      await user.selectOptions(select, "USB Podcast Mic");

      expect(await screen.findByText("Saved")).toBeInTheDocument();
      expect(backend.state.inputDevice).toBe("usb");
      await user.selectOptions(select, "System default (MacBook Pro Microphone)");
      await waitFor(() => expect(backend.state.inputDevice).toBeNull());
    });

    it("shows and saves the speech detection levels", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.sensitivity = { me: "normal", them: "high" };
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      const myVoice = await screen.findByLabelText("My voice");
      const participants = screen.getByLabelText("Meeting Participants");

      expect(myVoice).toHaveValue("normal");
      expect(participants).toHaveValue("high");
      await user.selectOptions(myVoice, "High");
      await user.selectOptions(participants, "Low");

      expect(await screen.findByText("Saved")).toBeInTheDocument();
      expect(backend.state.sensitivity).toEqual({ me: "high", them: "low" });
      expect(myVoice).toHaveValue("high");
      expect(participants).toHaveValue("low");
    });

    describe("audio recordings", () => {
      async function openWith(setup: (state: ReturnType<typeof fakeBackend>["state"]) => void) {
        const backend = fakeBackend([seedMeeting(1, "Sync")]);
        setup(backend.state);
        const user = userEvent.setup();
        render(<App />);
        await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
        await screen.findByRole("heading", { name: "Audio recordings" });
        return { backend, user };
      }

      it("turns keeping audio on and off", async () => {
        const { backend, user } = await openWith(() => {});
        const keep = await screen.findByRole("checkbox", { name: "Keep audio recordings" });

        expect(keep).not.toBeChecked();
        await user.click(keep);
        await waitFor(() => expect(backend.state.keepAudio).toBe(true));
        expect(keep).toBeChecked();
        await user.click(keep);
        await waitFor(() => expect(backend.state.keepAudio).toBe(false));
      });

      it("deletes every recording after confirming and shows the new size", async () => {
        const { backend, user } = await openWith((state) => {
          state.recordingsSize = 1_234_000_000;
        });

        await user.click(await screen.findByRole("button", { name: "Delete all recordings (1.2 GB)" }));
        const confirm = screen.getByRole("group", { name: "Delete all recordings" });
        expect(within(confirm).getByRole("button", { name: "Cancel" })).toHaveFocus();
        await user.click(within(confirm).getByRole("button", { name: "Delete" }));

        expect(await screen.findByText("Deleted all recordings.")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "No recordings to delete" })).toBeDisabled();
        expect(backend.state.recordingsSize).toBe(0);
      });

      it("keeps the recordings when the confirmation is cancelled", async () => {
        const { backend, user } = await openWith((state) => {
          state.recordingsSize = 230_000_000;
        });

        await user.click(await screen.findByRole("button", { name: "Delete all recordings (230 MB)" }));
        await user.click(screen.getByRole("button", { name: "Cancel" }));

        expect(screen.getByRole("button", { name: "Delete all recordings (230 MB)" })).toHaveFocus();
        expect(backend.state.recordingsSize).toBe(230_000_000);
      });

      it("can't delete recordings while recording", async () => {
        await openWith((state) => {
          state.recording = 1;
          state.recordingsSize = 5_000_000;
        });

        const button = await screen.findByRole("button", { name: "Delete all recordings (5 MB)" });
        expect(button).toBeDisabled();
        expect(button).toHaveAccessibleDescription("Stop recording to delete recordings.");
      });

      it("says when there are no recordings", async () => {
        await openWith(() => {});

        expect(await screen.findByRole("button", { name: "No recordings to delete" })).toBeDisabled();
      });

      it("shows the recordings folder in Finder", async () => {
        const { backend, user } = await openWith(() => {});

        await user.click(await screen.findByRole("button", { name: "Show in Finder" }));

        await waitFor(() => expect(backend.state.shownRecordings).toBe(true));
      });
    });

    it("warns when the chosen microphone isn't connected", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.inputDevice = "unplugged-headset";
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      expect(await screen.findByLabelText("Record from")).toHaveValue("unplugged-headset");
      expect(screen.getByText(/That microphone isn't connected/)).toBeInTheDocument();
    });

    it("switches the summary model", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const gemma = testModel();
      const other = { ...testModel("/models/other.gguf"), id: "local-abc", displayName: "My fine-tune", source: "import" as const };
      backend.state.installed = [gemma, other];
      backend.state.model = gemma;
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      const gemmaChoice = await screen.findByRole("radio", { name: /Gemma 4 E2B/ });
      expect(gemmaChoice).toBeChecked();
      const summaryModels = screen.getByRole("region", { name: "Summary model" });
      expect(within(summaryModels).getAllByText("Selected")).toHaveLength(1);
      expect(gemmaChoice.closest("label")).toHaveTextContent("Selected");
      await user.click(screen.getByRole("radio", { name: /My fine-tune/ }));

      await waitFor(() => expect(backend.state.model?.id).toBe("local-abc"));
      expect(screen.getByRole("radio", { name: /My fine-tune/ })).toBeChecked();
      expect(within(summaryModels).getAllByText("Selected")).toHaveLength(1);
      expect(screen.getByRole("radio", { name: /My fine-tune/ }).closest("label")).toHaveTextContent("Selected");
    });

    it("shows the Hugging Face source of installed models it knows", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const other = { ...testModel("/models/other.gguf"), id: "local-abc", displayName: "My fine-tune" };
      backend.state.installed = [testModel(), other];
      backend.state.model = testModel();
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      const gemmaCard = (await screen.findByRole("radio", { name: /Gemma 4 E2B/ })).closest(".installed-model") as HTMLElement;
      const otherCard = screen.getByRole("radio", { name: /My fine-tune/ }).closest(".installed-model") as HTMLElement;

      expect(gemmaCard).toHaveTextContent("unsloth/gemma-4-E2B-it-GGUF - Q4_K_M - 0314792");
      expect(within(otherCard).queryByRole("button", { name: /GGUF/ })).not.toBeInTheDocument();
      await user.click(within(gemmaCard).getByRole("button", { name: "unsloth/gemma-4-E2B-it-GGUF" }));

      expect(backend.state.openedModelPage).toBe("gemma-4-e2b-it-q4_k_m");
      expect(screen.getByRole("radio", { name: /Gemma 4 E2B/ })).toBeChecked();
    });

    it("removes a model after confirming, and can cancel", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const gemma = testModel();
      const other = { ...testModel("/models/other.gguf"), id: "local-abc", displayName: "My fine-tune" };
      backend.state.installed = [gemma, other];
      backend.state.model = gemma;
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Remove My fine-tune" }));
      const confirm = screen.getByRole("group", { name: "Remove My fine-tune" });
      expect(confirm).toHaveTextContent("Delete it and free 3.1 GB? You can import it again later.");
      expect(within(confirm).getByRole("button", { name: "Cancel" })).toHaveFocus();
      await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
      expect(screen.getByRole("button", { name: "Remove My fine-tune" })).toHaveFocus();
      expect(backend.state.installed).toHaveLength(2);

      await user.click(screen.getByRole("button", { name: "Remove My fine-tune" }));
      await user.click(within(screen.getByRole("group", { name: "Remove My fine-tune" })).getByRole("button", { name: "Remove" }));

      await waitFor(() => expect(screen.queryByRole("radio", { name: /My fine-tune/ })).not.toBeInTheDocument());
      expect(backend.state.installed).toEqual([gemma]);
      expect(screen.getByRole("radio", { name: /Gemma 4 E2B/ })).toBeChecked();
    });

    it("switches to the remaining model when the selected one is removed", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const gemma = testModel();
      const other = { ...testModel("/models/other.gguf"), id: "local-abc", displayName: "My fine-tune" };
      backend.state.installed = [gemma, other];
      backend.state.model = gemma;
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Remove Gemma 4 E2B" }));
      await user.click(within(screen.getByRole("group", { name: "Remove Gemma 4 E2B" })).getByRole("button", { name: "Remove" }));

      expect(await screen.findByRole("radio", { name: /My fine-tune/ })).toBeChecked();
      expect(screen.queryByRole("radio", { name: /Gemma 4 E2B/ })).not.toBeInTheDocument();
    });

    it("offers the download again after removing the last model", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const gemma = { ...testModel(), source: "download" as const };
      backend.state.installed = [gemma];
      backend.state.model = gemma;
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      const card = await screen.findByRole("group", { name: "Gemma 4 E2B" });
      await waitFor(() => expect(within(card).getByText("Installed")).toBeInTheDocument());

      await user.click(screen.getByRole("button", { name: "Remove Gemma 4 E2B" }));
      expect(screen.getByRole("group", { name: "Remove Gemma 4 E2B" })).toHaveTextContent("You can download it again later.");
      await user.click(within(screen.getByRole("group", { name: "Remove Gemma 4 E2B" })).getByRole("button", { name: "Remove" }));

      await waitFor(() =>
        expect(within(screen.getByRole("region", { name: "Summary model" })).queryByRole("radio")).not.toBeInTheDocument(),
      );
      expect(
        within(await screen.findByRole("group", { name: "Gemma 4 E2B" })).getByRole("button", { name: "Download (3.1 GB)" }),
      ).toBeInTheDocument();
    });

    it("treats a model that's already gone as removed", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.installed = [testModel()];
      backend.state.model = testModel();
      backend.state.removeError = { kind: "notFound", missing: "model", message: "model gemma-4-e2b-it-q4_k_m doesn't exist" };
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      await user.click(await screen.findByRole("button", { name: "Remove Gemma 4 E2B" }));
      backend.state.installed = [];
      backend.state.model = null;

      await user.click(within(screen.getByRole("group", { name: "Remove Gemma 4 E2B" })).getByRole("button", { name: "Remove" }));

      await waitFor(() =>
        expect(within(screen.getByRole("region", { name: "Summary model" })).queryByRole("radio")).not.toBeInTheDocument(),
      );
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("explains when a model to select is no longer installed", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const other = { ...testModel("/models/other.gguf"), id: "local-abc", displayName: "My fine-tune" };
      backend.state.installed = [testModel(), other];
      backend.state.model = testModel();
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      const radio = await screen.findByRole("radio", { name: /My fine-tune/ });
      backend.state.installed = [testModel()];

      await user.click(radio);

      expect(await screen.findByRole("alert")).toHaveTextContent("That summary model is no longer installed.");
      expect(screen.getByRole("alert")).not.toHaveTextContent("meeting");
    });

    it("explains why a model couldn't be removed", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.installed = [testModel()];
      backend.state.model = testModel();
      backend.state.removeError = { kind: "conflict", message: "a summary is already being written; wait for it or cancel it" };
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Remove Gemma 4 E2B" }));
      await user.click(within(screen.getByRole("group", { name: "Remove Gemma 4 E2B" })).getByRole("button", { name: "Remove" }));

      expect(await screen.findByRole("alert")).toHaveTextContent("A summary is already being written");
      expect(screen.getByRole("radio", { name: /Gemma 4 E2B/ })).toBeInTheDocument();
    });

    it("offers Whistle, selected, and Whisper to download", async () => {
      await openSettings();

      expect(await screen.findByRole("radio", { name: /Whistle/ })).toBeChecked();
      const whisper = screen.getByRole("radio", { name: /Whisper large-v3-turbo/ });
      expect(whisper).toBeDisabled();
      expect(screen.getByText("Fast and light, and works from the first launch.")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Download (574 MB)" })).toBeInTheDocument();
    });

    it("downloads Whisper and shows its progress", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Download (574 MB)" }));

      expect(backend.state.whisperDownloading).toBe(true);
      expect(await screen.findByRole("progressbar", { name: "Downloading Whisper large-v3-turbo" })).toBeInTheDocument();
      expect(screen.getByRole("status")).toHaveTextContent("0 MB of 574 MB (0%)");
    });

    it("chooses a downloaded Whisper for new recordings", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.whisperInstalled = true;
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("radio", { name: /Whisper large-v3-turbo/ }));

      expect(backend.state.transcriptionModel).toBe("whisper-large-v3-turbo-q5_0");
      await waitFor(() => expect(screen.getByRole("radio", { name: /Whisper large-v3-turbo/ })).toBeChecked());
    });

    it("removes Whisper and goes back to Whistle", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.whisperInstalled = true;
      backend.state.transcriptionModel = "whisper-large-v3-turbo-q5_0";
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Remove Whisper large-v3-turbo" }));
      await user.click(
        within(screen.getByRole("group", { name: "Remove Whisper large-v3-turbo" })).getByRole("button", { name: "Remove" }),
      );

      await waitFor(() => expect(screen.getByRole("radio", { name: /Whistle/ })).toBeChecked());
      expect(backend.state.whisperInstalled).toBe(false);
      expect(screen.getByRole("button", { name: "Download (574 MB)" })).toBeInTheDocument();
    });

    it("explains when transcription is unavailable", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.transcriptionUnavailable = "no Whistle model found";
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      expect(await screen.findByText("Transcription is unavailable: no Whistle model found.")).toBeInTheDocument();
    });
  });

  describe("first-run setup", () => {
    function firstRun(configure: (state: ReturnType<typeof fakeBackend>["state"]) => void = () => {}) {
      const backend = fakeBackend();
      backend.state.setupComplete = false;
      configure(backend.state);
      const user = userEvent.setup();
      render(<App />);
      return { backend, user };
    }

    it("shows the checklist until setup is finished", async () => {
      const { backend, user } = firstRun();

      expect(await screen.findByRole("heading", { level: 1, name: "Get set up" })).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Finish setup" }));

      expect(await screen.findByRole("heading", { name: "Start your first meeting" })).toBeInTheDocument();
      expect(backend.state.setupComplete).toBe(true);
    });

    it("includes a reminder about recording consent", async () => {
      firstRun();

      const item = (await screen.findByRole("heading", { level: 2, name: "Recording consent" })).closest("li");
      expect(item).not.toBeNull();
      expect(within(item as HTMLElement).getByText(CONSENT_NOTICE)).toBeInTheDocument();
    });

    it("asks for the microphone when macOS hasn't asked yet", async () => {
      const { user } = firstRun((state) => {
        state.micPermission = "notDetermined";
        state.systemAudioPermission = "notDetermined";
      });

      await user.click(await screen.findByRole("button", { name: "Allow microphone" }));

      expect(await screen.findByText("Allowed")).toBeInTheDocument();
    });

    it("links to Privacy settings when the microphone is blocked", async () => {
      const { backend, user } = firstRun((state) => {
        state.micPermission = "denied";
      });

      expect(await screen.findByText(/Turn on Earshot Notes in Privacy & Security > Microphone/)).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Open Microphone settings" }));

      expect(backend.state.openedPanes).toEqual(["microphone"]);
    });

    it("asks for system audio when macOS hasn't asked yet", async () => {
      const { user } = firstRun((state) => {
        state.systemAudioPermission = "notDetermined";
      });

      await user.click(await screen.findByRole("button", { name: "Allow system audio" }));

      const items = within(screen.getByRole("list"));
      expect(await items.findAllByText("Allowed")).toHaveLength(2);
    });

    it("links to Privacy settings when system audio is blocked", async () => {
      const { backend, user } = firstRun((state) => {
        state.systemAudioPermission = "notDetermined";
        state.systemAudioAnswer = "denied";
      });

      await user.click(await screen.findByRole("button", { name: "Allow system audio" }));
      expect(await screen.findByText(/System audio recording is off\. Turn on Earshot Notes in/)).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Open audio recording settings" }));

      expect(backend.state.openedPanes).toEqual(["systemAudio"]);
    });

    it("falls back to prompting without a status when macOS can't report it", async () => {
      const { user } = firstRun((state) => {
        state.systemAudioPermission = null;
        state.systemAudioAnswer = null;
      });

      await user.click(await screen.findByRole("button", { name: "Allow system audio" }));

      expect(await screen.findByText(/macOS asked for permission\. If you declined, turn on Earshot Notes in/)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Allow system audio" })).not.toBeInTheDocument();
    });

    it("sends you to settings to add a summary model", async () => {
      const { user } = firstRun();

      await user.click(await screen.findByRole("button", { name: "Set up summaries" }));

      expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
    });

    it("shows the summary model when one is installed", async () => {
      firstRun((state) => {
        state.model = testModel();
      });

      expect(await screen.findByText("Ready, using Gemma 4 E2B")).toBeInTheDocument();
    });

    it("can be shown again from settings", async () => {
      const backend = fakeBackend();
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));

      await user.click(await screen.findByRole("button", { name: "Show the setup checklist again" }));

      expect(await screen.findByRole("heading", { level: 1, name: "Get set up" })).toBeInTheDocument();
      expect(backend.state.setupComplete).toBe(false);
    });

    it("skips the system audio hint when permission is known to be granted", async () => {
      fakeBackend([seedMeeting(1, "Call")]);
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: /Call/ }));
      await user.click(await screen.findByRole("button", { name: "Start recording" }));
      const transcript = screen.getByRole("complementary", { name: "Transcript" });

      expect(await within(transcript).findByText(/Listening/)).toBeInTheDocument();
      expect(within(transcript).queryByText(/Not seeing the other side/)).not.toBeInTheDocument();
    });
  });
});
