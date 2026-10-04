import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type TranscriptSegment } from "../../lib/api";
import { formatOffset } from "./formatOffset";

interface TranscriptPanelProps {
  meetingId: number | null;
  recording: boolean;
}

type LoadState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready" };

/** Merges segments by id and keeps them in timeline order. */
function merge(current: TranscriptSegment[], incoming: TranscriptSegment[]): TranscriptSegment[] {
  const byId = new Map(current.map((s) => [s.id, s]));
  for (const segment of incoming) byId.set(segment.id, segment);
  return [...byId.values()].sort((a, b) => a.startMs - b.startMs || a.id - b.id);
}

export function TranscriptPanel({ meetingId, recording }: TranscriptPanelProps) {
  return (
    <aside className="transcript-panel" aria-label="Transcript">
      <h2 className="pane-heading">Transcript</h2>
      {meetingId === null ? (
        <p className="empty-hint">Open a meeting to see its transcript.</p>
      ) : (
        <MeetingTranscript key={meetingId} meetingId={meetingId} recording={recording} />
      )}
    </aside>
  );
}

function MeetingTranscript({ meetingId, recording }: { meetingId: number; recording: boolean }) {
  const [segments, setSegments] = useState<TranscriptSegment[]>([]);
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    // Subscribe before loading so no segment slips between the two.
    api
      .onTranscriptSegment((segment) => {
        if (!cancelled && segment.meetingId === meetingId) setSegments((s) => merge(s, [segment]));
      })
      .then((stop) => (cancelled ? stop() : (unlisten = stop)))
      .then(() => api.listTranscript(meetingId))
      .then((loaded) => {
        if (cancelled) return;
        setSegments((s) => merge(s, loaded ?? []));
        setState({ kind: "ready" });
      })
      .catch((error) => !cancelled && setState({ kind: "error", message: errorMessage(error) }));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [meetingId, attempt]);

  useEffect(() => {
    if (recording) endRef.current?.scrollIntoView?.({ block: "end" });
  }, [segments.length, recording]);

  if (state.kind === "loading") return <p className="empty-hint">Loading transcript…</p>;
  if (state.kind === "error") {
    return (
      <div className="inline-error" role="alert">
        <p>{state.message}</p>
        <button
          type="button"
          className="button"
          onClick={() => {
            setState({ kind: "loading" });
            setAttempt((n) => n + 1);
          }}
        >
          Try again
        </button>
      </div>
    );
  }

  return (
    <>
      {segments.length === 0 && (
        <p className="empty-hint">
          {recording
            ? "Listening. Speech appears here a few seconds after it's spoken."
            : "No transcript yet. Start recording to transcribe this meeting."}
        </p>
      )}
      <ol className="transcript" aria-live="polite">
        {segments.map((segment) => (
          <li key={segment.id} className="transcript-segment">
            <span className={`speaker speaker-${segment.source}`}>
              {segment.source === "me" ? "Me" : "Them"}{" "}
              <time className="segment-time">{formatOffset(segment.startMs)}</time>
            </span>
            <p>{segment.text}</p>
          </li>
        ))}
      </ol>
      {recording && !segments.some((s) => s.source === "them") && (
        <p className="empty-hint transcript-hint">
          Not seeing the other side? macOS may be blocking system audio. Allow Earshot in System Settings &gt;
          Privacy &amp; Security &gt; Screen &amp; System Audio Recording.
        </p>
      )}
      <div ref={endRef} />
    </>
  );
}
