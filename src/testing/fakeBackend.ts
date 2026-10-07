// Synthetic data only: these fixtures also render the committed screenshots.
import type { MeetingDetail, ModelRecord, Summary, TranscriptSegment } from "../lib/api";

type Args = Record<string, unknown>;

/**
 * An in-memory stand-in for the Rust commands, shared by the tests and the
 * screenshot preview. `handle` answers a command the way `invoke` would.
 */
export function createFakeBackend(seed: MeetingDetail[] = [], segments: TranscriptSegment[] = []) {
  const meetings = new Map(seed.map((m) => [m.id, { ...m }]));
  let nextId = Math.max(0, ...seed.map((m) => m.id)) + 1;
  const notFound = { kind: "notFound", missing: "meeting", message: "meeting doesn't exist" };
  const state = {
    listError: null as unknown,
    queries: [] as (string | null)[],
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
    sensitivity: { me: "normal", them: "normal" } as Record<string, string>,
    keepAudio: false,
    recordingsSize: 0,
    shownRecordings: false,
    installed: [] as ModelRecord[],
    transcriptionError: null as unknown,
    setupComplete: true,
    micPermission: "authorized" as string,
    systemAudioPermission: "authorized" as string | null,
    systemAudioAnswer: "authorized" as string | null,
    openedPanes: [] as string[],
  };

  const handlers: Record<string, (args: Args) => unknown> = {
    list_meetings: ({ query, after, limit }) => {
      if (state.listError) throw state.listError;
      state.queries.push((query as string | null) ?? null);
      const cursor = after as { createdAt: number; id: number } | null;
      // Like the FTS5 search: every word must start a word in the title, notes, or transcript.
      const wordsOf = (text: string) => text.toLowerCase().split(/[^\p{L}\p{N}]+/u);
      const terms = wordsOf((query as string | null) ?? "").filter(Boolean);
      const matches = (m: MeetingDetail) => {
        const words = wordsOf(
          [m.title, m.notesMarkdown, ...segments.filter((s) => s.meetingId === m.id).map((s) => s.text)].join(" "),
        );
        return terms.every((term) => words.some((word) => word.startsWith(term)));
      };
      const newestFirst = [...meetings.values()]
        .filter(matches)
        .sort((a, b) => b.createdAt - a.createdAt || b.id - a.id)
        .filter((m) => !cursor || m.createdAt < cursor.createdAt || (m.createdAt === cursor.createdAt && m.id < cursor.id));
      const page = newestFirst.slice(0, limit as number);
      const last = page[page.length - 1];
      return {
        meetings: page.map(({ id, title, createdAt, startedAt, endedAt }) => ({ id, title, createdAt, startedAt, endedAt })),
        next: newestFirst.length > page.length ? { createdAt: last.createdAt, id: last.id } : null,
      };
    },
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
          quantization: "Q4_K_M",
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
      return { engine: "Whistle", path: "/Applications/Earshot Notes.app/Contents/Resources/models/whistle.cact", bundled: true };
    },
    input_devices: () => state.devices,
    input_device: () => state.inputDevice,
    keep_audio: () => state.keepAudio,
    set_keep_audio: ({ enabled }) => {
      state.keepAudio = Boolean(enabled);
    },
    recordings_size: () => state.recordingsSize,
    delete_all_recordings: () => {
      if (state.recording !== null) throw { kind: "conflict", message: "stop recording to delete recordings" };
      state.recordingsSize = 0;
      return 0;
    },
    show_recordings: () => {
      state.shownRecordings = true;
    },
    speech_sensitivity: () => ({ ...state.sensitivity }),
    set_speech_sensitivity: ({ source, level }) => {
      state.sensitivity[String(source)] = String(level);
    },
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
      const model = state.installed.find((m) => m.id === modelId);
      if (!model) throw { kind: "notFound", missing: "model", message: `model ${String(modelId)} doesn't exist` };
      state.model = model;
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

  async function handle(command: string, args?: unknown): Promise<unknown> {
    const handler = handlers[command];
    if (!handler) throw new Error(`the fake backend has no command ${command}`);
    return handler((args ?? {}) as Args);
  }

  return Object.assign(meetings, { state, handle });
}

export type FakeBackend = ReturnType<typeof createFakeBackend>;

export const GEMMA_SOURCE = {
  repo: "unsloth/gemma-4-E2B-it-GGUF",
  quantization: "Q4_K_M",
  revision: "0314792d7f1f7e229411f620751375812bb9faf2",
};

export function testModel(path = "/models/gemma.gguf"): ModelRecord {
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

export function savedSummary(meetingId: number, bodyMarkdown: string, isFinal = true): Summary {
  return { id: 1, meetingId, uptoMs: 754_000, isFinal, bodyMarkdown, modelId: "gemma-4-e2b-it-q4_k_m", createdAt: 0 };
}

export function segment(id: number, meetingId: number, source: "me" | "them", startMs: number, text: string): TranscriptSegment {
  return { id, meetingId, source, startMs, endMs: startMs + 2000, text, language: "en" };
}

export function seedMeeting(id: number, title: string, notesMarkdown = ""): MeetingDetail {
  const createdAt = Date.now() - id * 1000;
  return { id, title, createdAt, startedAt: null, endedAt: null, notesMarkdown, notesUpdatedAt: createdAt };
}
