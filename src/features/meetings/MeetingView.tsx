import { useEffect, useState } from "react";
import { api, errorMessage, type MeetingDetail } from "../../lib/api";
import { Notepad } from "../notepad/Notepad";
import { ExportMeetingButton } from "../export/ExportBar";
import { RecordingControls } from "../recording/RecordingControls";
import { SummarizeButton, SummaryPanel } from "../summary/SummaryPanel";
import { useSummary } from "../summary/useSummary";

interface MeetingViewProps {
  meetingId: number;
  recordingMeetingId: number | null;
  onRecordingChange: (meetingId: number | null) => void;
  onRenamed: (id: number, title: string) => void;
  onDeleted: (id: number) => void;
}

type LoadState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready"; meeting: MeetingDetail };

export function MeetingView({
  meetingId,
  recordingMeetingId,
  onRecordingChange,
  onRenamed,
  onDeleted,
}: MeetingViewProps) {
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    api
      .getMeeting(meetingId)
      .then((meeting) => !cancelled && setState({ kind: "ready", meeting }))
      .catch((error) => !cancelled && setState({ kind: "error", message: errorMessage(error) }));
    return () => {
      cancelled = true;
    };
  }, [meetingId, attempt]);

  if (state.kind === "loading") {
    return <p className="empty-hint">Loading meeting…</p>;
  }
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
    <LoadedMeeting
      meeting={state.meeting}
      recordingMeetingId={recordingMeetingId}
      onRecordingChange={onRecordingChange}
      onRenamed={onRenamed}
      onDeleted={onDeleted}
    />
  );
}

type Tab = "notes" | "summary";

function LoadedMeeting({
  meeting,
  recordingMeetingId,
  onRecordingChange,
  onRenamed,
  onDeleted,
}: Omit<MeetingViewProps, "meetingId"> & { meeting: MeetingDetail }) {
  const [tab, setTab] = useState<Tab>("notes");
  const summary = useSummary(meeting.id);

  const tabButton = (id: Tab, label: string) => (
    <button
      type="button"
      role="tab"
      id={`tab-${id}`}
      aria-selected={tab === id}
      aria-controls={`panel-${id}`}
      className="tab"
      onClick={() => setTab(id)}
    >
      {label}
    </button>
  );

  return (
    <article className="meeting-view">
      <header className="meeting-header">
        <TitleField meeting={meeting} onRenamed={onRenamed} />
        <ExportMeetingButton meetingId={meeting.id} />
        <DeleteControl meetingId={meeting.id} onDeleted={onDeleted} />
      </header>
      <div role="tablist" aria-label="Meeting views" className="tabs">
        {tabButton("notes", "My notes")}
        {tabButton("summary", "Summary")}
      </div>
      {/* Both panels stay mounted so unsaved notes survive switching tabs. */}
      <div role="tabpanel" id="panel-notes" aria-labelledby="tab-notes" className="tab-panel" hidden={tab !== "notes"}>
        <Notepad meetingId={meeting.id} initialMarkdown={meeting.notesMarkdown} />
      </div>
      <div
        role="tabpanel"
        id="panel-summary"
        aria-labelledby="tab-summary"
        className="tab-panel"
        hidden={tab !== "summary"}
      >
        <SummaryPanel state={summary} />
      </div>
      <footer className="meeting-footer">
        <RecordingControls
          meetingId={meeting.id}
          recordingMeetingId={recordingMeetingId}
          onRecordingChange={onRecordingChange}
        />
        <SummarizeButton
          state={summary}
          recording={recordingMeetingId === meeting.id}
          onStarted={() => setTab("summary")}
        />
      </footer>
    </article>
  );
}

function TitleField({ meeting, onRenamed }: { meeting: MeetingDetail; onRenamed: MeetingViewProps["onRenamed"] }) {
  const [title, setTitle] = useState(meeting.title);
  const [savedTitle, setSavedTitle] = useState(meeting.title);
  const [error, setError] = useState<string | null>(null);

  async function commit() {
    if (title === savedTitle) return;
    try {
      const stored = await api.renameMeeting(meeting.id, title);
      setTitle(stored);
      setSavedTitle(stored);
      setError(null);
      onRenamed(meeting.id, stored);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div className="title-field">
      <label htmlFor="meeting-title" className="sr-only">
        Meeting title
      </label>
      <input
        id="meeting-title"
        className="title-input"
        value={title}
        onChange={(event) => setTitle(event.target.value)}
        onBlur={() => void commit()}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") setTitle(savedTitle);
        }}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? "meeting-title-error" : undefined}
      />
      {error && (
        <p id="meeting-title-error" className="field-error">
          {error}
        </p>
      )}
    </div>
  );
}

function DeleteControl({ meetingId, onDeleted }: { meetingId: number; onDeleted: MeetingViewProps["onDeleted"] }) {
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!confirming) {
    return (
      <button type="button" className="button subtle" onClick={() => setConfirming(true)}>
        Delete
      </button>
    );
  }

  async function confirmDelete() {
    setDeleting(true);
    try {
      await api.deleteMeeting(meetingId);
      onDeleted(meetingId);
    } catch (e) {
      setError(errorMessage(e));
      setDeleting(false);
    }
  }

  return (
    <div className="delete-confirm" role="group" aria-label="Confirm delete">
      <span>{error ?? "Delete this meeting and its notes? You can't undo this."}</span>
      <button type="button" className="button danger" onClick={() => void confirmDelete()} disabled={deleting}>
        {deleting ? "Deleting…" : "Delete meeting"}
      </button>
      <button type="button" className="button" onClick={() => setConfirming(false)} disabled={deleting}>
        Cancel
      </button>
    </div>
  );
}
