import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, type Meeting } from "./lib/api";
import { MeetingList } from "./features/meetings/MeetingList";
import { MeetingView } from "./features/meetings/MeetingView";
import "./styles.css";

type ListState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready"; meetings: Meeting[] };

export default function App() {
  const [list, setList] = useState<ListState>({ kind: "loading" });
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .listMeetings()
      .then((meetings) => setList({ kind: "ready", meetings }))
      .catch((error) => setList({ kind: "error", message: errorMessage(error) }));
  }, []);

  useEffect(load, [load]);

  async function createMeeting() {
    setCreating(true);
    setCreateError(null);
    try {
      const meeting = await api.createMeeting();
      setList((current) => (current.kind === "ready" ? { kind: "ready", meetings: [meeting, ...current.meetings] } : current));
      setSelectedId(meeting.id);
    } catch (error) {
      setCreateError(errorMessage(error));
    } finally {
      setCreating(false);
    }
  }

  function updateMeetings(update: (meetings: Meeting[]) => Meeting[]) {
    setList((current) => (current.kind === "ready" ? { kind: "ready", meetings: update(current.meetings) } : current));
  }

  const meetings = list.kind === "ready" ? list.meetings : [];

  return (
    <div className="app-shell">
      <MeetingList
        meetings={meetings}
        selectedId={selectedId}
        onSelect={setSelectedId}
        onCreate={() => void createMeeting()}
        creating={creating}
      />

      <main className="workspace">
        {createError && (
          <p className="inline-error" role="alert">
            Couldn't create the meeting. {createError}
          </p>
        )}
        {list.kind === "loading" && <p className="empty-hint">Loading meetings…</p>}
        {list.kind === "error" && (
          <div className="inline-error" role="alert">
            <p>{list.message}</p>
            <button
              type="button"
              className="button"
              onClick={() => {
                setList({ kind: "loading" });
                load();
              }}
            >
              Try again
            </button>
          </div>
        )}
        {list.kind === "ready" &&
          (selectedId === null ? (
            <div className="empty-state">
              <h1>{meetings.length === 0 ? "Start your first meeting" : "Pick a meeting"}</h1>
              <p>Take notes while the app transcribes on this device. Nothing leaves your computer.</p>
            </div>
          ) : (
            <MeetingView
              key={selectedId}
              meetingId={selectedId}
              onRenamed={(id, title) => updateMeetings((all) => all.map((m) => (m.id === id ? { ...m, title } : m)))}
              onDeleted={(id) => {
                updateMeetings((all) => all.filter((m) => m.id !== id));
                setSelectedId(null);
              }}
            />
          ))}
      </main>

      <aside className="transcript-panel" aria-label="Transcript">
        <h2 className="pane-heading">Transcript</h2>
        <p className="empty-hint">The live transcript appears here while you record.</p>
      </aside>
    </div>
  );
}
