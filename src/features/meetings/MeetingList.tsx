import type { ReactNode } from "react";
import type { Meeting } from "../../lib/api";
import { groupByDay } from "./groupByDay";
import type { SelectMode } from "./selection";
import { useToday } from "./useToday";

interface MeetingListProps {
  meetings: Meeting[];
  /** The open meeting. */
  selectedId: number | null;
  /** Meetings selected for a bulk action, including the open one. */
  selectedIds: number[];
  recordingMeetingId: number | null;
  onSelect: (id: number, mode: SelectMode) => void;
  onCreate: () => void;
  creating: boolean;
  footer?: ReactNode;
  /** Shown under the New meeting button, such as a search field. */
  search?: ReactNode;
  /** Shown when there are no meetings to list. */
  emptyHint?: string;
}

const timeFormat: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

export function MeetingList({
  meetings,
  selectedId,
  selectedIds,
  recordingMeetingId,
  onSelect,
  onCreate,
  creating,
  footer,
  search,
  emptyHint = "Your meetings appear here.",
}: MeetingListProps) {
  const multiSelect = selectedIds.length > 1;
  const today = useToday();
  const groups = groupByDay(meetings, today);

  return (
    <nav className="sidebar" aria-label="Meetings">
      <button type="button" className="button primary full-width" onClick={onCreate} disabled={creating}>
        {creating ? "Creating…" : "New meeting"}
      </button>
      {search}

      {groups.length === 0 ? (
        <p className="empty-hint sidebar-hint">{emptyHint}</p>
      ) : (
        groups.map((group) => (
          <section key={group.label} className="day-group" aria-label={group.label}>
            <h2 className="pane-heading">{group.label}</h2>
            <ul className="meeting-list">
              {group.meetings.map((meeting) => {
                const meta =
                  meeting.id === recordingMeetingId
                    ? "Recording"
                    : new Date(meeting.createdAt).toLocaleTimeString(undefined, timeFormat);
                const inSelection = multiSelect && selectedIds.includes(meeting.id);
                return (
                <li key={meeting.id}>
                  <button
                    type="button"
                    className={`meeting-item ${inSelection ? "is-selected" : ""}`}
                    aria-label={`${meeting.title}, ${meta}${inSelection ? ", selected" : ""}`}
                    aria-current={meeting.id === selectedId ? "page" : undefined}
                    onClick={(event) =>
                      onSelect(
                        meeting.id,
                        event.shiftKey ? "range" : event.metaKey || event.ctrlKey ? "toggle" : "single",
                      )
                    }
                  >
                    <span className="meeting-title">{meeting.title}</span>
                    <span className="meeting-meta">{meta}</span>
                  </button>
                </li>
                );
              })}
            </ul>
          </section>
        ))
      )}
      {footer}
    </nav>
  );
}
