import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type MeetingDetail } from "../../lib/api";
import { ChatPanel } from "../chat/ChatPanel";
import { useChat } from "../chat/useChat";
import { Notepad, type NotepadHandle } from "../notepad/Notepad";
import { ExportMeetingButton } from "../export/ExportBar";
import { RecordingControls } from "../recording/RecordingControls";
import { SummarizeButton, SummaryPanel } from "../summary/SummaryPanel";
import { useSummary } from "../summary/useSummary";

interface MeetingViewProps {
  meetingId: number;
  recordingMeetingId: number | null;
  onRecordingChange: (meetingId: number | null) => void;
  /** Start recording as soon as the meeting opens (see `RecordingControls`). */
  autoStartRecording?: boolean;
  onAutoStartRecording?: () => void;
  onRenamed: (id: number, title: string) => void;
  /** Asks before deleting the meeting. */
  onDelete: (id: number, title: string) => void;
}

type LoadState = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready"; meeting: MeetingDetail };

export function MeetingView({
  meetingId,
  recordingMeetingId,
  onRecordingChange,
  autoStartRecording,
  onAutoStartRecording,
  onRenamed,
  onDelete,
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
      autoStartRecording={autoStartRecording}
      onAutoStartRecording={onAutoStartRecording}
      onRenamed={onRenamed}
      onDelete={onDelete}
    />
  );
}

type Tab = "notes" | "summary" | "chat";

function LoadedMeeting({
  meeting,
  recordingMeetingId,
  onRecordingChange,
  autoStartRecording,
  onAutoStartRecording,
  onRenamed,
  onDelete,
}: Omit<MeetingViewProps, "meetingId"> & { meeting: MeetingDetail }) {
  const [tab, setTab] = useState<Tab>("notes");
  const summary = useSummary(meeting.id, () => setTab("summary"));
  const chat = useChat(meeting.id);
  const notepad = useRef<NotepadHandle>(null);

  function showNotes(passage: string) {
    setTab("notes");
    // The notes panel is hidden until the tab change renders.
    requestAnimationFrame(() => notepad.current?.select(passage));
  }

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
        <button type="button" className="button subtle danger" onClick={() => onDelete(meeting.id, meeting.title)}>
          Delete
        </button>
      </header>
      <div role="tablist" aria-label="Meeting views" className="tabs">
        {tabButton("notes", "My notes")}
        {tabButton("summary", "Summary")}
        {tabButton("chat", "Chat")}
      </div>
      {/* The panels stay mounted so unsaved notes and a typed question survive switching tabs. */}
      <div role="tabpanel" id="panel-notes" aria-labelledby="tab-notes" className="tab-panel" hidden={tab !== "notes"}>
        <Notepad ref={notepad} meetingId={meeting.id} initialMarkdown={meeting.notesMarkdown} />
      </div>
      <div
        role="tabpanel"
        id="panel-summary"
        aria-labelledby="tab-summary"
        className="tab-panel"
        hidden={tab !== "summary"}
      >
        <SummaryPanel state={summary} active={tab === "summary"} />
      </div>
      <div role="tabpanel" id="panel-chat" aria-labelledby="tab-chat" className="tab-panel" hidden={tab !== "chat"}>
        <ChatPanel
          state={chat}
          model={summary.model}
          onModelReady={summary.setModel}
          active={tab === "chat"}
          onShowNotes={showNotes}
        />
      </div>
      <footer className="meeting-footer">
        <RecordingControls
          meetingId={meeting.id}
          recordingMeetingId={recordingMeetingId}
          onRecordingChange={onRecordingChange}
          autoStart={autoStartRecording}
          onAutoStart={onAutoStartRecording}
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
  // The text in the field and the stored title, kept together so a title from a
  // summary can tell whether you've started typing.
  const [field, setField] = useState({ value: meeting.title, saved: meeting.title });
  const [error, setError] = useState<string | null>(null);
  const setTitle = (value: string) => setField((current) => ({ ...current, value }));

  // A summary can name an untitled meeting. Anything you're typing stays, and
  // replaces that title when you commit it.
  useEffect(() => {
    const unlisten = api.onSummaryDone(({ meetingId, title }) => {
      if (meetingId !== meeting.id || !title) return;
      setField((current) => ({ value: current.value === current.saved ? title : current.value, saved: title }));
    });
    return () => {
      unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [meeting.id]);

  async function commit() {
    const { value: title, saved } = field;
    if (title === saved) return;
    try {
      const stored = await api.renameMeeting(meeting.id, title);
      setField({ value: stored, saved: stored });
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
        value={field.value}
        onChange={(event) => setTitle(event.target.value)}
        onBlur={() => void commit()}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") setTitle(field.saved);
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
