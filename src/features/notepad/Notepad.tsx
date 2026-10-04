import { useState } from "react";
import { api } from "../../lib/api";
import { type SaveStatus, useAutosave } from "./useAutosave";

interface NotepadProps {
  meetingId: number;
  initialMarkdown: string;
}

const statusText: Record<SaveStatus, string> = {
  saved: "Saved",
  unsaved: "Editing",
  saving: "Saving…",
  error: "Couldn't save your notes. Keep typing to retry.",
};

export function Notepad({ meetingId, initialMarkdown }: NotepadProps) {
  const [body, setBody] = useState(initialMarkdown);
  const { status, schedule, flush } = useAutosave((value) => api.updateNotes(meetingId, value));

  return (
    <div className="notepad">
      <label htmlFor="notepad-body" className="sr-only">
        Meeting notes
      </label>
      <textarea
        id="notepad-body"
        className="notepad-body"
        value={body}
        placeholder="Write notes during the meeting. Markdown works."
        onChange={(event) => {
          setBody(event.target.value);
          schedule(event.target.value);
        }}
        onBlur={() => void flush()}
      />
      <p className={`save-status ${status === "error" ? "is-error" : ""}`} role="status">
        {statusText[status]}
      </p>
    </div>
  );
}
