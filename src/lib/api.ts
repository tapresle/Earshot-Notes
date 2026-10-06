import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

export interface Meeting {
  id: number;
  title: string;
  /** Unix milliseconds. */
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
}

/** The position of the last meeting on a page. The next page starts after it. */
export interface MeetingCursor {
  createdAt: number;
  id: number;
}

export interface MeetingPage {
  meetings: Meeting[];
  /** Where the next page starts, or null on the last page. */
  next: MeetingCursor | null;
}

/** How many meetings the sidebar loads at a time. */
export const MEETING_PAGE_SIZE = 50;

export interface MeetingDetail extends Meeting {
  notesMarkdown: string;
  notesUpdatedAt: number;
}

export interface TranscriptSegment {
  id: number;
  meetingId: number;
  source: "me" | "them";
  /** Milliseconds from the start of the meeting. */
  startMs: number;
  endMs: number;
  text: string;
  language: string | null;
}

export type Source = TranscriptSegment["source"];

/** How readily quiet audio counts as speech. Changes apply to the next recording. */
export type Sensitivity = "low" | "normal" | "high";

export interface SpeechSensitivity {
  me: Sensitivity;
  them: Sensitivity;
}

export interface SourceProblem {
  source: Source;
  message: string;
}

export interface StartReport {
  /** Streams that couldn't start. Recording continues with the others. */
  unavailable: SourceProblem[];
}

export interface RecordingIssue extends SourceProblem {
  meetingId: number;
}

export interface Summary {
  id: number;
  meetingId: number;
  /** Transcript covered, in milliseconds from the start of the meeting. */
  uptoMs: number;
  /** False when the meeting was still recording. */
  isFinal: boolean;
  bodyMarkdown: string;
  modelId: string;
  createdAt: number;
}

export interface ModelRecord {
  id: string;
  displayName: string;
  filePath: string;
  sizeBytes: number;
  sha256: string;
  source: "import" | "download";
  sourceUrl: string | null;
  version: string | null;
  minRuntime: string | null;
  addedAt: number;
}

/** Exactly which file a model is on Hugging Face. */
export interface HuggingFaceSource {
  repo: string;
  /** The quantization of the weights, such as `Q4_K_M`. */
  quantization: string;
  revision: string;
}

export type DownloadPhase = { phase: "downloading"; received: number; total: number } | { phase: "verifying" };

export interface AvailableModel {
  id: string;
  displayName: string;
  /** One line that helps someone choose between models. */
  description: string;
  fileName: string;
  sizeBytes: number;
  huggingFace: HuggingFaceSource;
  recommended: boolean;
  installed: boolean;
  /** Bytes saved by an earlier, unfinished download. */
  partialBytes: number;
  /** Progress, while this model is downloading. */
  download: DownloadPhase | null;
}

export type DownloadProgress = DownloadPhase & { modelId: string };

export interface DownloadFailed {
  modelId: string;
  message: string;
  cancelled: boolean;
}

export type SummaryStage = { stage: "loading" } | { stage: "reading"; part: number; parts: number } | { stage: "writing" };

export type SummaryProgress = SummaryStage & { meetingId: number };

export interface SummaryText {
  meetingId: number;
  text: string;
}

export interface SummaryDone {
  meetingId: number;
  summary: Summary;
  /** The title the summary gave a meeting that was still untitled, if any. */
  title: string | null;
}

export interface SummaryFailed {
  meetingId: number;
  message: string;
  cancelled: boolean;
}

export interface ExportReport {
  written: string[];
  failed: { title: string; message: string }[];
}

export interface TranscriptionModel {
  engine: string;
  path: string;
  /** False when a model in the user's models folder replaced the bundled one. */
  bundled: boolean;
}

export interface InputDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export type InstalledModel = ModelRecord & {
  /** Where the model comes from, when it's one the app knows. */
  huggingFace: HuggingFaceSource | null;
};

export interface InstalledModels {
  models: InstalledModel[];
  selectedId: string | null;
}

export type Permission = "notDetermined" | "restricted" | "denied" | "authorized";

