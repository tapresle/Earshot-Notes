import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export interface CommandError {
  kind: "notFound" | "conflict" | "unavailable" | "internal";
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
};
