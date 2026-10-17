import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type Source, type TranscriptPreview, type TranscriptSegment } from "../../lib/api";
import { formatOffset } from "./formatOffset";
import { segmentElementId } from "./showSegment";

/** Speaker labels, matching the labels in exports and summary prompts. */
const speakerLabel: Record<TranscriptSegment["source"], string> = {
  me: "Me",
  them: "Meeting Participants",
};

interface TranscriptPanelProps {
  meetingId: number | null;
  recording: boolean;
}

type LoadState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready" };

type Previews = Partial<Record<Source, TranscriptPreview>>;

/** Sets or, for empty text, clears a source's preview. */
function withPreview(current: Previews, preview: TranscriptPreview): Previews {
  const next = { ...current };
  if (preview.text) next[preview.source] = preview;
  else delete next[preview.source];
  return next;
}

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
  const [previews, setPreviews] = useState<Previews>({});
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
      api.onTranscriptPreview((preview) => {
        if (!cancelled && preview.meetingId === meetingId) setPreviews((p) => withPreview(p, preview));
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

  // A source's preview stays until the worker clears it, after the segment
  // that replaces it, including the last ones after recording stops.
  const tentative = Object.values(previews).sort((a, b) => a.startMs - b.startMs);
  const tentativeText = tentative.map((p) => p.text).join("\n");
  const heardThem = segments.some((s) => s.source === "them") || previews.them !== undefined;

  useEffect(() => {
    if (recording) endRef.current?.scrollIntoView?.({ block: "end" });
  }, [segments.length, tentativeText, recording]);

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
      {segments.length === 0 && tentative.length === 0 && (
        <p className="empty-hint">
          {recording
            ? "Listening. Speech appears here as it's spoken."
            : "No transcript yet. Start recording to transcribe this meeting."}
        </p>
      )}
      <ol className="transcript" aria-live="polite">
        {segments.map((segment) => (
          // Chat citations find, scroll to, and focus segments by these attributes.
          <li
            key={segment.id}
            id={segmentElementId(segment.id)}
            data-start-ms={segment.startMs}
            tabIndex={-1}
            className="transcript-segment"
          >
            <span className={`speaker speaker-${segment.source}`}>
              {speakerLabel[segment.source]}{" "}
              <time className="segment-time">{formatOffset(segment.startMs)}</time>
            </span>
            <p>{segment.text}</p>
          </li>
        ))}
      </ol>
      {/* Outside the live region: this text changes every second until its segment is final. */}
      {tentative.length > 0 && (
        <ol className="transcript">
          {tentative.map((preview) => (
            <li key={preview.source} className="transcript-segment is-tentative">
              <span className={`speaker speaker-${preview.source}`}>
                {speakerLabel[preview.source]} <time className="segment-time">{formatOffset(preview.startMs)}</time>
              </span>
              <p>
                {preview.text}
                <span className="sr-only"> (still listening)</span>
              </p>
            </li>
          ))}
        </ol>
      )}
      {recording && !systemAudioAllowed && !heardThem && (
        <p className="empty-hint transcript-hint">
          Not seeing the other side? macOS may be blocking system audio. Allow Earshot Notes in System Settings &gt;
          Privacy &amp; Security &gt; Screen &amp; System Audio Recording.
        </p>
      )}
      <div ref={endRef} />
    </>
  );
}
