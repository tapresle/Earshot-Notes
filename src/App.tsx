import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, errorMessage } from "./lib/api";
import { MeetingList } from "./features/meetings/MeetingList";
import { LoadMore } from "./features/meetings/LoadMore";
import { useMeetingList } from "./features/meetings/useMeetingList";
import { SearchField } from "./features/meetings/SearchField";
import { useDebounced } from "./features/meetings/useDebounced";
import { MeetingView } from "./features/meetings/MeetingView";
import { TranscriptPanel } from "./features/transcript/TranscriptPanel";
import { ExportSelectionBar, SaveStatusText, useFolderExport, useMeetingExport } from "./features/export/ExportBar";
import { DeleteMeetingDialog } from "./features/meetings/DeleteMeetingDialog";
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
  // A meeting just created with the "Start recording when I create a new meeting" setting on.
  const [recordOnOpen, setRecordOnOpen] = useState<number | null>(null);
  const [selection, setSelection] = useState<Selection>({ ids: [], anchor: null });
  const [view, setView] = useState<View>("meetings");
  // The meeting waiting for you to confirm its deletion.
  const [deleting, setDeleting] = useState<{ id: number; title: string } | null>(null);
  // Shared by the meeting list's menu and the selection bar.
  const folderExport = useFolderExport();
  const meetingExport = useMeetingExport();
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

  // What the window shows, for the menu bar's event listeners. They register
  // once and read this instead of re-registering on every change, which
  // would drop an event that arrives between two registrations.
  const meetings = list.kind === "ready" ? list.meetings : [];
  const shown = useRef({ view, selectedId, meetings });
  useLayoutEffect(() => {
    shown.current = { view, selectedId, meetings };
  });
  // Bumped on every openMeeting, so only the latest one opens its meeting.
  const openRequest = useRef(0);

  // Opens a meeting in the meetings view: selects it, lists it in the sidebar,
  // and switches away from Settings or the setup checklist. If you open
  // something else while the meeting loads, what you picked stays.
  const openMeeting = useCallback(
    (meetingId: number) => {
      const request = ++openRequest.current;
      const before = shown.current;
      api
        .getMeeting(meetingId)
        .then((meeting) => {
          const now = shown.current;
          if (request !== openRequest.current || now.view !== before.view || now.selectedId !== before.selectedId) {
            return;
          }
          // A search that hides the meeting is cleared, so the sidebar lists it.
          if (!now.meetings.some((m) => m.id === meeting.id)) {
            updateMeetings((all) => (all.some((m) => m.id === meeting.id) ? all : [meeting, ...all]));
            setSearch("");
          }
          setSelectedId(meeting.id);
          setSelection({ ids: [meeting.id], anchor: meeting.id });
          setView("meetings");
        })
        .catch((error) => console.error("couldn't open the meeting from the menu bar", meetingId, error));
    },
    [updateMeetings],
  );

  // Recordings can also start and stop from the menu bar. One started there
  // records a new meeting, which opens here so the window shows it. A
  // recording of the open meeting started from this window needs nothing more.
  useEffect(() => {
    const unlisten = api.onRecordingChanged(({ meetingId }) => {
      setRecordingMeetingId(meetingId);
      if (meetingId === null || meetingId === shown.current.selectedId) return;
      openMeeting(meetingId);
    });
    return () => {
      unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [openMeeting]);

  // A stop from the menu bar opens the meeting that stopped (#184), so the
  // transcript and summary you just finished are in front of you. The
  // window's own stop stays where it is, and a meeting already on screen and
  // in the sidebar needs nothing more, which keeps a multi-selection.
  useEffect(() => {
    const unlisten = api.onMeetingOpen(({ meetingId }) => {
      const now = shown.current;
      if (now.view === "meetings" && now.selectedId === meetingId && now.meetings.some((m) => m.id === meetingId)) {
        return;
      }
      openMeeting(meetingId);
    });
    return () => {
      unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [openMeeting]);

  async function createMeeting() {
    setCreating(true);
    setCreateError(null);
    try {
      const [meeting, record] = await Promise.all([
        api.createMeeting(),
        // Without the setting, the meeting still opens and you can start recording yourself.
        api.recordNewMeetings().catch((error) => {
          console.error("couldn't read whether to record new meetings", error);
          return false;
        }),
      ]);
      updateMeetings((all) => [meeting, ...all]);
      // Another recording keeps running; this meeting opens without starting a second one.
      setRecordOnOpen(record && recordingMeetingId === null ? meeting.id : null);
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

  function removeMeeting(id: number) {
    updateMeetings((all) => all.filter((m) => m.id !== id));
    setSelectedId((open) => (open === id ? null : open));
    setSelection((current) => ({
      ids: current.ids.filter((selected) => selected !== id),
      anchor: current.anchor === id ? null : current.anchor,
    }));
  }

  return (
    // Settings and setup have no transcript beside them, so they take its width too.
    <div className={view === "meetings" ? "app-shell" : "app-shell is-wide"}>
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
        onExport={(ids) => void (ids.length === 1 ? meetingExport.exportMeeting(ids[0]) : folderExport.exportTo(ids))}
        onDelete={(id, title) => setDeleting({ id, title })}
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
        listEnd={
          list.kind === "ready" && (
            <LoadMore
              hasMore={list.next !== null}
              loading={list.loadingMore}
              error={list.moreError}
              onLoadMore={loadMore}
            />
          )
        }
        footer={
          <>
            {meetingExport.status.kind !== "idle" && (
              <div className="export-bar">
                <SaveStatusText status={meetingExport.status} />
              </div>
            )}
            <ExportSelectionBar
              folderExport={folderExport}
              selectedIds={selection.ids}
              onClearSelection={() => setSelection({ ids: selectedId === null ? [] : [selectedId], anchor: selectedId })}
            />
            <button
              type="button"
              className="button subtle settings-button"
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
              autoStartRecording={recordOnOpen === selectedId}
              onAutoStartRecording={() => setRecordOnOpen(null)}
              onRenamed={renameInList}
              onDelete={(id, title) => setDeleting({ id, title })}
            />
          ))}
      </main>

      {deleting && (
        <DeleteMeetingDialog
          key={deleting.id}
          meetingId={deleting.id}
          // The list has the latest title, if you renamed the meeting since it opened.
          title={meetings.find((m) => m.id === deleting.id)?.title ?? deleting.title}
          onDeleted={(id) => {
            removeMeeting(id);
            setDeleting(null);
          }}
          onClose={() => setDeleting(null)}
        />
      )}

      {view === "meetings" && (
        <TranscriptPanel meetingId={selectedId} recording={selectedId !== null && selectedId === recordingMeetingId} />
      )}
    </div>
  );
}
