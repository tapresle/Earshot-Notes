import { useState } from "react";
import { api, errorMessage } from "../../lib/api";

interface RecordingControlsProps {
  meetingId: number;
  /** The meeting currently recording, if any. */
  recordingMeetingId: number | null;
  onRecordingChange: (meetingId: number | null) => void;
}

export function RecordingControls({ meetingId, recordingMeetingId, onRecordingChange }: RecordingControlsProps) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isRecording = recordingMeetingId === meetingId;

  async function toggle() {
    setPending(true);
    setError(null);
    try {
      if (isRecording) {
        await api.stopRecording();
        onRecordingChange(null);
      } else {
        await api.startRecording(meetingId);
        onRecordingChange(meetingId);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setPending(false);
    }
  }

  const label = pending ? (isRecording ? "Stopping…" : "Starting…") : isRecording ? "Stop recording" : "Start recording";

  return (
    <div className="recording-controls">
      <button
        type="button"
        className={`button ${isRecording ? "danger" : "primary"}`}
        onClick={() => void toggle()}
        disabled={pending}
      >
        {label}
      </button>
      {isRecording && !pending && (
        <span className="recording-indicator" role="status">
          <span className="recording-dot" aria-hidden="true" />
          Recording your microphone
        </span>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
