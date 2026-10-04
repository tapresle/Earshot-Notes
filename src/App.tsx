import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, type Meeting } from "./lib/api";
import { MeetingList } from "./features/meetings/MeetingList";
import { MeetingView } from "./features/meetings/MeetingView";
import { TranscriptPanel } from "./features/transcript/TranscriptPanel";
import { ExportBar } from "./features/export/ExportBar";
import { SettingsView } from "./features/settings/SettingsView";
import { select, type Selection } from "./features/meetings/selection";
import "./styles.css";

type ListState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready"; meetings: Meeting[] };

export default function App() {
  const [list, setList] = useState<ListState>({ kind: "loading" });
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [recordingMeetingId, setRecordingMeetingId] = useState<number | null>(null);
  const [selection, setSelection] = useState<Selection>({ ids: [], anchor: null });
  const [showSettings, setShowSettings] = useState(false);

  const load = useCallback(() => {
    api
      .listMeetings()
      .then((meetings) => setList({ kind: "ready", meetings }))
      .catch((error) => setList({ kind: "error", message: errorMessage(error) }));
  }, []);

  useEffect(load, [load]);

  useEffect(() => {
    api
      .activeRecording()
      .then((id) => setRecordingMeetingId(id ?? null))
      .catch(() => setRecordingMeetingId(null));
  }, []);

  async function createMeeting() {
    setCreating(true);
    setCreateError(null);
    try {
      const meeting = await api.createMeeting();
      setList((current) => (current.kind === "ready" ? { kind: "ready", meetings: [meeting, ...current.meetings] } : current));
      setSelectedId(meeting.id);
      setSelection({ ids: [meeting.id], anchor: meeting.id });
      setShowSettings(false);
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
        selectedIds={selection.ids}
        recordingMeetingId={recordingMeetingId}
        onSelect={(id, mode) => {
          // Start a multi-selection from the open meeting, like Finder.
          const current = selection.ids.length > 0 || selectedId === null ? selection : { ids: [selectedId], anchor: selectedId };
          setSelection(select(meetings.map((m) => m.id), current, id, mode));
          if (mode === "single") {
            setSelectedId(id);
            setShowSettings(false);
          }
        }}
        onCreate={() => void createMeeting()}
        creating={creating}
        footer={
          <>
            <ExportBar
              selectedIds={selection.ids}
              meetingCount={meetings.length}
              onClearSelection={() => setSelection({ ids: selectedId === null ? [] : [selectedId], anchor: selectedId })}
            />
            <button
              type="button"
              className={`button subtle settings-button ${meetings.length === 0 ? "pinned-bottom" : ""}`}
              aria-current={showSettings ? "page" : undefined}
              onClick={() => setShowSettings(true)}
            >
              Settings
            </button>
          </>
        }
      />

      <main className="workspace">
        {createError && (
          <p className="inline-error" role="alert">
            Couldn't create the meeting. {createError}
          </p>
        )}
        {showSettings && <SettingsView />}
        {!showSettings && list.kind === "loading" && <p className="empty-hint">Loading meetings…</p>}
        {!showSettings && list.kind === "error" && (
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
        {!showSettings &&
          list.kind === "ready" &&
          (selectedId === null ? (
            <div className="empty-state">
              <h1>{meetings.length === 0 ? "Start your first meeting" : "Pick a meeting"}</h1>
              <p>Take notes while the app transcribes on this device. Nothing leaves your computer.</p>
            </div>
          ) : (
            <MeetingView
              key={selectedId}
              meetingId={selectedId}
              recordingMeetingId={recordingMeetingId}
              onRecordingChange={setRecordingMeetingId}
              onRenamed={(id, title) => updateMeetings((all) => all.map((m) => (m.id === id ? { ...m, title } : m)))}
              onDeleted={(id) => {
                updateMeetings((all) => all.filter((m) => m.id !== id));
                setSelectedId(null);
                setSelection((current) => ({ ids: current.ids.filter((selected) => selected !== id), anchor: null }));
              }}
            />
          ))}
      </main>

      <TranscriptPanel meetingId={selectedId} recording={selectedId !== null && selectedId === recordingMeetingId} />
    </div>
  );
}
