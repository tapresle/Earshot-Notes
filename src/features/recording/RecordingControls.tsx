import { useEffect, useEffectEvent, useRef, useState } from "react";
import { api, errorMessage, type Source, type SourceProblem } from "../../lib/api";
import { CONSENT_NOTICE } from "./consent";

interface RecordingControlsProps {
  meetingId: number;
  /** The meeting currently recording, if any. */
  recordingMeetingId: number | null;
  onRecordingChange: (meetingId: number | null) => void;
  /** Start recording once, as soon as the meeting opens, unless another meeting is recording. */
  autoStart?: boolean;
  /** Called when the automatic start is attempted, so it happens only once. */
  onAutoStart?: () => void;
}

const sourceName: Record<Source, string> = {
  me: "your microphone",
  them: "system audio",
};

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function RecordingControls({
  meetingId,
  recordingMeetingId,
  onRecordingChange,
  autoStart = false,
  onAutoStart,
}: RecordingControlsProps) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [problems, setProblems] = useState<SourceProblem[]>([]);
  const isRecording = recordingMeetingId === meetingId;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    api
      .onRecordingIssue((issue) => {
        if (issue.meetingId !== meetingId) return;
        setProblems((current) => [...current.filter((p) => p.source !== issue.source), issue]);
      })
      .then((stop) => (cancelled ? stop() : (unlisten = stop)))
      .catch((e) => console.error("couldn't subscribe to recording issues", e));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [meetingId]);

  // Starts like clicking Start recording, so it shows the same progress and errors.
  // The ref keeps React's development double run of effects from starting twice.
  const autoStarted = useRef(false);
  const startAutomatically = useEffectEvent(() => {
    if (autoStarted.current) return;
    autoStarted.current = true;
    onAutoStart?.();
    if (recordingMeetingId === null) void toggle();
  });
  useEffect(() => {
    if (autoStart) startAutomatically();
  }, [autoStart]);

  async function toggle() {
    setPending(true);
    setError(null);
    try {
      if (isRecording) {
        await api.stopRecording();
        setProblems([]);
        onRecordingChange(null);
      } else {
        const report = await api.startRecording(meetingId);
        setProblems(report.unavailable);
        onRecordingChange(meetingId);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setPending(false);
    }
  }

  const missing = new Set(problems.map((p) => p.source));
  const capturing = (["me", "them"] as const).filter((s) => !missing.has(s)).map((s) => sourceName[s]);
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
          {capturing.length > 0 ? `Recording ${capturing.join(" and ")}` : "Recording stopped working"}
        </span>
      )}
      {!isRecording && !pending && (
        <>
          <p className="recording-hint">
            Use headphones for the best transcript. With speakers, your microphone also picks up the other side.
          </p>
          <p className="recording-hint">{CONSENT_NOTICE}</p>
        </>
      )}
      {isRecording &&
        problems.map((problem) => (
          <p key={problem.source} className="field-warning" role="alert">
            {capitalize(problem.message)}.
          </p>
        ))}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
