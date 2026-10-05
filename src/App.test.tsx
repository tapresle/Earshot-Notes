import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { MeetingDetail, ModelRecord, Summary, TranscriptSegment } from "./lib/api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

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
    model: null as ModelRecord | null,
    summaries: new Map<number, Summary>(),
    summarizing: null as number | null,
    summarizeError: null as unknown,
    importError: null as unknown,
    cancelled: false,
    partialBytes: 0,
    downloading: false,
    downloadedId: null as string | null,
    removeError: null as unknown,
    openedModelPage: null as string | null,
    openModelPageError: null as unknown,
    downloadCancelled: false,
    exports: [] as { ids: number[] | null; folder: string }[],
    exportFailures: [] as { title: string; message: string }[],
    savedTo: null as string | null,
    devices: [
      { id: "builtin", name: "MacBook Pro Microphone", isDefault: true },
      { id: "usb", name: "USB Podcast Mic", isDefault: false },
    ],
    inputDevice: null as string | null,
    installed: [] as ModelRecord[],
    transcriptionError: null as unknown,
    setupComplete: true,
    micPermission: "authorized" as string,
    systemAudioPermission: "authorized" as string | null,
    systemAudioAnswer: "authorized" as string | null,
    openedPanes: [] as string[],
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
    summary_model: () => state.model,
    available_models: () => [
      {
        id: "gemma-4-e2b-it-q4_k_m",
        displayName: "Gemma 4 E2B",
        description: "Fast and light. A good fit for most meetings.",
        fileName: "gemma-4-E2B-it-Q4_K_M.gguf",
        huggingFace: GEMMA_SOURCE,
        sizeBytes: 3_106_738_272,
        recommended: true,
        installed: state.model !== null,
        partialBytes: state.partialBytes,
        download: state.downloading ? { phase: "downloading", received: state.partialBytes, total: 3_106_738_272 } : null,
      },
      {
        id: "qwen3-8b-q4_k_m",
        displayName: "Qwen3 8B",
        description: "High-quality summaries. Slower, and uses more memory.",
        fileName: "Qwen3-8B-Q4_K_M.gguf",
        huggingFace: {
          repo: "unsloth/Qwen3-8B-GGUF",
          fileName: "Qwen3-8B-Q4_K_M.gguf",
          revision: "a6adef130ffb23ddaf1a62fec9dced968c9bc482",
        },
        sizeBytes: 5_027_784_512,
        recommended: false,
        installed: false,
        partialBytes: 0,
        download: null,
      },
    ],
    download_model: ({ modelId }) => {
      state.downloading = true;
      state.downloadedId = String(modelId);
    },
    export_file_name: ({ meetingId }) => `2026-10-03-meeting-${meetingId}.md`,
    export_meeting: ({ path }) => {
      state.savedTo = String(path);
      return String(path);
    },
    export_meetings: ({ meetingIds, folder }) => {
      const ids = (meetingIds as number[] | null) ?? [...meetings.keys()];
      state.exports.push({ ids: meetingIds as number[] | null, folder: String(folder) });
      const failed = state.exportFailures;
      return { written: ids.slice(failed.length).map((id) => `${folder}/${id}.md`), failed };
    },
    transcription_model: () => {
      if (state.transcriptionError) throw state.transcriptionError;
      return { engine: "Whistle", path: "/Applications/Earshot.app/Contents/Resources/models/whistle.cact", bundled: true };
    },
    input_devices: () => state.devices,
    input_device: () => state.inputDevice,
    set_input_device: ({ deviceId }) => {
      state.inputDevice = (deviceId as string | null) ?? null;
    },
    installed_models: () => ({
      models: state.installed.map((m) => ({ ...m, huggingFace: m.id === "gemma-4-e2b-it-q4_k_m" ? GEMMA_SOURCE : null })),
      selectedId: state.model?.id ?? null,
    }),
    open_model_page: ({ modelId }) => {
      if (state.openModelPageError) throw state.openModelPageError;
      state.openedModelPage = String(modelId);
    },
    select_summary_model: ({ modelId }) => {
      state.model = state.installed.find((m) => m.id === modelId) ?? null;
    },
    remove_summary_model: ({ modelId }) => {
      if (state.removeError) throw state.removeError;
      state.installed = state.installed.filter((m) => m.id !== modelId);
      if (state.model?.id === modelId) state.model = state.installed[0] ?? null;
    },
    setup_complete: () => state.setupComplete,
    set_setup_complete: ({ complete }) => {
      state.setupComplete = Boolean(complete);
    },
    mic_permission: () => state.micPermission,
    request_mic_permission: () => {
      state.micPermission = "authorized";
      return state.micPermission;
    },
    system_audio_permission: () => state.systemAudioPermission,
    request_system_audio_permission: () => {
      state.systemAudioPermission = state.systemAudioAnswer;
      return state.systemAudioPermission;
    },
    open_privacy_settings: ({ pane }) => {
      state.openedPanes.push(String(pane));
    },
    cancel_download: () => {
      state.downloadCancelled = true;
      return true;
    },
    import_model: ({ path }) => {
      if (state.importError) throw state.importError;
      state.model = testModel(String(path));
      return state.model;
    },
    latest_summary: ({ meetingId }) => state.summaries.get(meetingId as number) ?? null,
    active_summary: () => state.summarizing,
    summarize_meeting: ({ meetingId }) => {
      if (state.summarizeError) throw state.summarizeError;
      state.summarizing = meetingId as number;
    },
    cancel_summary: () => {
      state.cancelled = true;
      return true;
    },
  };

  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => handlers[command]((args ?? {}) as Args));
  return Object.assign(meetings, { state });
}

