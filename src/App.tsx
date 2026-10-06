import { useCallback, useEffect, useState } from "react";
import { api, errorMessage } from "./lib/api";
import { MeetingList } from "./features/meetings/MeetingList";
import { LoadMore } from "./features/meetings/LoadMore";
import { useMeetingList } from "./features/meetings/useMeetingList";
import { SearchField } from "./features/meetings/SearchField";
import { useDebounced } from "./features/meetings/useDebounced";
import { MeetingView } from "./features/meetings/MeetingView";
import { TranscriptPanel } from "./features/transcript/TranscriptPanel";
import { ExportBar } from "./features/export/ExportBar";
import { SettingsView } from "./features/settings/SettingsView";
import { SetupChecklist } from "./features/setup/SetupChecklist";
import { select, type Selection } from "./features/meetings/selection";
import "./styles.css";

type View = "meetings" | "settings" | "setup";

/** How long typing must pause before the sidebar searches. */
const SEARCH_DELAY_MS = 200;

export default function App() {
  const [search, setSearch] = useState("");
  const query = useDebounced(search.trim(), SEARCH_DELAY_MS);
  const { list, reload, loadMore, update: updateMeetings } = useMeetingList(query);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [recordingMeetingId, setRecordingMeetingId] = useState<number | null>(null);
  const [selection, setSelection] = useState<Selection>({ ids: [], anchor: null });
  const [view, setView] = useState<View>("meetings");
  const renameInList = useCallback(
    (id: number, title: string) => updateMeetings((all) => all.map((m) => (m.id === id ? { ...m, title } : m))),
    [updateMeetings],
  );

  useEffect(() => {
    api
      .setupComplete()
      .then((complete) => !complete && setView("setup"))
      .catch((error) => console.error("couldn't read the setup state", error));
  }, []);

  // A summary can name an untitled meeting, including one that isn't open.
  useEffect(() => {
    const unlisten = api.onSummaryDone(({ meetingId, title }) => {
      if (title) renameInList(meetingId, title);
    });
    return () => {
      unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [renameInList]);

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
      updateMeetings((all) => [meeting, ...all]);
      // A new meeting matches no search, so show the full list it belongs to.
      setSearch("");
      setSelectedId(meeting.id);
      setSelection({ ids: [meeting.id], anchor: meeting.id });
      setView("meetings");
    } catch (error) {
      setCreateError(errorMessage(error));
    } finally {
      setCreating(false);
    }
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
            setView("meetings");
          }
        }}
        onCreate={() => void createMeeting()}
        creating={creating}
        search={
          <SearchField
            value={search}
            onChange={(value) => {
              setSearch(value);
              // Keep only the open meeting selected, since the list is about to change.
              setSelection({ ids: selectedId === null ? [] : [selectedId], anchor: selectedId });
            }}
          />
        }
        emptyHint={query === "" ? undefined : `No meetings match “${query}”.`}
        footer={
          <>
            {list.kind === "ready" && (
              <LoadMore
                hasMore={list.next !== null}
                loading={list.loadingMore}
                error={list.moreError}
                onLoadMore={loadMore}
              />
            )}
            <ExportBar
              selectedIds={selection.ids}
              meetingCount={meetings.length}
              onClearSelection={() => setSelection({ ids: selectedId === null ? [] : [selectedId], anchor: selectedId })}
            />
            <button
              type="button"
              className={`button subtle settings-button ${meetings.length === 0 ? "pinned-bottom" : ""}`}
              aria-current={view === "settings" ? "page" : undefined}
              onClick={() => setView("settings")}
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
        {view === "settings" && <SettingsView recording={recordingMeetingId !== null} onShowSetup={() => setView("setup")} />}
        {view === "setup" && (
          <SetupChecklist onOpenSettings={() => setView("settings")} onFinish={() => setView("meetings")} />
        )}
        {view === "meetings" && list.kind === "loading" && <p className="empty-hint">Loading meetings…</p>}
        {view === "meetings" && list.kind === "error" && (
          <div className="inline-error" role="alert">
            <p>{list.message}</p>
            <button
              type="button"
              className="button"
              onClick={reload}
            >
              Try again
            </button>
          </div>
        )}
        {view === "meetings" &&
          list.kind === "ready" &&
          (selectedId === null ? (
            <div className="empty-state">
              <h1>{meetings.length === 0 && query === "" ? "Start your first meeting" : "Pick a meeting"}</h1>
              <p>Take notes while the app transcribes on this device. Nothing leaves your computer.</p>
            </div>
          ) : (
            <MeetingView
              key={selectedId}
              meetingId={selectedId}
              recordingMeetingId={recordingMeetingId}
              onRecordingChange={setRecordingMeetingId}
              onRenamed={renameInList}
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
