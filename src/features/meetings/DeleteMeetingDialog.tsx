import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../../lib/api";

interface DeleteMeetingDialogProps {
  meetingId: number;
  title: string;
  onDeleted: (id: number) => void;
  /** Called when the dialog closes without deleting: Cancel or Escape. */
  onClose: () => void;
}

/**
 * Asks before deleting a meeting. A modal <dialog> keeps the rest of the
 * window inert while it's open, and Escape cancels it.
 */
export function DeleteMeetingDialog({ meetingId, title, onDeleted, onClose }: DeleteMeetingDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const element = dialog.current;
    // React's development double run of effects would otherwise open it twice.
    if (element && !element.open) element.showModal();
    // Deleting can't be undone, so the safe choice has focus.
    cancel.current?.focus();
  }, []);

  async function confirm() {
    setDeleting(true);
    setError(null);
    try {
      await api.deleteMeeting(meetingId);
      onDeleted(meetingId);
    } catch (e) {
      setError(errorMessage(e));
      setDeleting(false);
    }
  }

  return (
    <dialog
      ref={dialog}
      className="confirm-dialog"
      aria-labelledby="delete-meeting-title"
      aria-describedby="delete-meeting-detail"
      onClose={onClose}
      // Escape waits while the delete is in progress.
      onCancel={(event) => deleting && event.preventDefault()}
    >
      <h2 id="delete-meeting-title">Delete “{title}”?</h2>
      <p id="delete-meeting-detail">Its notes, transcript, summary, and chat are deleted too. You can't undo this.</p>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
      <div className="dialog-actions">
        <button type="button" className="button" ref={cancel} onClick={() => dialog.current?.close()} disabled={deleting}>
          Cancel
        </button>
        <button type="button" className="button danger" onClick={() => void confirm()} disabled={deleting}>
          {deleting ? "Deleting…" : "Delete meeting"}
        </button>
      </div>
    </dialog>
  );
}
