import { type Ref, useImperativeHandle, useRef, useState } from "react";
import { api } from "../../lib/api";
import { type SaveStatus, useAutosave } from "./useAutosave";

/** Lets a chat citation open the notes at the passage it cites. */
export interface NotepadHandle {
  /** Focuses the notes and selects `passage`, or puts the cursor at the start when it's no longer there. */
  select: (passage: string) => void;
}

interface NotepadProps {
  meetingId: number;
  initialMarkdown: string;
  ref?: Ref<NotepadHandle>;
}

const statusText: Record<SaveStatus, string> = {
  saved: "Saved",
  unsaved: "Editing",
  saving: "Saving…",
  error: "Couldn't save your notes. Keep typing to retry.",
};

export function Notepad({ meetingId, initialMarkdown, ref }: NotepadProps) {
  const [body, setBody] = useState(initialMarkdown);
  const { status, schedule, flush } = useAutosave((value) => api.updateNotes(meetingId, value));
  const textarea = useRef<HTMLTextAreaElement>(null);

  useImperativeHandle(
    ref,
    () => ({
      select(passage) {
        const element = textarea.current;
        if (!element) return;
        const start = Math.max(0, element.value.indexOf(passage));
        const end = element.value.startsWith(passage, start) ? start + passage.length : start;
        element.focus();
        element.setSelectionRange(start, end);
      },
    }),
    [],
  );

  return (
    <div className="notepad">
      <label htmlFor="notepad-body" className="sr-only">
        Meeting notes
      </label>
      <textarea
        ref={textarea}
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