const GEMMA_SOURCE = {
  repo: "unsloth/gemma-4-E2B-it-GGUF",
  fileName: "gemma-4-E2B-it-Q4_K_M.gguf",
  revision: "0314792d7f1f7e229411f620751375812bb9faf2",
};

function testModel(path = "/models/gemma.gguf"): ModelRecord {
  return {
    id: "gemma-4-e2b-it-q4_k_m",
    displayName: "Gemma 4 E2B",
    filePath: path,
    sizeBytes: 3_106_738_272,
    sha256: "abc",
    source: "import",
    sourceUrl: null,
    version: null,
    minRuntime: null,
    addedAt: 0,
  };
}

function savedSummary(meetingId: number, bodyMarkdown: string, isFinal = true): Summary {
  return { id: 1, meetingId, uptoMs: 754_000, isFinal, bodyMarkdown, modelId: "gemma-4-e2b-it-q4_k_m", createdAt: 0 };
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
    const backend = fakeBackend([seedMeeting(1, "Call")]);
    backend.state.systemAudioPermission = null;
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
      expect(qwen).toHaveTextContent("unsloth/Qwen3-8B-GGUF/Qwen3-8B-Q4_K_M.gguf - a6adef1");
      await user.click(within(qwen).getByRole("button", { name: "unsloth/Qwen3-8B-GGUF/Qwen3-8B-Q4_K_M.gguf" }));

      expect(backend.state.openedModelPage).toBe("qwen3-8b-q4_k_m");
    });

    it("explains when a model's page can't be opened", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.openModelPageError = { kind: "unavailable", message: "couldn't open the model's Hugging Face page" };
      const user = await openMeeting("Sync");
      await user.click(screen.getByRole("tab", { name: "Summary" }));

      await user.click(await screen.findByRole("button", { name: "unsloth/gemma-4-E2B-it-GGUF/gemma-4-E2B-it-Q4_K_M.gguf" }));

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

    it("exports all meetings to a folder", async () => {
      const backend = fakeBackend([seedMeeting(1, "One"), seedMeeting(2, "Two")]);
      vi.mocked(open).mockResolvedValue("/Users/me/Exports");
      const user = userEvent.setup();
      render(<App />);

      await user.click(await within(sidebar()).findByRole("button", { name: "Export all" }));

      expect(await within(sidebar()).findByText(/Exported 2 meetings to/)).toBeInTheDocument();
      expect(backend.state.exports).toEqual([{ ids: null, folder: "/Users/me/Exports" }]);
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

      await user.click(await within(sidebar()).findByRole("button", { name: "Export all" }));

      expect(await within(sidebar()).findByText(/Exported 1 meeting to/)).toBeInTheDocument();
      expect(within(sidebar()).getByText("Couldn't export 1 meeting:")).toBeInTheDocument();
      expect(within(sidebar()).getByText("Two: permission denied")).toBeInTheDocument();
    });

    it("hides export when there are no meetings", async () => {
      fakeBackend();
      render(<App />);

      await screen.findByRole("heading", { name: "Start your first meeting" });
      expect(screen.queryByRole("button", { name: "Export all" })).not.toBeInTheDocument();
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
      expect(screen.getAllByText("Selected")).toHaveLength(1);
      expect(gemmaChoice.closest("label")).toHaveTextContent("Selected");
      await user.click(screen.getByRole("radio", { name: /My fine-tune/ }));

      await waitFor(() => expect(backend.state.model?.id).toBe("local-abc"));
      expect(screen.getByRole("radio", { name: /My fine-tune/ })).toBeChecked();
      expect(screen.getAllByText("Selected")).toHaveLength(1);
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

      expect(gemmaCard).toHaveTextContent("unsloth/gemma-4-E2B-it-GGUF/gemma-4-E2B-it-Q4_K_M.gguf - 0314792");
      expect(within(otherCard).queryByRole("button", { name: /GGUF/ })).not.toBeInTheDocument();
      await user.click(within(gemmaCard).getByRole("button", { name: "unsloth/gemma-4-E2B-it-GGUF/gemma-4-E2B-it-Q4_K_M.gguf" }));

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

      await waitFor(() => expect(screen.queryByRole("radio")).not.toBeInTheDocument());
      expect(
        within(await screen.findByRole("group", { name: "Gemma 4 E2B" })).getByRole("button", { name: "Download (3.1 GB)" }),
      ).toBeInTheDocument();
    });

    it("treats a model that's already gone as removed", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.installed = [testModel()];
      backend.state.model = testModel();
      backend.state.removeError = { kind: "notFound", message: "summary model gemma-4-e2b-it-q4_k_m doesn't exist" };
      const user = userEvent.setup();
      render(<App />);
      await user.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
      await user.click(await screen.findByRole("button", { name: "Remove Gemma 4 E2B" }));
      backend.state.installed = [];
      backend.state.model = null;

      await user.click(within(screen.getByRole("group", { name: "Remove Gemma 4 E2B" })).getByRole("button", { name: "Remove" }));

      await waitFor(() => expect(screen.queryByRole("radio")).not.toBeInTheDocument());
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
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

    it("shows which transcription model is running", async () => {
      await openSettings();

      expect(await screen.findByText("Whistle, bundled with the app. Runs on this Mac.")).toBeInTheDocument();
    });

    it("explains when transcription is unavailable", async () => {
      const backend = fakeBackend([seedMeeting(1, "Sync")]);
      backend.state.transcriptionError = { kind: "unavailable", message: "transcription is unavailable: no Whistle model found" };
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

      await user.click(await screen.findByRole("button", { name: "Open Microphone settings" }));

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
      expect(await screen.findByText(/System audio recording is off/)).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Open audio recording settings" }));

      expect(backend.state.openedPanes).toEqual(["systemAudio"]);
    });

    it("falls back to prompting without a status when macOS can't report it", async () => {
      const { user } = firstRun((state) => {
        state.systemAudioPermission = null;
        state.systemAudioAnswer = null;
      });

      await user.click(await screen.findByRole("button", { name: "Allow system audio" }));

      expect(await screen.findByText(/macOS asked for permission/)).toBeInTheDocument();
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
