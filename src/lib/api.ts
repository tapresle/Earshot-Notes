import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

export interface Meeting {
  id: number;
  title: string;
  /** Unix milliseconds. */
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
}

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

export interface RecommendedModel {
  displayName: string;
  fileName: string;
  sizeBytes: number;
  url: string;
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
}

export interface SummaryFailed {
  meetingId: number;
  message: string;
  cancelled: boolean;
}

export interface CommandError {
  kind: "notFound" | "conflict" | "unavailable" | "invalid" | "internal";
  message: string;
}

function isCommandError(error: unknown): error is CommandError {
  return typeof error === "object" && error !== null && "kind" in error && "message" in error;
}

/** A user-facing message for any error thrown by a command. */
export function errorMessage(error: unknown): string {
  if (!isCommandError(error)) return "Something went wrong.";
  switch (error.kind) {
    case "notFound":
      return "That meeting no longer exists.";
    case "conflict":
    case "unavailable":
    case "invalid":
      // These messages are written for people and explain what to do.
      return `${error.message.charAt(0).toUpperCase()}${error.message.slice(1)}.`;
    case "internal":
      return "The app couldn't read or write your meetings.";
  }
}

export const api = {
  createMeeting: (title = "") => invoke<Meeting>("create_meeting", { title }),
  listMeetings: () => invoke<Meeting[]>("list_meetings"),
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
  recommendedModel: () => invoke<RecommendedModel>("recommended_model"),
  importModel: (path: string) => invoke<ModelRecord>("import_model", { path }),
  /** Asks the user for a .gguf file. Resolves to null if they cancel. */
  pickModelFile: async (): Promise<string | null> => {
    const path = await open({ multiple: false, directory: false, filters: [{ name: "GGUF model", extensions: ["gguf"] }] });
    return typeof path === "string" ? path : null;
  },
  onSummaryProgress: (handler: (event: SummaryProgress) => void): Promise<UnlistenFn> =>
    listen<SummaryProgress>("summary:progress", (event) => handler(event.payload)),
  onSummaryText: (handler: (event: SummaryText) => void): Promise<UnlistenFn> =>
    listen<SummaryText>("summary:text", (event) => handler(event.payload)),
  onSummaryDone: (handler: (event: SummaryDone) => void): Promise<UnlistenFn> =>
    listen<SummaryDone>("summary:done", (event) => handler(event.payload)),
  onSummaryFailed: (handler: (event: SummaryFailed) => void): Promise<UnlistenFn> =>
    listen<SummaryFailed>("summary:failed", (event) => handler(event.payload)),
};
