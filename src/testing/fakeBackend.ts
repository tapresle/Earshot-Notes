// Synthetic data only: these fixtures also render the committed screenshots.
import type {
  ChatMessage,
  MeetingDetail,
  ModelRecord,
  Summary,
  TranscriptSegment,
  UpdateCheck,
  VocabularyTerm,
} from "../lib/api";

type Args = Record<string, unknown>;

const WHISPER_ID = "whisper-large-v3-turbo-q5_0";
const WHISPER_BYTES = 574_041_195;

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
    chats: new Map<number, ChatMessage[]>(),
    asking: null as number | null,
    asked: [] as { meetingId: number; question: string }[],
    askError: null as unknown,
    chatCancelled: false,
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
    keepModelLoaded: "fiveMinutes" as string,
    launchAtLogin: { enabled: false, needsApproval: false, available: true },
    launchAtLoginError: null as unknown,
    recordingsSize: 0,
    shownRecordings: false,
    installed: [] as ModelRecord[],
    transcriptionUnavailable: null as string | null,
    transcriptionModelsError: null as unknown,
    transcriptionModel: "whistle",
    whisperInstalled: false,
    whisperPartialBytes: 0,
    whisperDownloading: false,
    setupComplete: true,
    micPermission: "authorized" as string,
    systemAudioPermission: "authorized" as string | null,
    systemAudioAnswer: "authorized" as string | null,
    openedPanes: [] as string[],
    vocabulary: [] as VocabularyTerm[],
    vocabularyUpdated: 0,
    vocabularyError: null as unknown,
    updateCheckEnabled: false,
    updateChecks: 0,
    updateCheck: { current: "0.1.6", latest: "0.1.6", newer: false, notes: "" } as UpdateCheck,
    updateCheckError: null as unknown,
    openedReleasePage: null as string | null,
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
    selected_model: () => state.model,
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
    transcription_models: () => {
      if (state.transcriptionModelsError) throw state.transcriptionModelsError;
      return {
        models: [
          {
            id: "whistle",
            displayName: "Whistle",
            description: "Fast and light, and works from the first launch.",
            bundled: true,
            sizeBytes: 0,
            installed: true,
            partialBytes: 0,
            download: null,
            huggingFace: null,
          },
          {
            id: WHISPER_ID,
            displayName: "Whisper",
            description:
              "More accurate, especially for the other side of calls and technical terms. Heavier: it stays loaded while the app is open, using about 1 GB of memory.",
            bundled: false,
            sizeBytes: WHISPER_BYTES,
            installed: state.whisperInstalled,
            partialBytes: state.whisperPartialBytes,
            download: state.whisperDownloading
              ? { phase: "downloading", received: state.whisperPartialBytes, total: WHISPER_BYTES }
              : null,
            huggingFace: {
              repo: "ggerganov/whisper.cpp",
              quantization: "Q5_0",
              revision: "98aa99a0a9db05ae2342309f5096248665f7cba3",
              variant: "large-v3-turbo-q5_0",
            },
          },
        ],
        selectedId: state.transcriptionModel,
        unavailable: state.transcriptionUnavailable,
      };
    },
    select_transcription_model: ({ modelId }) => {
      if (modelId === WHISPER_ID && !state.whisperInstalled) {
        throw { kind: "invalid", message: "download Whisper before choosing it" };
      }
      state.transcriptionModel = String(modelId);
    },
    download_transcription_model: () => {
      state.whisperDownloading = true;
    },
    remove_transcription_model: () => {
      state.whisperInstalled = false;
      state.transcriptionModel = "whistle";
    },
    input_devices: () => state.devices,
    input_device: () => state.inputDevice,
    vocabulary: () => state.vocabulary.map((t) => ({ ...t, heardAs: [...t.heardAs] })),
    set_vocabulary: ({ terms }) => {
      if (state.vocabularyError) throw state.vocabularyError;
      // Trims like the real command, which also validates.
      state.vocabulary = (terms as VocabularyTerm[]).map((t) => ({
        term: t.term.trim(),
        heardAs: t.heardAs.map((h) => h.trim()).filter(Boolean),
      }));
      return { terms: state.vocabulary, updated: state.vocabularyUpdated };
    },
    update_check_enabled: () => state.updateCheckEnabled,
    check_for_update: () => {
      if (!state.updateCheckEnabled) throw { kind: "unavailable", message: "checking for updates isn't available" };
      state.updateChecks++;
      if (state.updateCheckError) throw state.updateCheckError;
      return { ...state.updateCheck };
    },
    open_release_page: ({ version }) => {
      state.openedReleasePage = version as string;
    },
    launch_at_login: () => ({ ...state.launchAtLogin }),
    set_launch_at_login: ({ enabled }) => {
      if (state.launchAtLoginError) throw state.launchAtLoginError;
      state.launchAtLogin = { ...state.launchAtLogin, enabled: Boolean(enabled) };
      return { ...state.launchAtLogin };
    },
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
    keep_model_loaded: () => state.keepModelLoaded,
    set_keep_model_loaded: ({ keep }) => {
      state.keepModelLoaded = String(keep);
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
    select_model: ({ modelId }) => {
      const model = state.installed.find((m) => m.id === modelId);
      if (!model) throw { kind: "notFound", missing: "model", message: `model ${String(modelId)} doesn't exist` };
      state.model = model;
    },
    remove_model: ({ modelId }) => {
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
    chat_messages: ({ meetingId }) => state.chats.get(meetingId as number) ?? [],
    active_chat: () => state.asking,
    ask_meeting: ({ meetingId, question }) => {
      if (state.askError) throw state.askError;
      state.asking = meetingId as number;
      state.asked.push({ meetingId: meetingId as number, question: String(question) });
    },
    cancel_chat: () => {
      state.chatCancelled = true;
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

/** A saved question and its answer, with ids `id` and `id + 1`. */
export function chatExchange(
  meetingId: number,
  id: number,
  question: string,
  answer: string,
  sources: ChatMessage["sources"] = [],
): ChatMessage[] {
  const base = { meetingId, createdAt: 0 };
  return [
    { ...base, id, role: "user", bodyMarkdown: question, sources: [], modelId: null },
    { ...base, id: id + 1, role: "assistant", bodyMarkdown: answer, sources, modelId: "gemma-4-e2b-it-q4_k_m" },
  ];
}
