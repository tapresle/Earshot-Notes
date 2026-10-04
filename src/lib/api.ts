import { invoke } from "@tauri-apps/api/core";

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

export interface CommandError {
  kind: "notFound" | "internal";
  message: string;
}

function isCommandError(error: unknown): error is CommandError {
  return typeof error === "object" && error !== null && "kind" in error && "message" in error;
}

/** A user-facing message for any error thrown by a command. */
export function errorMessage(error: unknown): string {
  if (isCommandError(error)) {
    return error.kind === "notFound" ? "That meeting no longer exists." : "The app couldn't read or write your meetings.";
  }
  return "Something went wrong.";
}

export const api = {
  createMeeting: (title = "") => invoke<Meeting>("create_meeting", { title }),
  listMeetings: () => invoke<Meeting[]>("list_meetings"),
  getMeeting: (id: number) => invoke<MeetingDetail>("get_meeting", { id }),
  renameMeeting: (id: number, title: string) => invoke<string>("rename_meeting", { id, title }),
  updateNotes: (id: number, bodyMarkdown: string) => invoke<number>("update_notes", { id, bodyMarkdown }),
  deleteMeeting: (id: number) => invoke<void>("delete_meeting", { id }),
};