export type PrivacyPane = "microphone" | "systemAudio";

export interface CommandError {
  kind: "notFound" | "conflict" | "unavailable" | "invalid" | "internal";
  message: string;
  /** What a `notFound` error couldn't find. */
  missing?: "meeting" | "model";
}

export function isCommandError(error: unknown): error is CommandError {
  return typeof error === "object" && error !== null && "kind" in error && "message" in error;
}

/** A user-facing message for any error thrown by a command. */
export function errorMessage(error: unknown): string {
  if (!isCommandError(error)) return "Something went wrong.";
  switch (error.kind) {
    case "notFound":
      switch (error.missing) {
        case "meeting":
          return "That meeting no longer exists.";
        case "model":
          return "That summary model is no longer installed.";
        default:
          return sentence(error.message);
      }
    case "conflict":
    case "unavailable":
    case "invalid":
      // These messages are written for people and explain what to do.
      return sentence(error.message);
    case "internal":
      return "The app couldn't read or write your meetings.";
  }
}

function sentence(message: string): string {
  return `${message.charAt(0).toUpperCase()}${message.slice(1)}.`;
}

export const api = {
  createMeeting: (title = "") => invoke<Meeting>("create_meeting", { title }),
  /**
   * One page of meetings, newest first. With a search query, only meetings whose title, notes, or
   * transcript contain every word. Pass the previous page's `next` to get the following page.
   */
  listMeetings: (query = "", after: MeetingCursor | null = null, limit = MEETING_PAGE_SIZE) =>
    invoke<MeetingPage>("list_meetings", { query: query.trim() || null, after, limit }),
  getMeeting: (id: number) => invoke<MeetingDetail>("get_meeting", { id }),
  renameMeeting: (id: number, title: string) => invoke<string>("rename_meeting", { id, title }),
  updateNotes: (id: number, bodyMarkdown: string) => invoke<number>("update_notes", { id, bodyMarkdown }),
  deleteMeeting: (id: number) => invoke<void>("delete_meeting", { id }),
  startRecording: (meetingId: number) => invoke<StartReport>("start_recording", { meetingId }),
  stopRecording: () => invoke<number | null>("stop_recording"),
  activeRecording: () => invoke<number | null>("active_recording"),
  listTranscript: (meetingId: number) => invoke<TranscriptSegment[]>("list_transcript", { meetingId }),
  onTranscriptSegment: (handler: (segment: TranscriptSegment) => void): Promise<UnlistenFn> =>
    listen<TranscriptSegment>("transcript:segment", (event) => handler(event.payload)),
  onRecordingIssue: (handler: (issue: RecordingIssue) => void): Promise<UnlistenFn> =>
    listen<RecordingIssue>("recording:issue", (event) => handler(event.payload)),

  summarizeMeeting: (meetingId: number) => invoke<void>("summarize_meeting", { meetingId }),
  cancelSummary: () => invoke<boolean>("cancel_summary"),
  activeSummary: () => invoke<number | null>("active_summary"),
  latestSummary: (meetingId: number) => invoke<Summary | null>("latest_summary", { meetingId }),
  summaryModel: () => invoke<ModelRecord | null>("summary_model"),
  availableModels: () => invoke<AvailableModel[]>("available_models"),
  downloadModel: (modelId: string) => invoke<void>("download_model", { modelId }),
  cancelDownload: () => invoke<boolean>("cancel_download"),
  onDownloadProgress: (handler: (event: DownloadProgress) => void): Promise<UnlistenFn> =>
    listen<DownloadProgress>("model:download-progress", (event) => handler(event.payload)),
  onDownloadDone: (handler: (event: { model: ModelRecord }) => void): Promise<UnlistenFn> =>
    listen<{ model: ModelRecord }>("model:download-done", (event) => handler(event.payload)),
  onDownloadFailed: (handler: (event: DownloadFailed) => void): Promise<UnlistenFn> =>
    listen<DownloadFailed>("model:download-failed", (event) => handler(event.payload)),
  importModel: (path: string) => invoke<ModelRecord>("import_model", { path }),
  /** Asks the user for a .gguf file. Resolves to null if they cancel. */
  pickModelFile: async (): Promise<string | null> => {
    const path = await open({ multiple: false, directory: false, filters: [{ name: "GGUF model", extensions: ["gguf"] }] });
    return typeof path === "string" ? path : null;
  },
  exportFileName: (meetingId: number) => invoke<string>("export_file_name", { meetingId }),
  exportMeeting: (meetingId: number, path: string) => invoke<string>("export_meeting", { meetingId, path }),
  /** Exports the given meetings, or all of them when `meetingIds` is null. */
  exportMeetings: (meetingIds: number[] | null, folder: string) =>
    invoke<ExportReport>("export_meetings", { meetingIds, folder }),
  /** Asks where to save one file. Resolves to null if the user cancels. */
  pickSavePath: async (defaultName: string): Promise<string | null> =>
    (await save({ defaultPath: defaultName, filters: [{ name: "Markdown", extensions: ["md"] }] })) ?? null,
  /** Asks for a folder. Resolves to null if the user cancels. */
  pickFolder: async (): Promise<string | null> => {
    const path = await open({ directory: true, multiple: false });
    return typeof path === "string" ? path : null;
  },
  transcriptionModel: () => invoke<TranscriptionModel>("transcription_model"),
  inputDevices: () => invoke<InputDevice[]>("input_devices"),
  inputDevice: () => invoke<string | null>("input_device"),
  setInputDevice: (deviceId: string | null) => invoke<void>("set_input_device", { deviceId }),
  keepAudio: () => invoke<boolean>("keep_audio"),
  setKeepAudio: (enabled: boolean) => invoke<void>("set_keep_audio", { enabled }),
  /** The total size of the kept recordings, in bytes. */
  recordingsSize: () => invoke<number>("recordings_size"),
  /** Deletes every kept recording and resolves to the size left. Refused while recording. */
  deleteAllRecordings: () => invoke<number>("delete_all_recordings"),
  showRecordings: () => invoke<void>("show_recordings"),
  speechSensitivity: () => invoke<SpeechSensitivity>("speech_sensitivity"),
  setSpeechSensitivity: (source: Source, level: Sensitivity) =>
    invoke<void>("set_speech_sensitivity", { source, level }),
  installedModels: () => invoke<InstalledModels>("installed_models"),
  selectSummaryModel: (modelId: string) => invoke<void>("select_summary_model", { modelId }),
  openModelPage: (modelId: string) => invoke<void>("open_model_page", { modelId }),
  removeSummaryModel: (modelId: string) => invoke<void>("remove_summary_model", { modelId }),
  micPermission: () => invoke<Permission>("mic_permission"),
  requestMicPermission: () => invoke<Permission>("request_mic_permission"),
  /** Null when macOS doesn't let the app check. */
  systemAudioPermission: () => invoke<Permission | null>("system_audio_permission"),
  /** Shows the macOS prompt. Null when the answer can't be read. */
  requestSystemAudioPermission: () => invoke<Permission | null>("request_system_audio_permission"),
  openPrivacySettings: (pane: PrivacyPane) => invoke<void>("open_privacy_settings", { pane }),
  setupComplete: () => invoke<boolean>("setup_complete"),
  setSetupComplete: (complete: boolean) => invoke<void>("set_setup_complete", { complete }),
  onSummaryProgress: (handler: (event: SummaryProgress) => void): Promise<UnlistenFn> =>
    listen<SummaryProgress>("summary:progress", (event) => handler(event.payload)),
  onSummaryText: (handler: (event: SummaryText) => void): Promise<UnlistenFn> =>
    listen<SummaryText>("summary:text", (event) => handler(event.payload)),
  onSummaryDone: (handler: (event: SummaryDone) => void): Promise<UnlistenFn> =>
    listen<SummaryDone>("summary:done", (event) => handler(event.payload)),
  onSummaryFailed: (handler: (event: SummaryFailed) => void): Promise<UnlistenFn> =>
    listen<SummaryFailed>("summary:failed", (event) => handler(event.payload)),
};
