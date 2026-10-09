import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type TranscriptSegment } from "../../lib/api";
import { formatOffset } from "./formatOffset";

/** Speaker labels, matching the labels in exports and summary prompts. */
const speakerLabel: Record<TranscriptSegment["source"], string> = {
  me: "Me",
  them: "Meeting Participants",
};

interface TranscriptPanelProps {
  meetingId: number | null;
  recording: boolean;
  onOpenSettings: () => void;
}

type LoadState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready" };

/** Merges segments by id, leaves out removed ones, and keeps them in timeline order. */
function merge(
  current: TranscriptSegment[],
  incoming: TranscriptSegment[],
  removed: ReadonlySet<number>,
): TranscriptSegment[] {
  const byId = new Map(current.map((s) => [s.id, s]));
  for (const segment of incoming) byId.set(segment.id, segment);
  return [...byId.values()]
    .filter((s) => !removed.has(s.id))
    .sort((a, b) => a.startMs - b.startMs || a.id - b.id);
}

export function TranscriptPanel({ meetingId, recording, onOpenSettings }: TranscriptPanelProps) {
  return (
    <aside className="transcript-panel" aria-label="Transcript">
      <h2 className="pane-heading">Transcript</h2>
      {meetingId === null ? (
        <p className="empty-hint">Open a meeting to see its transcript.</p>
      ) : (
        <MeetingTranscript
          key={meetingId}
          meetingId={meetingId}
          recording={recording}
          onOpenSettings={onOpenSettings}
        />
      )}
    </aside>
  );
}

function MeetingTranscript({
  meetingId,
  recording,
  onOpenSettings,
}: {
  meetingId: number;
  recording: boolean;
  onOpenSettings: () => void;
}) {
  const [segments, setSegments] = useState<TranscriptSegment[]>([]);
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [systemAudioAllowed, setSystemAudioAllowed] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    const stops: (() => void)[] = [];
    // Echo found after it was shown is removed. Remembered, so a transcript
    // loaded before the removal can't bring it back.
    const removed = new Set<number>();

    // Subscribe before loading so no segment slips between the two.
    Promise.all([
      api.onTranscriptSegment((segment) => {
        if (!cancelled && segment.meetingId === meetingId) setSegments((s) => merge(s, [segment], removed));
      }),
      api.onTranscriptRemoved((event) => {
        if (cancelled || event.meetingId !== meetingId) return;
        event.ids.forEach((id) => removed.add(id));
        setSegments((s) => merge(s, [], removed));
      }),
    ])
      .then((unlisten) => (cancelled ? unlisten.forEach((stop) => stop()) : stops.push(...unlisten)))
      .then(() => api.listTranscript(meetingId))
      .then((loaded) => {
        if (cancelled) return;
        setSegments((s) => merge(s, loaded ?? [], removed));
        setState({ kind: "ready" });
      })
      .catch((error) => !cancelled && setState({ kind: "error", message: errorMessage(error) }));

    return () => {
      cancelled = true;
      stops.forEach((stop) => stop());
    };
  }, [meetingId, attempt]);

  useEffect(() => {
    api
      .systemAudioPermission()
      .then((permission) => setSystemAudioAllowed(permission === "authorized"))
      .catch(() => setSystemAudioAllowed(false));
  }, [recording]);

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
              {speakerLabel[segment.source]}{" "}
              <time className="segment-time">{formatOffset(segment.startMs)}</time>
            </span>
            <p>{segment.text}</p>
          </li>
        ))}
      </ol>
      {recording && !systemAudioAllowed && !segments.some((s) => s.source === "them") && (
        <p className="empty-hint transcript-hint">
          Not seeing the other side? macOS may be blocking system audio. Allow Earshot Notes in System Settings &gt;
          Privacy &amp; Security &gt; Screen &amp; System Audio Recording.
        </p>
      )}
      {!recording && segments.length > 0 && <WhisperSuggestion onOpenSettings={onOpenSettings} />}
      <div ref={endRef} />
    </>
  );
}

/** After a recording, suggests the more accurate Whisper model until it's chosen or dismissed (#25). */
function WhisperSuggestion({ onOpenSettings }: { onOpenSettings: () => void }) {
  const [shown, setShown] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .transcriptionModels()
      .then((models) => !cancelled && setShown(models.suggestWhisper))
      .catch(() => {}); // A suggestion isn't worth an error message.
    return () => {
      cancelled = true;
    };
  }, []);

  async function dismiss() {
    try {
      await api.dismissWhisperSuggestion();
      setShown(false);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  if (!shown) return null;
  return (
    <aside className="transcript-suggestion" aria-label="Transcription suggestion">
      <p>
        For a more accurate transcript, especially of the other people on a call, download Whisper large-v3-turbo (574
        MB) in Settings.
      </p>
      <div className="transcript-suggestion-actions">
        <button type="button" className="button" onClick={onOpenSettings}>
          Open Settings
        </button>
        <button type="button" className="button subtle" onClick={() => void dismiss()}>
          Don&rsquo;t show again
        </button>
      </div>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </aside>
  );
}
